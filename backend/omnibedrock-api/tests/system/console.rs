//! Console pipeline tests: high-volume reliable replay, oversized and
//! EOF-truncated lines, disk retention gaps, level/TPS/error statistics, and
//! the console WebSocket history, replay, command, and shutdown path.
use crate::harness::{manager, setup, state, wait_line, wait_status};
use futures_util::{SinkExt, StreamExt};
use omnibedrock_server::server::{Console, ServerSettings, Status, detect_level};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test]
async fn high_volume_replays_without_missing_lines() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let mut after = Some(m.console.replay(None, 0).latest);
    m.send_command("burst").await.unwrap();
    wait_line(&m, "burst complete").await;
    let mut numbers = Vec::new();
    loop {
        let replay = m.console.replay_reliable(after, 256).await.unwrap();
        assert!(replay.gap.is_none());
        if replay.lines.is_empty() {
            break;
        }
        for line in replay.lines {
            after = Some(line.sequence);
            if let Some(n) = line
                .line
                .strip_prefix("[INFO] burst ")
                .and_then(|n| n.parse::<u32>().ok())
            {
                numbers.push(n);
            }
        }
        if after == Some(replay.latest) {
            break;
        }
    }
    assert_eq!(numbers, (0..10000).collect::<Vec<_>>());
    assert!(m.console.replay(None, 10000).lines.len() <= 128);
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn oversized_and_eof_lines_are_preserved() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.configure(ServerSettings {
        auto_restart: false,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let after = Some(m.console.replay(None, 0).latest);
    m.send_command("long").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replay = m.console.replay_reliable(after, 128).await.unwrap();
            let size: usize = replay
                .lines
                .iter()
                .filter(|l| l.line.starts_with('x'))
                .map(|l| l.line.len())
                .sum();
            if size == 10000 {
                assert!(replay.lines.iter().any(|l| l.continued));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    m.send_command("partial").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    wait_line(&m, "unterminated").await;
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn disk_retention_reports_explicit_gap() {
    let (dir, mut c) = setup().await;
    c.console_journal_bytes = 4 * 1024 * 1024;
    let console = Console::new(&c);
    for i in 0..6000 {
        console
            .publish(format!("{i} {}", "x".repeat(1000)), "stdout", false)
            .await;
    }
    let replay = console.replay_reliable(Some(0), 256).await.unwrap();
    assert!(replay.gap.is_some());
    let bytes: u64 = walkdir::WalkDir::new(dir.path().join("logs/console"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().unwrap().len())
        .sum();
    assert!(bytes <= c.console_journal_bytes);
}
#[tokio::test]
async fn level_error_stats_and_mspt() {
    let (_dir, c) = setup().await;
    let console = Console::new(&c);
    assert_eq!(detect_level("[WARNING] hi"), "warn");
    assert_eq!(detect_level("Fatal exception"), "error");
    assert_eq!(detect_level("[INFO] error wording"), "info");
    console.publish("TPS: 19.8".into(), "stdout", false).await;
    assert_eq!(console.tps(), Some(19.8));
    console.publish("MSPT: 100".into(), "stdout", false).await;
    assert_eq!(console.tps(), Some(10.0));
    console
        .publish("[ERROR] failure 12 at 1.2.3".into(), "stderr", false)
        .await;
    console
        .publish("[ERROR] failure 99 at 2.3.4".into(), "stderr", false)
        .await;
    assert_eq!(console.errors().len(), 1);
    assert_eq!(console.errors()[0].count, 2);
}

#[tokio::test]
async fn websocket_history_replay_command_permissions_and_shutdown() {
    let (_dir, s, app, token) = state().await;
    s.server.action("start").await.unwrap();
    wait_line(&s.server, "ready").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = s.shutdown.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .unwrap();
    });
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/v1/console/ws?token={token}"))
            .await
            .unwrap();
    let mut sequence = 0;
    let mut stream = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = ws.next().await {
            if let Ok(text) = message.to_text() {
                let v: Value = serde_json::from_str(text).unwrap();
                if v["type"] == "console" {
                    sequence = v["sequence"].as_u64().unwrap();
                    stream = v["stream_id"].as_str().unwrap().to_owned();
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(sequence > 0);
    ws.close(None).await.unwrap();
    s.server.send_command("offline-command").await.unwrap();
    wait_line(&s.server, "offline-command").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/api/v1/console/ws?token={token}&after={sequence}&stream_id={stream}"
    ))
    .await
    .unwrap();
    let mut found = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = ws.next().await {
            if let Ok(text) = message.to_text() {
                let v: Value = serde_json::from_str(text).unwrap();
                if v["type"] == "console" {
                    assert!(v["sequence"].as_u64().unwrap() > sequence);
                    if v["line"].as_str().unwrap().contains("offline-command") {
                        found = true;
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(found);
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"command":"through-websocket"}).to_string().into(),
    ))
    .await
    .unwrap();
    wait_line(&s.server, "through-websocket").await;
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap();
}
