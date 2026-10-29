//! MCXboxBroadcast Standalone integration.
//!
//! Every server instance owns its own broadcaster: a Java process running in
//! that instance's broadcaster directory holding `config.yml`,
//! `screenshot.jpg`, and the `cache/` token store, so accounts, settings, and
//! sign-in state never leak between instances. Output streams through a
//! per-instance Console ring like the game server; device-code prompts
//! emitted during Microsoft authentication are parsed out of the log stream so
//! the UI can surface the code and verification URL.
//!
//! The jar itself is shared: instance dirs stay small because `jar_path`
//! falls back to the panel-level broadcaster directory when the instance has
//! no jar of its own.
//!
//! Lifecycle safety nets:
//! - Exactly one broadcaster per instance. Before spawning, the panel scans
//!   for a foreign `java -jar …/{JAR_NAME}` process working in this
//!   instance's broadcaster directory (started by hand or orphaned by a
//!   previous panel run) and refuses to start a second one; the UI can stop
//!   or take over the foreign process instead.
//! - Relay-target validation at every start. `session.remote-address` is the
//!   offer-relay target and MUST be loopback on a co-located setup; a public
//!   IP there is announced loudly because hairpin NAT breaks every join.
use crate::server::{Console, read_pipe};
use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncWriteExt,
    net::UdpSocket,
    process::{Child, Command},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;

pub const JAR_NAME: &str = "MCXboxBroadcastStandalone.jar";
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a takeover waits for a foreign broadcaster to exit after SIGTERM.
const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(10);
/// RakNet ping budget, matching the jar's own query timeout.
const PING_TIMEOUT: Duration = Duration::from_millis(1500);
/// How often the panel re-probes the relay target while the broadcaster runs.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// RakNet offline magic shared by unconnected ping/pong packets.
const RAKNET_MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];

static DEVICE_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)open the page (\S+?) and enter the code ([A-Za-z0-9-]+)").unwrap()
});
static SUB_SESSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[Sub-Session ([^\]]+)\]").unwrap());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BcStatus {
    #[default]
    Stopped,
    Running,
    Stopping,
    Crashed,
}
#[derive(Clone, Debug, Serialize)]
pub struct BcSnapshot {
    pub status: BcStatus,
    pub pid: Option<u32>,
    pub uptime: f64,
    #[serde(skip)]
    pub started_at: Option<Instant>,
}
impl Default for BcSnapshot {
    fn default() -> Self {
        Self {
            status: BcStatus::Stopped,
            pid: None,
            uptime: 0.0,
            started_at: None,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct DeviceCode {
    pub account: String,
    pub code: String,
    pub url: String,
    pub at: f64,
}

/// One operational address parsed out of config.yml with a co-location verdict.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RelayTarget {
    /// Rendered `ip:port` after resolving `auto` the way the jar does.
    pub text: String,
    /// True when the address is globally routable — on this panel the
    /// broadcaster always shares the host with the server, so a public relay
    /// target is the classic hairpin-NAT join failure.
    pub public: bool,
}

/// The config.yml fields this panel validates and surfaces. Roles differ:
/// `session.remote-address/remote-port` relay join offers (and seed the
/// transfer address), while `session-info.ip/port` only survive in legacy
/// pre-v7 configs which the jar migrates into the relay keys on load.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RelayAnalysis {
    pub query_server: bool,
    pub relay: Option<RelayTarget>,
    pub legacy: Option<RelayTarget>,
}

impl RelayAnalysis {
    fn unknown() -> Self {
        Self {
            query_server: true,
            relay: None,
            legacy: None,
        }
    }
}

/// Panel-side health of the broadcaster's query ping against the server.
/// `ok: None` means no verdict yet (broadcaster stopped, or query-server
/// disabled so pings are not part of the deal).
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
pub struct PingHealth {
    pub target: Option<String>,
    pub ok: Option<bool>,
    pub failing_since: Option<f64>,
    pub last_checked: Option<f64>,
}

/// A broadcaster process for this instance that the panel did not spawn.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ExternalBroadcaster {
    pub pid: u32,
    /// True when the pidfile says this process was spawned by a previous
    /// panel run (it kept running after the panel went away).
    pub orphaned: bool,
}

/// Sidecar container hosting for the broadcaster (docker deployment backend,
/// Linux host networking — see `instances::broadcaster_docker_spec`). The
/// container has NO restart policy: the panel owns its lifecycle exactly like
/// the native Java child, only through the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerSpec {
    pub container: String,
    pub image: String,
}

type Reply = oneshot::Sender<anyhow::Result<String>>;
enum Control {
    Start(Reply),
    Stop(Reply),
    Restart(Reply),
    /// Stop a foreign broadcaster for this instance, then start a panel-owned
    /// one in its place.
    TakeOver(Reply),
    Command(String, oneshot::Sender<anyhow::Result<()>>),
    Shutdown(Reply),
}
struct Running {
    child: Child,
    pid: u32,
    /// The stdin writer task; `None` for the docker sidecar (its commands go
    /// through the FIFO, and the child is only a `docker logs` reader).
    stdin: Option<mpsc::Sender<String>>,
    writer: Option<tokio::task::JoinHandle<()>>,
    readers: Vec<tokio::task::JoinHandle<()>>,
    /// Set when the child reads the sidecar container's logs.
    container: Option<String>,
    /// Sidecar command FIFO on the host (inside the mounted dir).
    fifo: Option<PathBuf>,
}

