//! Panel-side health checking of the broadcaster's query target.
//!
//! Since BDS 1.26.60 a server runs one of two transports, and they answer
//! on different sockets:
//!
//! - `transport=nethernet` (the modern default): HTTP signaling on TCP
//!   `server-port` — `GET /v1/join` returns `{"name","version","players",…}`.
//!   This is exactly what MCXboxBroadcast (build 160+) probes. Such a server
//!   does NOT answer RakNet pings on UDP at all.
//! - `transport=raknet`: the classic unconnected ping/pong on UDP.
//!
//! A RakNet-only probe therefore reported "cannot reach the server" for every
//! healthy NetherNet server while the jar happily updated its session — a
//! pure false positive. The probe now tries the configured transport first
//! and the other second, and only a server that answers NEITHER is
//! unreachable.
//!
//! False alarms are further suppressed by: pausing while the instance's game
//! server is not running (nothing to reach), a boot grace period, and
//! requiring two consecutive failed probes before raising the flag. The jar's
//! own "failed to ping server" log line is still mirrored immediately.

use anyhow::{Context, bail};
use omnibedrock_core::config::Config;
use regex::Regex;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::config::analyze_relay_config;
use super::consts::{PING_INTERVAL, PING_TIMEOUT, RAKNET_MAGIC};
use super::helpers::server_facts;
use super::types::{BcSnapshot, BcStatus, PingHealth, PongInfo, RelayAnalysis};
use crate::server::{Console, Snapshot, Status};

/// Failed probes in a row before the health flips to unhealthy.
const FAILURES_BEFORE_ALERT: u32 = 2;
/// Freshly started servers load packs/worlds before they answer; probing
/// inside this window would only produce boot-time false alarms.
const BOOT_GRACE: Duration = Duration::from_secs(30);
/// First probe after the broadcaster (or the game server) comes up.
const FIRST_PROBE_DELAY: Duration = Duration::from_secs(8);

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

/// Parse a RakNet unconnected pong's MOTD
/// (`MCPE;name;protocol;version;players;max;guid;level;gamemode;…`).
fn parse_raknet_pong(packet: &[u8]) -> PongInfo {
    // 0x1c, time(8), server guid(8), magic(16), strlen(2), motd
    let motd = packet
        .get(35..)
        .map(|m| String::from_utf8_lossy(m).into_owned())
        .unwrap_or_default();
    let fields: Vec<&str> = motd.split(';').collect();
    let at = |i: usize| fields.get(i).map(|s| s.to_string()).unwrap_or_default();
    PongInfo {
        name: at(1),
        version: at(3),
        level: at(7),
        players: fields.get(4).and_then(|s| s.parse().ok()),
        max_players: fields.get(5).and_then(|s| s.parse().ok()),
    }
}

/// One RakNet unconnected ping, the same packet older broadcaster builds use.
async fn raknet_ping(target: SocketAddr) -> anyhow::Result<PongInfo> {
    let socket = UdpSocket::bind(if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await?;
    let mut packet = Vec::with_capacity(33);
    packet.push(0x01); // Unconnected Ping
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    packet.extend_from_slice(&now.to_be_bytes());
    packet.extend_from_slice(&RAKNET_MAGIC);
    packet.extend_from_slice(&[0u8; 8]); // client GUID
    socket.send_to(&packet, target).await?;
    let mut buf = [0u8; 2048];
    let (n, _) = tokio::time::timeout(PING_TIMEOUT, socket.recv_from(&mut buf))
        .await
        .context("no RakNet pong (UDP)")??;
    anyhow::ensure!(n > 0 && buf[0] == 0x1c, "reply is not a RakNet pong");
    Ok(parse_raknet_pong(&buf[..n]))
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // The target is this host's own game server: never route it through
        // an HTTP(S)_PROXY from the environment.
        .no_proxy()
        .connect_timeout(PING_TIMEOUT)
        .timeout(PING_TIMEOUT * 2)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static HTTP client")
});

/// The NetherNet HTTP signaling probe MCXboxBroadcast itself uses:
/// `GET http://<target>/v1/join` answered with the server's status JSON.
async fn nethernet_ping(target: SocketAddr) -> anyhow::Result<PongInfo> {
    let response = HTTP
        .get(format!("http://{target}/v1/join"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                anyhow::anyhow!("NetherNet signaling refused (TCP)")
            } else if e.is_timeout() {
                anyhow::anyhow!("NetherNet signaling timed out (TCP)")
            } else {
                anyhow::anyhow!("NetherNet signaling failed: {e}")
            }
        })?;
    if !response.status().is_success() {
        bail!(
            "NetherNet signaling answered HTTP {}",
            response.status().as_u16()
        );
    }
    let bytes = response
        .bytes()
        .await
        .context("NetherNet signaling body unreadable")?;
    anyhow::ensure!(
        bytes.len() <= 64 * 1024,
        "NetherNet status is implausibly large"
    );
    let body: serde_json::Value = serde_json::from_slice(&bytes)
        .context("NetherNet signaling did not answer with status JSON")?;
    if body.get("name").is_none() && body.get("version").is_none() {
        bail!("answer is not a NetherNet status");
    }
    let text = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
    let number = |k: &str| body[k].as_u64().and_then(|n| u32::try_from(n).ok());
    Ok(PongInfo {
        name: text("name"),
        version: text("version"),
        level: text("level"),
        players: number("players"),
        max_players: number("maxPlayers"),
    })
}

