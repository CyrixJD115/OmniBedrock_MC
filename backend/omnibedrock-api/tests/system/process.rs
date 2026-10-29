//! ServerManager process tests: start/command/restart/stop round trips,
//! serialized concurrent starts, launch-failure recovery, the crash-restart
//! budget, and stop escalation that kills launcher descendants.
use crate::harness::{manager, setup, wait_line, wait_status};
use omnibedrock_server::server::{ServerSettings, Status};
use std::{path::PathBuf, time::Duration};
use tempfile::TempDir;

#[tokio::test]
async fn process_round_trip_restart_and_no_stale_commands() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    assert!(m.send_command("before-start").await.is_err());
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let first_pid = m.snapshot().pid;
    m.send_command("héllo").await.unwrap();
    wait_line(&m, "command=héllo").await;
    assert!(m.send_command("op Steve\nstop").await.is_err());
    assert!(m.send_command("").await.is_err());
    m.action("restart").await.unwrap();
    assert_ne!(m.snapshot().pid, first_pid);
    assert_eq!(m.snapshot().status, Status::Running);
    m.action("stop").await.unwrap();
    assert_eq!(m.snapshot().status, Status::Stopped);
    assert!(m.snapshot().pid.is_none());
    let lock = tokio::fs::read_to_string(m_settings_path(&_dir, "console_lock_state.yaml"))
        .await
        .unwrap();
    assert!(lock.contains("unlocked"));
    m.shutdown().await.unwrap();
}
fn m_settings_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join("data").join(name)
}
#[tokio::test]
async fn concurrent_start_is_serialized() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    let (a, b) = tokio::join!(m.action("start"), m.action("start"));
    assert!(a.is_ok() && b.is_ok());
    assert!(a.unwrap().contains("PID"));
    assert!(b.unwrap().contains("already"));
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_launch_recovers_to_stopped() {
    let (_dir, mut c) = setup().await;
    c.endstone_bin = Some("/nonexistent/omnibedrock-endstone".into());
    let m = manager(c).await;
    assert!(m.action("start").await.is_err());
    assert_eq!(m.snapshot().status, Status::Stopped);
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn crash_restart_budget_and_cancel() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.configure(ServerSettings {
        auto_restart_delay: 1,
        max_crashes: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    let first = m.snapshot().pid;
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    wait_status(&m, Status::Running).await;
    assert_ne!(first, m.snapshot().pid);
    assert_eq!(m.snapshot().crash_count, 1);
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(m.snapshot().status, Status::Crashed);
    assert_eq!(m.snapshot().crash_count, 2);
    m.action("start").await.unwrap();
    assert_eq!(m.snapshot().crash_count, 0);
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    m.action("stop").await.unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(m.snapshot().status, Status::Stopped);
    m.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_escalates_and_kills_launcher_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, c) = setup().await;
    let script = r#"#!/bin/sh
trap '' TERM
sleep 600 &
printf '%s' "$!" > "$3/descendant.pid"
printf '[INFO] descendant ready\n'
while IFS= read -r command; do printf '[INFO] ignored %s\n' "$command"; done
"#;
    tokio::fs::write(c.endstone_bin.as_ref().unwrap(), script)
        .await
        .unwrap();
    tokio::fs::set_permissions(
        c.endstone_bin.as_ref().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .await
    .unwrap();
    let path = c.bedrock_server_dir.join("descendant.pid");
    let m = manager(c).await;
    m.configure(ServerSettings {
        stop_timeout: 1,
        kill_timeout: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    wait_line(&m, "descendant ready").await;
    let pid: i32 = tokio::fs::read_to_string(path)
        .await
        .unwrap()
        .parse()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), m.action("stop"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m.snapshot().status, Status::Stopped);
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    assert!(stat.is_err() || stat.unwrap().split_whitespace().nth(2) == Some("Z"));
    m.shutdown().await.unwrap();
}