#[derive(Clone)]
pub struct Broadcaster {
    control: mpsc::Sender<Control>,
    pub status: watch::Receiver<BcSnapshot>,
    pub console: Console,
    pub device_codes: watch::Receiver<BTreeMap<String, DeviceCode>>,
    /// `java -version` first line, None until detection finishes or if absent.
    pub java: watch::Receiver<Option<String>>,
    /// Panel-side query-ping health against the relay target.
    pub ping: watch::Receiver<PingHealth>,
    config: Arc<Config>,
    /// Panel-level broadcaster directory used to resolve the jar when this
    /// instance's own dir has none. `None` for the default instance, whose
    /// dir IS the panel dir.
    jar_home: Option<PathBuf>,
    /// Sidecar hosting spec; `Some` only for docker instances on Linux.
    docker: watch::Sender<Option<DockerSpec>>,
}

impl Broadcaster {
    pub fn new(
        config: Arc<Config>,
        shutdown: CancellationToken,
        jar_home: Option<PathBuf>,
        docker: Option<DockerSpec>,
    ) -> anyhow::Result<Self> {
        let console = Console::with_journal_root(
            config.logs_dir.join("broadcaster-console"),
            config.console_max_lines,
            config.console_max_bytes,
            (config.console_journal_bytes / 4).max(4 * 1024 * 1024),
        );
        let (status_tx, status) = watch::channel(BcSnapshot::default());
        let (codes_tx, device_codes) = watch::channel(BTreeMap::new());
        let (java_tx, java) = watch::channel(None);
        let (ping_tx, _) = watch::channel(PingHealth::default());
        let ping_tx = Arc::new(ping_tx);
        let (docker_tx, docker_rx) = watch::channel(docker);
        let (control, control_rx) = mpsc::channel(16);
        let manager = Self {
            control,
            status: status.clone(),
            console: console.clone(),
            device_codes,
            java: java.clone(),
            ping: ping_tx.subscribe(),
            config: config.clone(),
            jar_home: jar_home.clone(),
            docker: docker_tx,
        };
        // Detect the Java runtime in the background; startup never blocks on it.
        let detect_config = config.clone();
        tokio::spawn(async move {
            let detected = detect_java(&detect_config).await;
            java_tx.send_replace(detected);
        });
        // Flush the broadcaster journal like the game console does.
        let flush_console = console.clone();
        let flush_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = flush_shutdown.cancelled() => { let _ = flush_console.flush().await; break; }
                    _ = interval.tick() => {
                        if let Err(e) = flush_console.flush().await {
                            tracing::error!(%e, "broadcaster journal flush failed");
                        }
                    }
                }
            }
        });
        tokio::spawn(watch_device_codes(console.clone(), codes_tx));
        // Ping failures are only an ERROR line in the jar's output — surface
        // them as structured health so the UI can raise a banner (Bug 4).
        tokio::spawn(watch_ping_failures(console.clone(), ping_tx.clone()));
        tokio::spawn(ping_poller(
            config.clone(),
            status.clone(),
            ping_tx,
            shutdown.clone(),
        ));
        tokio::spawn(supervise(
            config,
            jar_home.clone(),
            control_rx,
            status_tx,
            console,
            shutdown,
            docker_rx,
        ));
        Ok(manager)
    }

    /// Switch sidecar hosting (deployment changes are only allowed while the
    /// instance is stopped, so no running process has to be migrated).
    pub fn set_docker(&self, docker: Option<DockerSpec>) {
        self.docker.send_replace(docker);
    }

    pub fn dir(&self) -> PathBuf {
        self.config.broadcaster_dir.clone()
    }
    /// Jar resolution: explicit override → this instance's dir → the
    /// panel-level broadcaster dir, so a shared jar serves every instance
    /// without copies while a per-instance jar still wins.
    pub fn jar_path(&self) -> PathBuf {
        resolve_jar(&self.config, self.jar_home.as_deref())
    }
    pub fn config_path(&self) -> PathBuf {
        self.config.broadcaster_dir.join("config.yml")
    }
    pub fn screenshot_path(&self, account: &str) -> PathBuf {
        if account.is_empty() || account.eq_ignore_ascii_case("main") {
            self.config.broadcaster_dir.join("screenshot.jpg")
        } else {
            self.config
                .broadcaster_dir
                .join("cache")
                .join(account)
                .join("screenshot.jpg")
        }
    }
    pub fn snapshot(&self) -> BcSnapshot {
        let mut s = self.status.borrow().clone();
        s.uptime = s
            .started_at
            .filter(|_| s.status == BcStatus::Running)
            .map_or(0.0, |i| i.elapsed().as_secs_f64());
        s
    }
    pub fn ping_health(&self) -> PingHealth {
        self.ping.borrow().clone()
    }
    /// The instance port the relay target should point at (server.properties
    /// `server-port`, default 19132) — used to seed fresh configs and to
    /// resolve `auto` ports the way this deployment actually runs.
    pub fn instance_port(&self) -> u16 {
        instance_port(&self.config)
    }
    /// Parse the on-disk config.yml for the relay targets; `None` when the
    /// file is missing or unreadable (read_config is deliberately not called:
    /// a status poll must not materialize a template).
    pub async fn relay_status(&self) -> RelayAnalysis {
        match tokio::fs::read_to_string(self.config_path()).await {
            Ok(text) => analyze_relay_config(&text, self.instance_port()),
            Err(_) => RelayAnalysis::unknown(),
        }
    }
    /// A broadcaster java process for this instance that the panel did not
    /// spawn, if one is running right now (Linux only; unsupported platforms
    /// cannot scan for it). The panel's own child is excluded so the field
    /// stays "foreign process" and never describes ourselves.
    pub async fn external(&self) -> Option<ExternalBroadcaster> {
        // Bind first: a `status.borrow()` temporary inside the await
        // expression would be held across it and make this future !Send.
        let exclude = self.status.borrow().pid;
        find_external(&self.config.broadcaster_dir, exclude).await
    }
    pub async fn action(&self, action: &str) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        let control = match action {
            "start" => Control::Start(tx),
            "stop" => Control::Stop(tx),
            "restart" => Control::Restart(tx),
            "takeover" => Control::TakeOver(tx),
            other => bail!("unknown broadcaster action '{other}'"),
        };
        self.control
            .send(control)
            .await
            .context("broadcaster stopped")?;
        rx.await.context("broadcaster stopped")?
    }
    pub async fn send_command(&self, command: &str) -> anyhow::Result<()> {
        if command.trim().is_empty() || command.len() > 8192 || command.contains(['\r', '\n', '\0'])
        {
            bail!("command must be a single nonempty line of at most 8192 bytes");
        }
        if self.snapshot().status != BcStatus::Running {
            bail!("Broadcaster is not running");
        }
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Command(command.into(), tx))
            .await
            .context("broadcaster stopped")?;
        rx.await.context("broadcaster stopped")?
    }

    /// Read the raw config.yml, seeding the documented default when absent.
    /// The template's `{{PORT}}` placeholder gets the instance's real
    /// server-port so a fresh config's relay target is immediately correct
    /// even when the instance does not run on 19132.
    pub async fn read_config(&self) -> anyhow::Result<String> {
        let path = self.config_path();
        tokio::fs::create_dir_all(self.dir()).await?;
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let seeded = DEFAULT_CONFIG.replace("{{PORT}}", &self.instance_port().to_string());
                atomic_write(&path, seeded.as_bytes()).await?;
                Ok(seeded)
            }
            Err(e) => Err(e.into()),
        }
    }
    /// Validate and persist config.yml.
    pub async fn write_config(&self, text: &str) -> anyhow::Result<()> {
        if text.len() > 512 * 1024 {
            bail!("config.yml is too large");
        }
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(text).context("config.yml is not valid YAML")?;
        if !parsed.is_mapping() {
            bail!("config.yml must be a YAML mapping");
        }
        atomic_write(&self.config_path(), text.as_bytes()).await
    }
    /// Accounts from cache/sub_sessions.json plus the implicit main account.
    pub async fn accounts(&self) -> Value {
        let mut accounts = vec![json!({"id": "main", "main": true})];
        if let Ok(text) =
            tokio::fs::read_to_string(self.dir().join("cache").join("sub_sessions.json")).await
            && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text)
        {
            for item in items {
                if let Some(id) = item.as_str() {
                    accounts.push(json!({"id": id, "main": false}));
                }
            }
        }
        json!(accounts)
    }
    pub fn device_codes(&self) -> BTreeMap<String, DeviceCode> {
        self.device_codes.borrow().clone()
    }
    pub async fn shutdown(&self) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Shutdown(tx))
            .await
            .context("broadcaster stopped")?;
        rx.await.context("broadcaster stopped")?
    }
}