/// Probe outcome: which protocol answered and what it said, or why both failed.
pub(super) struct ProbeResult {
    pub(super) outcome: Result<(&'static str, PongInfo), String>,
    pub(super) latency_ms: u64,
}

/// Resolve the target like the jar would (hostnames included), then try the
/// configured transport first and the other one second.
pub(super) async fn probe(text: &str, transport: &str) -> ProbeResult {
    let started = Instant::now();
    let addr = match text.parse::<SocketAddr>() {
        Ok(addr) => Ok(addr),
        Err(_) => match tokio::net::lookup_host(text).await {
            Ok(mut addrs) => addrs
                .next()
                .ok_or_else(|| format!("{text} did not resolve to an address")),
            Err(e) => Err(format!("{text} did not resolve: {e}")),
        },
    };
    let addr = match addr {
        Ok(addr) => addr,
        Err(e) => {
            return ProbeResult {
                outcome: Err(e),
                latency_ms: 0,
            };
        }
    };
    let raknet_first = transport.eq_ignore_ascii_case("raknet");
    let order: [&'static str; 2] = if raknet_first {
        ["raknet", "nethernet"]
    } else {
        ["nethernet", "raknet"]
    };
    let mut errors = Vec::new();
    for protocol in order {
        let attempt = if protocol == "nethernet" {
            nethernet_ping(addr).await
        } else {
            raknet_ping(addr).await
        };
        match attempt {
            Ok(info) => {
                return ProbeResult {
                    outcome: Ok((protocol, info)),
                    latency_ms: started.elapsed().as_millis() as u64,
                };
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    ProbeResult {
        outcome: Err(errors.join("; ")),
        latency_ms: started.elapsed().as_millis() as u64,
    }
}

/// Whether the game server is up long enough to be probed.
fn game_ready(game: Option<&watch::Receiver<Snapshot>>) -> GameState {
    let Some(game) = game else {
        return GameState::Ready;
    };
    let snap = game.borrow();
    match snap.status {
        Status::Running => match snap.started_at {
            Some(at) if at.elapsed() < BOOT_GRACE => GameState::Booting(BOOT_GRACE - at.elapsed()),
            _ => GameState::Ready,
        },
        Status::Starting => GameState::Booting(BOOT_GRACE),
        _ => GameState::Offline,
    }
}

enum GameState {
    Ready,
    Booting(Duration),
    Offline,
}

/// Record one probe result. Health only turns unhealthy after
/// [`FAILURES_BEFORE_ALERT`] consecutive failures.
fn record(health: &watch::Sender<PingHealth>, target: &str, transport: &str, result: ProbeResult) {
    let now = now_f64();
    health.send_if_modified(|h| {
        let before = h.clone();
        h.target = Some(target.to_owned());
        h.transport = Some(transport.to_owned());
        h.server_running = Some(true);
        h.last_checked = Some(now);
        h.latency_ms = Some(result.latency_ms);
        match &result.outcome {
            Ok((protocol, info)) => {
                h.ok = Some(true);
                h.protocol = Some((*protocol).to_owned());
                h.server = Some(info.clone());
                h.error = None;
                h.failing_since = None;
                h.consecutive_failures = 0;
            }
            Err(e) => {
                h.consecutive_failures = h.consecutive_failures.saturating_add(1);
                h.error = Some(e.clone());
                h.protocol = None;
                if h.failing_since.is_none() {
                    h.failing_since = Some(now);
                }
                if h.consecutive_failures >= FAILURES_BEFORE_ALERT {
                    h.ok = Some(false);
                    h.server = None;
                }
            }
        }
        // last_checked/latency change every round; only notify on verdicts.
        before.ok != h.ok
            || before.target != h.target
            || before.protocol != h.protocol
            || before.server != h.server
            || before.server_running != h.server_running
            || before.error != h.error
    });
}

/// Probe the relay target while the broadcaster runs so a misconfigured
/// target (the "failed to ping server" trap) shows up as structured health
/// instead of a buried log line. The jar's own ping cadence is 30s; this
/// matches it, with a quick first probe so the page gets a verdict fast.
pub(super) async fn ping_poller(
    config: Arc<Config>,
    mut status: watch::Receiver<BcSnapshot>,
    mut game: Option<watch::Receiver<Snapshot>>,
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
        let mut wait = FIRST_PROBE_DELAY;
        // While running: re-read the config each round so edits apply
        // without a panel restart, probe, and record the verdict.
        loop {
            let game_changed = async {
                match game.as_mut() {
                    Some(g) => {
                        if g.changed().await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
                // A game start/stop re-evaluates promptly instead of waiting
                // out a whole interval with a stale verdict.
                _ = game_changed => { wait = FIRST_PROBE_DELAY; continue; }
            }
            wait = PING_INTERVAL;
            if status.borrow().status != BcStatus::Running {
                health.send_if_modified(|h| {
                    let cleared = *h != PingHealth::default();
                    *h = PingHealth::default();
                    cleared
                });
                break;
            }
            let facts = server_facts(&config);
            let analysis =
                match tokio::fs::read_to_string(config.broadcaster_dir.join("config.yml")).await {
                    Ok(text) => analyze_relay_config(&text, facts.port),
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
            let target = ping_target(&analysis, facts.port);
            match game_ready(game.as_ref()) {
                GameState::Offline => {
                    // Nothing to reach: no verdict, and no stale alarm.
                    health.send_if_modified(|h| {
                        let next = PingHealth {
                            target: Some(target.clone()),
                            transport: Some(facts.transport.clone()),
                            server_running: Some(false),
                            ..PingHealth::default()
                        };
                        let changed = *h != next;
                        *h = next;
                        changed
                    });
                    continue;
                }
                GameState::Booting(left) => {
                    wait = left.max(Duration::from_secs(2));
                    continue;
                }
                GameState::Ready => {}
            }
            let result = probe(&target, &facts.transport).await;
            if let Err(e) = &result.outcome {
                tracing::debug!(target = %target, error = %e, "broadcaster query-target probe failed");
                // Re-check soon so a single dropped packet clears quickly.
                wait = Duration::from_secs(10);
            }
            record(&health, &target, &facts.transport, result);
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
                    // The game server being down explains the jar's failure;
                    // that is not a reachability problem worth a banner.
                    if h.ok == Some(false) || h.server_running == Some(false) {
                        false
                    } else {
                        h.ok = Some(false);
                        h.error = Some("the broadcaster logged \"Failed to ping server\"".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn nethernet_http_signaling_counts_as_reachable() {
        // A NetherNet-transport server: TCP only, no UDP answer at all.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                assert!(request.starts_with("GET /v1/join"), "{request}");
                let body = r#"{"name":"Furralis Craft","protocol":2225,"version":"1.26.65","level":"Realism","players":2,"maxPlayers":10,"gameType":0}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        let result = probe(&addr.to_string(), "nethernet").await;
        let (protocol, info) = result.outcome.expect("reachable over NetherNet");
        assert_eq!(protocol, "nethernet");
        assert_eq!(info.name, "Furralis Craft");
        assert_eq!(info.version, "1.26.65");
        assert_eq!(info.players, Some(2));
        assert_eq!(info.max_players, Some(10));
    }

    #[tokio::test]
    async fn raknet_pong_counts_as_reachable_even_when_transport_says_nethernet() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let (_, from) = socket.recv_from(&mut buf).await.unwrap();
            let motd = b"MCPE;Omni;2225;1.26.65;3;20;1;World;Survival;1;0;0;";
            let mut pong = vec![0x1c];
            pong.extend_from_slice(&[0u8; 16]);
            pong.extend_from_slice(&RAKNET_MAGIC);
            pong.extend_from_slice(&(motd.len() as u16).to_be_bytes());
            pong.extend_from_slice(motd);
            socket.send_to(&pong, from).await.unwrap();
        });
        // Nothing listens on TCP at this port, so NetherNet fails first and
        // the RakNet fallback must still produce a healthy verdict.
        let result = probe(&addr.to_string(), "nethernet").await;
        let (protocol, info) = result.outcome.expect("reachable over RakNet");
        assert_eq!(protocol, "raknet");
        assert_eq!(info.name, "Omni");
        assert_eq!(info.players, Some(3));
        assert_eq!(info.max_players, Some(20));
        assert_eq!(info.level, "World");
    }

    #[tokio::test]
    async fn dead_target_reports_every_protocol_tried() {
        // Bind then drop to get a port nobody listens on.
        let port = {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            s.local_addr().unwrap().port()
        };
        let result = probe(&format!("127.0.0.1:{port}"), "raknet").await;
        let error = result.outcome.expect_err("unreachable");
        assert!(
            error.contains("RakNet") && error.contains("NetherNet"),
            "{error}"
        );
    }

    #[test]
    fn one_failure_does_not_raise_the_alarm() {
        let (tx, _rx) = watch::channel(PingHealth::default());
        let fail = || ProbeResult {
            outcome: Err("x".into()),
            latency_ms: 1,
        };
        record(&tx, "127.0.0.1:1", "nethernet", fail());
        assert_eq!(tx.borrow().ok, None, "a single miss must not alarm");
        record(&tx, "127.0.0.1:1", "nethernet", fail());
        assert_eq!(tx.borrow().ok, Some(false));
        record(
            &tx,
            "127.0.0.1:1",
            "nethernet",
            ProbeResult {
                outcome: Ok(("nethernet", PongInfo::default())),
                latency_ms: 1,
            },
        );
        assert_eq!(tx.borrow().ok, Some(true));
        assert_eq!(tx.borrow().consecutive_failures, 0);
        assert!(tx.borrow().failing_since.is_none());
    }
}
