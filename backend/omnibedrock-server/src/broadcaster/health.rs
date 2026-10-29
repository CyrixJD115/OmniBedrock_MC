//! Panel-side RakNet health checking: resolve the relay target out of
//! config.yml, ping it with the same unconnected-ping packet the jar's query
//! uses while the broadcaster runs, record the verdict into the PingHealth
//! watch, and mirror the jar's own "failed to ping server" log lines into
//! that watch immediately instead of waiting for the next probe.

use anyhow::Context;
use omnibedrock_core::config::Config;
use regex::Regex;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::config::analyze_relay_config;
use super::consts::{PING_INTERVAL, PING_TIMEOUT, RAKNET_MAGIC};
use super::helpers::instance_port;
use super::types::{BcSnapshot, BcStatus, PingHealth, RelayAnalysis};
use crate::server::Console;

/// The address the jar's query ping goes to: the relay target, falling back
/// through the legacy session-info keys exactly like the jar's migration.
fn ping_target(analysis: &RelayAnalysis, fallback_port: u16) -> String {
    if let Some(target) = &analysis.relay {
        return target.text.clone();
    }
    if let Some(legacy) = &analysis.legacy {
        return legacy.text.clone();
    }
    format!("127.0.0.1:{fallback_port}")
}

fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// One RakNet unconnected ping, the same packet the broadcaster's query uses.
/// Any datagram that comes back with the pong header counts as healthy — the
/// metadata parsing is the jar's job, reachability is ours.
async fn raknet_ping(target: SocketAddr) -> anyhow::Result<()> {
    let socket = UdpSocket::bind(if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await?;
    let mut packet = Vec::with_capacity(25);
    packet.push(0x01); // Unconnected Ping
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    packet.extend_from_slice(&now.to_be_bytes());
    packet.extend_from_slice(&RAKNET_MAGIC);
    socket.send_to(&packet, target).await?;
    let mut buf = [0u8; 2048];
    let (n, _) = tokio::time::timeout(PING_TIMEOUT, socket.recv_from(&mut buf))
        .await
        .context("ping timed out")??;
    anyhow::ensure!(n > 0 && buf[0] == 0x1c, "reply is not a RakNet pong");
    Ok(())
}

/// Resolve the configured relay target to a socket address, resolving
/// hostnames like the jar would.
async fn probe_target(text: &str) -> anyhow::Result<()> {
    match text.parse::<SocketAddr>() {
        Ok(addr) => raknet_ping(addr).await,
        Err(_) => {
            let mut addrs = tokio::net::lookup_host(text)
                .await
                .context("target did not resolve")?;
            let addr = addrs
                .next()
                .context("target did not resolve to an address")?;
            raknet_ping(addr).await
        }
    }
}

/// Probe the relay target while the broadcaster runs so a misconfigured
/// target (the "failed to ping server" trap) shows up as structured health
/// instead of a buried log line. The jar's own ping cadence is 30s; this
/// matches it.
pub(super) async fn ping_poller(
    config: Arc<Config>,
    mut status: watch::Receiver<BcSnapshot>,
    health: Arc<watch::Sender<PingHealth>>,
    shutdown: CancellationToken,
) {
    loop {
        // Park until the broadcaster runs (or we are shutting down).
        loop {
            if shutdown.is_cancelled() {
                return;
            }
            if status.borrow().status == BcStatus::Running {
                break;
            }
            if status.changed().await.is_err() {
                return;
            }
        }
        // While running: re-read the config each round so edits apply
        // without a panel restart, probe, and record the verdict.
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(PING_INTERVAL) => {}
            }
            if status.borrow().status != BcStatus::Running {
                health.send_if_modified(|h| {
                    let cleared = *h != PingHealth::default();
                    *h = PingHealth::default();
                    cleared
                });
                break;
            }
            let port = instance_port(&config);
            let analysis =
                match tokio::fs::read_to_string(config.broadcaster_dir.join("config.yml")).await {
                    Ok(text) => analyze_relay_config(&text, port),
                    Err(_) => RelayAnalysis::unknown(),
                };
            if !analysis.query_server {
                // Pings are not part of this setup; nothing to degrade.
                health.send_if_modified(|h| {
                    let changed = h.target.is_some() || h.ok.is_some();
                    *h = PingHealth::default();
                    changed
                });
                continue;
            }
            let target = ping_target(&analysis, port);
            let ok = probe_target(&target).await.is_ok();
            let now = now_f64();
            health.send_if_modified(|h| {
                let changed = h.target.as_deref() != Some(target.as_str()) || h.ok != Some(ok);
                h.target = Some(target.clone());
                h.ok = Some(ok);
                if ok {
                    h.failing_since = None;
                } else if h.failing_since.is_none() {
                    h.failing_since = Some(now);
                }
                h.last_checked = Some(now);
                changed
            });
            if !ok {
                tracing::debug!(target = %target, "broadcaster relay-target ping failed");
            }
        }
    }
}

/// Mirror the jar's own ping failures into the health watch immediately,
/// instead of waiting up to a probe interval for confirmation.
pub(super) async fn watch_ping_failures(console: Console, health: Arc<watch::Sender<PingHealth>>) {
    static PING_FAIL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)failed to ping server").unwrap());
    let mut changed = console.changed.subscribe();
    let mut cursor = console.replay(None, 0).latest;
    loop {
        if changed.changed().await.is_err() {
            break;
        }
        changed.borrow_and_update();
        let batch = console.replay(Some(cursor), 1024);
        for line in &batch.lines {
            cursor = line.sequence;
            if PING_FAIL.is_match(&line.line) {
                health.send_if_modified(|h| {
                    if h.ok == Some(false) {
                        false
                    } else {
                        h.ok = Some(false);
                        if h.failing_since.is_none() {
                            h.failing_since = Some(line.timestamp);
                        }
                        true
                    }
                });
            }
        }
    }
}
