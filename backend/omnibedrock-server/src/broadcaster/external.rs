//! /proc-based detection of foreign broadcaster processes working in this
//! instance's broadcaster directory — started by hand or orphaned by a
//! previous panel run — plus the SIGTERM/SIGKILL takeover that replaces them
//! before the panel spawns its own broadcaster.

use anyhow::bail;
use std::path::Path;
use std::time::{Duration, Instant};

use super::consts::{JAR_NAME, TAKEOVER_TIMEOUT};
use super::types::ExternalBroadcaster;

/// A process is "our broadcaster" when it runs the shared jar with its
/// working directory in this instance's broadcaster dir. Re-checked before
/// every signal so a recycled PID is never killed blindly.
#[cfg(target_os = "linux")]
async fn matches_broadcaster(pid: u32, dir: &Path) -> bool {
    let Ok(cmd) = tokio::fs::read(format!("/proc/{pid}/cmdline")).await else {
        return false;
    };
    if !String::from_utf8_lossy(&cmd).contains(JAR_NAME) {
        return false;
    }
    match (
        tokio::fs::read_link(format!("/proc/{pid}/cwd")).await,
        tokio::fs::canonicalize(dir).await,
    ) {
        (Ok(cwd), Ok(dir)) => cwd == dir,
        _ => false,
    }
}

/// Find a broadcaster java process working in `dir` that the panel does not
/// own right now: started by hand, or orphaned by a previous panel run
/// (recognized through the pidfile). This is what keeps two javas from
/// fighting over one Xbox session — the exact duplicate that answered a
/// friend's join offer with a stale relay target.
#[cfg(target_os = "linux")]
pub async fn find_external(dir: &Path, exclude: Option<u32>) -> Option<ExternalBroadcaster> {
    let mut entries = tokio::fs::read_dir("/proc").await.ok()?;
    while let Some(entry) = entries.next_entry().await.ok()? {
        let pid: u32 = match entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        {
            Some(pid) => pid,
            None => continue,
        };
        if Some(pid) == exclude {
            continue;
        }
        if !matches_broadcaster(pid, dir).await {
            continue;
        }
        let orphaned = match tokio::fs::read_to_string(dir.join("panel-broadcaster.pid")).await {
            Ok(text) => text.trim().parse::<u32>().ok() == Some(pid),
            Err(_) => false,
        };
        return Some(ExternalBroadcaster { pid, orphaned });
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub async fn find_external(_dir: &Path, _exclude: Option<u32>) -> Option<ExternalBroadcaster> {
    None
}

/// Stop a foreign broadcaster: SIGTERM, wait for exit, SIGKILL if needed.
/// Fails loudly when the process outlives both signals so the caller never
/// silently doubles up broadcasters.
#[cfg(target_os = "linux")]
pub(super) async fn kill_external(ext: &ExternalBroadcaster, dir: &Path) -> anyhow::Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    let pid = Pid::from_raw(ext.pid as i32);
    if !matches_broadcaster(ext.pid, dir).await {
        return Ok(()); // gone or recycled between scan and signal
    }
    if let Err(e) = kill(pid, Signal::SIGTERM) {
        tracing::warn!(%e, pid = ext.pid, "external broadcaster SIGTERM failed");
    }
    let deadline = Instant::now() + TAKEOVER_TIMEOUT;
    while Instant::now() < deadline {
        if !matches_broadcaster(ext.pid, dir).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if matches_broadcaster(ext.pid, dir).await {
        let _ = kill(pid, Signal::SIGKILL);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !matches_broadcaster(ext.pid, dir).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    if matches_broadcaster(ext.pid, dir).await {
        bail!(
            "external broadcaster (PID {}) ignored both SIGTERM and SIGKILL",
            ext.pid
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) async fn kill_external(_ext: &ExternalBroadcaster, _dir: &Path) -> anyhow::Result<()> {
    bail!("external broadcaster detection is not supported on this platform")
}
