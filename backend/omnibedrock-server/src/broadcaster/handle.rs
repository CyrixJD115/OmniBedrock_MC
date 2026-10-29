//! The per-instance `Broadcaster` handle: the cloneable facade other modules
//! hold (console, status/device-code/ping watches, config and jar path
//! helpers, control actions) that talks to the supervisor in lifecycle.rs,
//! plus the console watcher that parses Microsoft device-code prompts.

use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::config::analyze_relay_config;
use super::consts::DEFAULT_CONFIG;
use super::external::find_external;
use super::health::{ping_poller, watch_ping_failures};
use super::helpers::{instance_port, resolve_jar};
use super::lifecycle::supervise;
use super::process::detect_java;
use super::types::{
    BcSnapshot, BcStatus, Control, DeviceCode, DockerSpec, ExternalBroadcaster, PingHealth,
    RelayAnalysis,
};
use crate::server::Console;

static DEVICE_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)open the page (\S+?) and enter the code ([A-Za-z0-9-]+)").unwrap()
});
static SUB_SESSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[Sub-Session ([^\]]+)\]").unwrap());

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
