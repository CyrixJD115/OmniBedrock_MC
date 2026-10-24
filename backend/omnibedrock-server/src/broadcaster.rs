//! MCXboxBroadcast Standalone integration.
//!
//! The broadcaster runs as its own Java process in a dedicated directory that
//! holds `config.yml`, `screenshot.jpg`, and the `cache/` token store. Output
//! streams through a Console ring like the game server; device-code prompts
//! emitted during Microsoft authentication are parsed out of the log stream so
//! the UI can surface the code and verification URL.
use crate::server::{Console, read_pipe};
use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;

pub const JAR_NAME: &str = "MCXboxBroadcastStandalone.jar";
const STOP_TIMEOUT: Duration = Duration::from_secs(15);

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

type Reply = oneshot::Sender<anyhow::Result<String>>;
enum Control {
    Start(Reply),
    Stop(Reply),
    Restart(Reply),
    Command(String, oneshot::Sender<anyhow::Result<()>>),
    Shutdown(Reply),
}
struct Running {
    child: Child,
    pid: u32,
    stdin: mpsc::Sender<String>,
    writer: tokio::task::JoinHandle<()>,
    readers: Vec<tokio::task::JoinHandle<()>>,
}

#[derive(Clone)]
pub struct Broadcaster {
    control: mpsc::Sender<Control>,
    pub status: watch::Receiver<BcSnapshot>,
    pub console: Console,
    pub device_codes: watch::Receiver<BTreeMap<String, DeviceCode>>,
    /// `java -version` first line, None until detection finishes or if absent.
    pub java: watch::Receiver<Option<String>>,
    config: Arc<Config>,
}

impl Broadcaster {
    pub fn new(config: Arc<Config>, shutdown: CancellationToken) -> anyhow::Result<Self> {
        let console = Console::with_journal_root(
            config.logs_dir.join("broadcaster-console"),
            config.console_max_lines,
            config.console_max_bytes,
            (config.console_journal_bytes / 4).max(4 * 1024 * 1024),
        );
        let (status_tx, status) = watch::channel(BcSnapshot::default());
        let (codes_tx, device_codes) = watch::channel(BTreeMap::new());
        let (java_tx, java) = watch::channel(None);
        let (control, control_rx) = mpsc::channel(16);
        let manager = Self {
            control,
            status,
            console: console.clone(),
            device_codes,
            java: java.clone(),
            config: config.clone(),
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
        tokio::spawn(supervise(config, control_rx, status_tx, console, shutdown));
        Ok(manager)
    }

    pub fn dir(&self) -> PathBuf {
        self.config.broadcaster_dir.clone()
    }
    pub fn jar_path(&self) -> PathBuf {
        self.config
            .broadcaster_jar
            .clone()
            .unwrap_or_else(|| self.config.broadcaster_dir.join(JAR_NAME))
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
    pub async fn action(&self, action: &str) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        let control = match action {
            "start" => Control::Start(tx),
            "stop" => Control::Stop(tx),
            "restart" => Control::Restart(tx),
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
    pub async fn read_config(&self) -> anyhow::Result<String> {
        let path = self.config_path();
        tokio::fs::create_dir_all(self.dir()).await?;
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                atomic_write(&path, DEFAULT_CONFIG.as_bytes()).await?;
                Ok(DEFAULT_CONFIG.into())
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

fn java_command(config: &Config) -> Command {
    let mut cmd = Command::new(
        config
            .java_bin
            .as_deref()
            .unwrap_or(std::path::Path::new("java")),
    );
    cmd.arg("-jar")
        .arg(
            config
                .broadcaster_jar
                .clone()
                .unwrap_or_else(|| config.broadcaster_dir.join(JAR_NAME)),
        )
        .current_dir(&config.broadcaster_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

async fn launch(config: &Config, console: &Console) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.broadcaster_dir).await?;
    let jar = config
        .broadcaster_jar
        .clone()
        .unwrap_or_else(|| config.broadcaster_dir.join(JAR_NAME));
    if !jar.exists() {
        bail!(
            "broadcaster jar not found at {}; place {} there or set BROADCASTER_JAR",
            jar.display(),
            JAR_NAME
        );
    }
    let mut child = java_command(config)
        .spawn()
        .context("java not found; install Java or set JAVA_BIN")?;
    let pid = child.id().context("child has no PID")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
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
    tracing::info!(pid, "broadcaster started");
    Ok(Running {
        child,
        pid,
        stdin: tx,
        writer,
        readers,
    })
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
    let _ = running.stdin.send("stop".into()).await;
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
    signal_tree(running.pid, true);
    running.writer.abort();
    let _ = running.writer.await;
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
    mut control: mpsc::Receiver<Control>,
    status: watch::Sender<BcSnapshot>,
    console: Console,
    shutdown: CancellationToken,
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
                    status.send_modify(|s| {
                        s.status = BcStatus::Stopped;
                        s.pid = None;
                        s.started_at = None;
                    });
                }
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let _ = terminate(&mut r).await;
                    drain(r).await;
                    status.send_modify(|s| {
                        s.status = BcStatus::Stopped;
                        s.pid = None;
                        s.started_at = None;
                    });
                }
                let _ = reply.send(Ok("Broadcaster stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Start(reply))) => {
                let result = if running.is_some() {
                    Ok("Broadcaster is already running".into())
                } else {
                    match launch(&config, &console).await {
                        Ok(r) => {
                            status.send_modify(|s| {
                                s.status = BcStatus::Running;
                                s.pid = Some(r.pid);
                                s.started_at = Some(Instant::now());
                            });
                            let pid = r.pid;
                            running = Some(r);
                            Ok(format!("Broadcaster started (PID: {pid})"))
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Stop(reply))) => {
                let result = if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let result = terminate(&mut r).await;
                    drain(r).await;
                    status.send_modify(|s| {
                        s.status = BcStatus::Stopped;
                        s.pid = None;
                        s.started_at = None;
                    });
                    result.map(|_| "Broadcaster stopped".into())
                } else {
                    Ok("Broadcaster is not running".into())
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Restart(reply))) => {
                let result = async {
                    if let Some(mut r) = running.take() {
                        status.send_modify(|s| s.status = BcStatus::Stopping);
                        let stopped = terminate(&mut r).await;
                        drain(r).await;
                        status.send_modify(|s| {
                            s.status = BcStatus::Stopped;
                            s.pid = None;
                            s.started_at = None;
                        });
                        stopped?;
                    }
                    match launch(&config, &console).await {
                        Ok(r) => {
                            status.send_modify(|s| {
                                s.status = BcStatus::Running;
                                s.pid = Some(r.pid);
                                s.started_at = Some(Instant::now());
                            });
                            let pid = r.pid;
                            running = Some(r);
                            Ok(format!("Broadcaster restarted (PID: {pid})"))
                        }
                        Err(e) => Err(e),
                    }
                }
                .await;
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Command(text, reply))) => {
                let result = match running.as_ref() {
                    Some(r) => r
                        .stdin
                        .send(text)
                        .await
                        .map(|_| ())
                        .map_err(|e| anyhow::anyhow!("stdin closed: {e}")),
                    None => Err(anyhow::anyhow!("Broadcaster is not running")),
                };
                let _ = reply.send(result);
            }
            Wake::Exit(exit) => {
                if let Some(r) = running.take() {
                    drain(r).await;
                }
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