/// Scan console lines for Microsoft device-code prompts, keyed per account.
async fn watch_device_codes(console: Console, codes: watch::Sender<BTreeMap<String, DeviceCode>>) {
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
            if let Some(captures) = DEVICE_CODE.captures(&line.line) {
                let account = SUB_SESSION
                    .captures(&line.line)
                    .map_or("main".to_string(), |c| c[1].to_string());
                codes.send_modify(|map| {
                    map.insert(
                        account.clone(),
                        DeviceCode {
                            account,
                            code: captures[2].to_string(),
                            url: captures[1].to_string(),
                            at: line.timestamp,
                        },
                    );
                });
            }
        }
    }
}

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
async fn ping_poller(
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
async fn watch_ping_failures(console: Console, health: Arc<watch::Sender<PingHealth>>) {
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

/// Jar resolution shared by `Broadcaster::jar_path` and the supervisor:
/// explicit override, then the instance's own dir, then the panel-level dir.
fn resolve_jar(config: &Config, jar_home: Option<&std::path::Path>) -> PathBuf {
    if let Some(jar) = &config.broadcaster_jar {
        return jar.clone();
    }
    let local = config.broadcaster_dir.join(JAR_NAME);
    if local.exists() {
        return local;
    }
    if let Some(home) = jar_home {
        let shared = home.join(JAR_NAME);
        if shared.exists() {
            return shared;
        }
    }
    local
}

/// The instance's RakNet port from server.properties; 19132 when absent.
fn instance_port(config: &Config) -> u16 {
    let Ok(text) = std::fs::read_to_string(config.bedrock_server_dir.join("server.properties"))
    else {
        return 19132;
    };
    text.lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("server-port=")?;
            value.trim().parse::<u16>().ok()
        })
        .unwrap_or(19132)
}

/// True for globally routable addresses — the ones a co-located broadcaster
/// must never use as its relay target because reaching them loops through the
/// router (hairpin NAT), which most consumer routers refuse.
fn publicly_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                // 100.64/10 carrier-grade NAT (Tailscale and friends): locally
                // bound on this host's VPN interface, so relaying to it works.
                || v4.octets()[0] == 100)
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified())
        }
    }
}

/// Resolve one config address/port pair. `auto` resolves exactly like the
/// jar's standalone bootstrap: loopback, with this instance's real port
/// instead of the jar's hardcoded 19132 default. Hostnames pass through
/// unvalidated (no IP to judge routability with).
fn resolve_target(
    addr: Option<&serde_yaml::Value>,
    port: Option<&serde_yaml::Value>,
    fallback_port: u16,
) -> RelayTarget {
    fn as_text(value: Option<&serde_yaml::Value>) -> Option<String> {
        match value {
            Some(serde_yaml::Value::String(s)) => Some(s.trim().to_string()),
            Some(serde_yaml::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }
    let addr_text = as_text(addr).filter(|s| !s.is_empty());
    let port_text = as_text(port).filter(|s| !s.is_empty());
    let ip = match addr_text.as_deref() {
        None | Some("auto") => IpAddr::from([127, 0, 0, 1]),
        Some(text) => text
            .parse::<IpAddr>()
            .unwrap_or(IpAddr::from([127, 0, 0, 1])),
    };
    let port = match port_text.as_deref() {
        None | Some("auto") => fallback_port,
        Some(text) => text.parse::<u16>().unwrap_or(fallback_port),
    };
    RelayTarget {
        text: format!("{ip}:{port}"),
        public: publicly_routable(ip),
    }
}

/// Parse config.yml for everything the panel validates: the operational
/// relay target, the legacy session-info address, and whether query pings
/// are enabled at all.
fn analyze_relay_config(text: &str, fallback_port: u16) -> RelayAnalysis {
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return RelayAnalysis::unknown();
    };
    let session = doc.get("session");
    let query_server = session
        .and_then(|s| s.get("query-server"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let relay = match session {
        Some(s) if s.get("remote-address").is_some() || s.get("remote-port").is_some() => Some(
            resolve_target(s.get("remote-address"), s.get("remote-port"), fallback_port),
        ),
        _ => None,
    };
    let legacy = session
        .and_then(|s| s.get("session-info"))
        .filter(|info| info.get("ip").is_some() || info.get("port").is_some())
        .map(|info| resolve_target(info.get("ip"), info.get("port"), fallback_port));
    RelayAnalysis {
        query_server,
        relay,
        legacy,
    }
}

/// Announce the resolved relay targets at every broadcaster start and warn
/// loudly about the public-IP trap that costs hours to diagnose by log alone.
async fn announce_relay(config: &Config, console: &Console) {
    let text = match tokio::fs::read_to_string(config.broadcaster_dir.join("config.yml")).await {
        Ok(text) => text,
        Err(e) => {
            console
                .publish(
                    format!("[LOCAL] Could not read config.yml for relay validation: {e}"),
                    "panel",
                    false,
                )
                .await;
            return;
        }
    };
    let analysis = analyze_relay_config(&text, instance_port(config));
    match &analysis.relay {
        Some(target) => {
            console
                .publish(
                    format!(
                        "[LOCAL] Relay target (session.remote-address): {}",
                        target.text
                    ),
                    "panel",
                    false,
                )
                .await;
            if target.public {
                console
                    .publish(
                        format!(
                            "[LOCAL] WARNING: the relay target {} is a PUBLIC IP, but this \
                             broadcaster runs on the same host as the server. Routers refuse the \
                             hairpin loop, so friends-list joins will fail (\"Failed to relay \
                             offer ... connection refused\", client-side InitialConnection-81). \
                             Set session.remote-address to 127.0.0.1.",
                            target.text
                        ),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
        None => {
            console
                .publish(
                    "[LOCAL] config.yml has no session.remote-address; the jar default (auto = \
                     127.0.0.1) applies."
                        .to_string(),
                    "panel",
                    false,
                )
                .await;
        }
    }
    if let Some(legacy) = &analysis.legacy {
        console
            .publish(
                format!(
                    "[LOCAL] Legacy session-info address: {} {}",
                    legacy.text,
                    if legacy.public {
                        "(PUBLIC — this pre-v7 key becomes the relay target on jar migration; \
                         switch it to 127.0.0.1)"
                    } else {
                        "(migrated into session.remote-address by the jar on load)"
                    }
                ),
                "panel",
                false,
            )
            .await;
    }
}

/// The panel writes its child's PID here so a restarted panel can tell
/// orphaned sidecars ("adopt/replace me") from manually started ones.
fn pidfile_path(config: &Config) -> PathBuf {
    config.broadcaster_dir.join("panel-broadcaster.pid")
}

async fn write_pidfile(config: &Config, pid: u32) {
    let data = format!("{pid}\n");
    if let Err(e) = atomic_write(&pidfile_path(config), data.as_bytes()).await {
        tracing::warn!(%e, "broadcaster pidfile write failed");
    }
}

async fn clear_pidfile(config: &Config) {
    let _ = tokio::fs::remove_file(pidfile_path(config)).await;
}

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
async fn kill_external(ext: &ExternalBroadcaster, dir: &Path) -> anyhow::Result<()> {
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
async fn kill_external(_ext: &ExternalBroadcaster, _dir: &Path) -> anyhow::Result<()> {
    bail!("external broadcaster detection is not supported on this platform")
}

fn java_command(config: &Config, jar: &std::path::Path) -> Command {
    let mut cmd = Command::new(
        config
            .java_bin
            .as_deref()
            .unwrap_or(std::path::Path::new("java")),
    );
    cmd.arg("-jar")
        .arg(jar)
        .current_dir(&config.broadcaster_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

async fn launch(
    jar: &std::path::Path,
    config: &Config,
    console: &Console,
    docker: Option<&DockerSpec>,
) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.broadcaster_dir).await?;
    if !jar.exists() {
        bail!(
            "broadcaster jar not found at {}; place {} there or set BROADCASTER_JAR",
            jar.display(),
            JAR_NAME
        );
    }
    let container = docker.map(|spec| spec.container.clone());
    let fifo = docker.map(|_| crate::docker::host_stdin_fifo(&config.broadcaster_dir));
    let (mut cmd, with_stdin) = match docker {
        // The sidecar's output streams through `docker logs -f`; the child
        // has no stdin of its own.
        Some(spec) => {
            let argv = crate::docker::logs_argv(&spec.container, 200);
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]);
            (cmd, false)
        }
        None => (java_command(config, jar), true),
    };
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if with_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().context(if container.is_some() {
        "failed to read the broadcaster container logs"
    } else {
        "java not found; install Java or set JAVA_BIN"
    })?;
    let pid = child.id().context("child has no PID")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let (tx, writer) = if with_stdin {
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let (tx, mut rx) = mpsc::channel::<String>(config.command_queue_capacity);
        let writer = tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if stdin.write_all(text.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });
        (Some(tx), Some(writer))
    } else {
        (None, None)
    };
    let readers = vec![
        tokio::spawn(read_pipe(
            stdout,
            console.clone(),
            "stdout",
            config.console_max_line_bytes,
        )),
        tokio::spawn(read_pipe(
            stderr,
            console.clone(),
            "stderr",
            config.console_max_line_bytes,
        )),
    ];
    tracing::info!(pid, container = ?container, "broadcaster started");
    Ok(Running {
        child,
        pid,
        stdin: tx,
        writer,
        readers,
        container,
        fifo,
    })
}

/// Run the broadcaster as a sidecar container on the host network so its
/// ICE/UDP behavior matches the native setup (Linux only; see
/// `instances::broadcaster_docker_spec`). The broadcaster directory is
/// mounted at /data (config.yml, token cache, screenshots) and the shared
/// jar read-only, so the container itself stays stateless.
async fn launch_docker(
    spec: &DockerSpec,
    jar: &std::path::Path,
    config: &Config,
    console: &Console,
) -> anyhow::Result<()> {
    crate::docker::ensure_image(
        config,
        &spec.image,
        crate::docker::DOCKERFILE_BROADCASTER,
        console,
    )
    .await?;
    let jar_text = jar.to_string_lossy();
    anyhow::ensure!(
        !jar_text.contains(':'),
        "broadcaster jar path {jar_text} contains ':' and cannot be bind-mounted"
    );
    match crate::docker::state(&spec.container).await.as_deref() {
        Some("running") => {
            console
                .publish(
                    format!(
                        "[LOCAL] Broadcaster container {} is already running; reattaching.",
                        spec.container
                    ),
                    "panel",
                    false,
                )
                .await;
        }
        _ => {
            crate::docker::rm(&spec.container).await;
            // Commands reach the sidecar through a FIFO inside the mounted
            // broadcaster directory (same mechanism as the systemd backend).
            let fifo = crate::docker::host_stdin_fifo(&config.broadcaster_dir);
            crate::systemd::ensure_fifo(&fifo).await?;
            let mut args: Vec<std::ffi::OsString> = vec![
                "docker".into(),
                "run".into(),
                "-d".into(),
                "--name".into(),
                spec.container.clone().into(),
            ];
            if cfg!(target_os = "linux") {
                // Host networking keeps ICE/UDP behavior identical to the
                // native broadcaster process.
                args.push("--network".into());
                args.push("host".into());
            }
            #[cfg(unix)]
            {
                let uid = nix::unistd::getuid();
                let gid = nix::unistd::getgid();
                args.push("--user".into());
                args.push(format!("{}:{}", uid.as_raw(), gid.as_raw()).into());
            }
            args.push("-v".into());
            args.push(
                format!(
                    "{}:/data",
                    config.broadcaster_dir.to_string_lossy().replace('\\', "/")
                )
                .into(),
            );
            args.push("-v".into());
            args.push(format!("{jar_text}:/data/{JAR_NAME}:ro").into());
            args.push("-w".into());
            args.push("/data".into());
            args.push(spec.image.clone().into());
            args.extend(crate::docker::command_override(&[
                "java".into(),
                "-jar".into(),
                format!("/data/{JAR_NAME}").into(),
            ]));
            let output = tokio::time::timeout(
                Duration::from_secs(120),
                Command::new(&args[0])
                    .args(&args[1..])
                    .stdin(Stdio::null())
                    .output(),
            )
            .await
            .context("docker run timed out")?
            .context("failed to run docker")?;
            if !output.status.success() {
                bail!(
                    "docker run for the broadcaster failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            console
                .publish(
                    format!(
                        "[LOCAL] Broadcaster container {} started (host network).",
                        spec.container
                    ),
                    "panel",
                    false,
                )
                .await;
        }
    }
    Ok(())
}

fn signal_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let signal = if force {
            Signal::SIGKILL
        } else {
            Signal::SIGTERM
        };
        if let Err(e) = killpg(Pid::from_raw(pid as i32), signal)
            && e != nix::errno::Errno::ESRCH
        {
            tracing::warn!(%e, "broadcaster group signal failed");
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}

async fn terminate(running: &mut Running) -> anyhow::Result<()> {
    if let Some(container) = running.container.clone() {
        // The sidecar has no restart policy, but the daemon owns the
        // process: graceful `stop` through the command FIFO, then `docker
        // stop` (SIGTERM), then `docker kill`.
        let Some(fifo) = running.fifo.clone() else {
            return Ok(());
        };
        if let Err(e) = write_fifo(&fifo, "stop").await {
            tracing::debug!(%e, "broadcaster fifo stop failed; escalating");
        }
        if tokio::time::timeout(Duration::from_secs(3), running.child.wait())
            .await
            .is_ok()
        {
            return Ok(());
        }
        if let Err(e) = crate::docker::stop(&container, 5).await {
            tracing::warn!(%e, container, "broadcaster docker stop failed; escalating");
            crate::docker::kill(&container).await;
        }
        if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
            .await
            .is_err()
        {
            crate::docker::kill(&container).await;
            let _ = tokio::time::timeout(STOP_TIMEOUT, running.child.wait()).await;
        }
        return Ok(());
    }
    let _ = running
        .stdin
        .as_ref()
        .context("broadcaster lost its stdin")?
        .send("stop".into())
        .await;
    if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    signal_tree(running.pid, false);
    #[cfg(not(unix))]
    running.child.start_kill()?;
    if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    signal_tree(running.pid, true);
    running.child.kill().await.context("kill broadcaster")
}

async fn drain(mut running: Running) {
    // For a logs reader this only tears down panel plumbing; the
    // container's lifetime belongs to the daemon.
    signal_tree(running.pid, true);
    if let Some(writer) = running.writer.take() {
        writer.abort();
        let _ = writer.await;
    }
    for mut reader in running.readers.drain(..) {
        if tokio::time::timeout(Duration::from_secs(10), &mut reader)
            .await
            .is_err()
        {
            reader.abort();
            let _ = reader.await;
        }
    }
}

async fn supervise(
    config: Arc<Config>,
    jar_home: Option<PathBuf>,
    mut control: mpsc::Receiver<Control>,
    status: watch::Sender<BcSnapshot>,
    console: Console,
    shutdown: CancellationToken,
    docker_rx: watch::Receiver<Option<DockerSpec>>,
) {
    let mut running: Option<Running> = None;
    loop {
        enum Wake {
            Control(Option<Control>),
            Exit(std::io::Result<std::process::ExitStatus>),
            Shutdown,
        }
        let wake = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Wake::Shutdown,
            c = control.recv() => Wake::Control(c),
            exit = async { match running.as_mut() { Some(r) => r.child.wait().await, None => std::future::pending().await } } => Wake::Exit(exit),
        };
        match wake {
            Wake::Shutdown | Wake::Control(None) => {
                if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    if let Err(e) = terminate(&mut r).await {
                        tracing::error!(%e, "broadcaster shutdown failed");
                    }
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                }
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let _ = terminate(&mut r).await;
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                }
                let _ = reply.send(Ok("Broadcaster stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Start(reply))) => {
                let result = if running.is_some() {
                    Ok("Broadcaster is already running".into())
                } else {
                    match start_owned(&config, &jar_home, &console, &docker_rx, &status).await {
                        Ok(r) => {
                            let pid = r.pid;
                            running = Some(r);
                            Ok(format!("Broadcaster started (PID: {pid})"))
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::TakeOver(reply))) => {
                let result = async {
                    if running.is_some() {
                        anyhow::bail!("Broadcaster is already running");
                    }
                    let docker_spec = docker_rx.borrow().clone();
                    if docker_spec.is_none() {
                        // Exactly one broadcaster per Xbox account: replace a
                        // manually started (or panel-orphaned) java before
                        // spawning our own.
                        let dir = &config.broadcaster_dir;
                        if let Some(ext) = find_external(dir, None).await {
                            let origin = if ext.orphaned {
                                "left over from a previous panel run"
                            } else {
                                "started outside the panel"
                            };
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] Stopping external broadcaster (PID {}, {}) \
                                         before starting a panel-managed one.",
                                        ext.pid, origin
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                            kill_external(&ext, dir).await?;
                        }
                    }
                    let r = start_owned(&config, &jar_home, &console, &docker_rx, &status).await?;
                    let pid = r.pid;
                    running = Some(r);
                    Ok(format!("Broadcaster taken over (PID: {pid})"))
                }
                .await;
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Stop(reply))) => {
                let result = if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let result = terminate(&mut r).await;
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                    result.map(|_| "Broadcaster stopped".into())
                } else {
                    // Nothing panel-owned: a foreign broadcaster for this
                    // instance can still be stopped from here so the UI's
                    // Stop button works for externally started javas too.
                    async {
                        let docker_spec = docker_rx.borrow().clone();
                        if docker_spec.is_none() {
                            let dir = &config.broadcaster_dir;
                            if let Some(ext) = find_external(dir, None).await {
                                console
                                    .publish(
                                        format!(
                                            "[LOCAL] Stopping external broadcaster (PID {})...",
                                            ext.pid
                                        ),
                                        "panel",
                                        false,
                                    )
                                    .await;
                                kill_external(&ext, dir).await?;
                                return Ok("External broadcaster stopped".into());
                            }
                        }
                        Ok("Broadcaster is not running".into())
                    }
                    .await
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Restart(reply))) => {
                let result = async {
                    if let Some(mut r) = running.take() {
                        status.send_modify(|s| s.status = BcStatus::Stopping);
                        let stopped = terminate(&mut r).await;
                        drain(r).await;
                        mark_stopped(&status, &config).await;
                        stopped?;
                    }
                    let r = start_owned(&config, &jar_home, &console, &docker_rx, &status).await?;
                    let pid = r.pid;
                    running = Some(r);
                    Ok(format!("Broadcaster restarted (PID: {pid})"))
                }
                .await;
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Command(text, reply))) => {
                let result = match running.as_ref() {
                    Some(r) => match &r.stdin {
                        Some(stdin) => stdin
                            .send(text)
                            .await
                            .map(|_| ())
                            .map_err(|e| anyhow::anyhow!("stdin closed: {e}")),
                        None => {
                            // Sidecar: write the command into the FIFO the
                            // container's java process reads its stdin from.
                            match &r.fifo {
                                Some(fifo) => write_fifo(fifo, &text).await,
                                None => Err(anyhow::anyhow!("sidecar has no command FIFO")),
                            }
                        }
                    },
                    None => Err(anyhow::anyhow!("Broadcaster is not running")),
                };
                let _ = reply.send(result);
            }
            Wake::Exit(exit) => {
                if let Some(r) = running.take() {
                    drain(r).await;
                }
                clear_pidfile(&config).await;
                status.send_modify(|s| {
                    s.status = BcStatus::Crashed;
                    s.pid = None;
                    s.started_at = None;
                });
                console
                    .publish(
                        format!("[LOCAL] Broadcaster exited unexpectedly ({exit:?})"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    }
}

/// Reset the snapshot to Stopped and retire the pidfile: nothing panel-owned
/// is running anymore.
async fn mark_stopped(status: &watch::Sender<BcSnapshot>, config: &Config) {
    status.send_modify(|s| {
        s.status = BcStatus::Stopped;
        s.pid = None;
        s.started_at = None;
    });
    clear_pidfile(config).await;
}

/// The shared launch path for Start / Restart / TakeOver: docker preflight,
/// duplicate guard against foreign broadcasters, spawn, pidfile, and the
/// relay-target boot banner.
async fn start_owned(
    config: &Arc<Config>,
    jar_home: &Option<PathBuf>,
    console: &Console,
    docker_rx: &watch::Receiver<Option<DockerSpec>>,
    status: &watch::Sender<BcSnapshot>,
) -> anyhow::Result<Running> {
    let docker_spec = docker_rx.borrow().clone();
    if docker_spec.is_none() {
        // Two javas answering one Xbox session race each other and the loser
        // is whoever holds the stale config — refuse to spawn a second one.
        let dir = &config.broadcaster_dir;
        if let Some(ext) = find_external(dir, None).await {
            let origin = if ext.orphaned {
                "left over from a previous panel run"
            } else {
                "started outside the panel"
            };
            bail!(
                "a broadcaster is already running for this instance (PID {}, {origin}); \
                 stop it or use Take over before starting a panel-managed one",
                ext.pid
            );
        }
    }
    let preflight = match &docker_spec {
        Some(spec) => {
            launch_docker(
                spec,
                &resolve_jar(config, jar_home.as_deref()),
                config,
                console,
            )
            .await
        }
        None => Ok(()),
    };
    let r = match preflight {
        Err(e) => return Err(e),
        Ok(()) => {
            launch(
                &resolve_jar(config, jar_home.as_deref()),
                config,
                console,
                docker_spec.as_ref(),
            )
            .await?
        }
    };
    status.send_modify(|s| {
        s.status = BcStatus::Running;
        s.pid = Some(r.pid);
        s.started_at = Some(Instant::now());
    });
    // Only native children get a pidfile: the container sidecar has no host
    // java PID to record and duplicates are reattached, not spawned.
    if docker_spec.is_none() {
        write_pidfile(config, r.pid).await;
    }
    announce_relay(config, console).await;
    Ok(r)
}

/// Write one command line into a FIFO, opening read+write so the call never
/// blocks waiting for a reader.
async fn write_fifo(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut fifo = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .await?;
        fifo.write_all(text.as_bytes()).await?;
        fifo.write_all(b"\n").await?;
        fifo.flush().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("FIFO command timed out")?
}

pub const DEFAULT_CONFIG: &str = include_str!("broadcaster_default_config.yml");

/// Probe the configured Java runtime; returns the first `java -version` line.
pub async fn detect_java(config: &Config) -> Option<String> {
    let mut cmd = Command::new(
        config
            .java_bin
            .as_deref()
            .unwrap_or(std::path::Path::new("java")),
    );
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().next().unwrap_or_default().trim();
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_current_v7_config() {
        let text = r"
session:
  remote-address: 127.0.0.1
  remote-port: 24474
  query-server: true
  session-info:
    host-name: Omni
";
        let a = analyze_relay_config(text, 19132);
        assert_eq!(
            a.relay,
            Some(RelayTarget {
                text: "127.0.0.1:24474".into(),
                public: false
            })
        );
        assert!(a.legacy.is_none());
        assert!(a.query_server);
    }

    #[test]
    fn flags_public_relay_target() {
        let text = r"
session:
  remote-address: 203.0.113.7
  remote-port: 24474
";
        let a = analyze_relay_config(text, 19132);
        let relay = a.relay.expect("relay parsed");
        assert!(relay.public, "a public IP must be flagged");
        assert_eq!(relay.text, "203.0.113.7:24474");
    }

    #[test]
    fn private_addresses_are_not_flagged() {
        for ip in [
            "127.0.0.1",
            "192.168.1.115",
            "10.0.0.5",
            "100.74.133.65",
            "::1",
        ] {
            let text = format!("session:\n  remote-address: {ip}\n  remote-port: 19132\n");
            let a = analyze_relay_config(&text, 19132);
            assert!(!a.relay.unwrap().public, "{ip} must not be flagged public");
        }
    }

    #[test]
    fn legacy_session_info_is_detected() {
        // Pre-v7 layout (config-version 5): the address lived in session-info
        // and the jar migrates it into the relay keys on load.
        let text = r"
session:
  query-server: true
  session-info:
    ip: 203.0.113.7
    port: 19132
";
        let a = analyze_relay_config(text, 19132);
        assert!(a.relay.is_none());
        let legacy = a.legacy.expect("legacy parsed");
        assert!(legacy.public);
        assert_eq!(legacy.text, "203.0.113.7:19132");
    }

    #[test]
    fn auto_resolves_like_the_jar_but_with_the_real_port() {
        let text = r"
session:
  remote-address: auto
  remote-port: auto
";
        let a = analyze_relay_config(text, 24474);
        assert_eq!(a.relay.unwrap().text, "127.0.0.1:24474");
    }

    #[test]
    fn query_server_off_is_reported() {
        let text = "session:\n  query-server: false\n";
        assert!(!analyze_relay_config(text, 19132).query_server);
        assert!(analyze_relay_config("session: {}", 19132).query_server);
    }

    #[test]
    fn broken_config_degrades_to_unknown() {
        let a = analyze_relay_config("\t:not yaml: [", 19132);
        assert_eq!(a, RelayAnalysis::unknown());
        assert!(a.relay.is_none());
    }

    #[test]
    fn fresh_template_carries_loopback_relay_target() {
        let text = DEFAULT_CONFIG.replace("{{PORT}}", "24474");
        let a = analyze_relay_config(&text, 24474);
        assert_eq!(
            a.relay,
            Some(RelayTarget {
                text: "127.0.0.1:24474".into(),
                public: false
            })
        );
        assert!(
            !text.contains("{{PORT}}"),
            "placeholder must be substituted"
        );
        // Post-v7 layouts have no session-info address to trip over (Bug 1).
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert!(doc["session"]["session-info"].get("ip").is_none());
        assert_eq!(doc["config-version"].as_i64(), Some(7));
    }
}
