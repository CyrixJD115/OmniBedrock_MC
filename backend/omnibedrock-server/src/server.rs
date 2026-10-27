//! The supervisor exclusively owns the child. Readers only mutate bounded console state.
//! No process wait or pipe operation blocks a Tokio worker thread.
use crate::docker;
use crate::instances::{Deployment, DockerConfig, ResourcePolicy};
use crate::journal::Journal;
use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    process::Stdio,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

static LEVEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[(ERROR|WARN(?:ING)?|INFO|DEBUG|LOCAL)\]").unwrap());
static ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)error|exception|traceback|fatal").unwrap());
static TPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(tps|mspt)[:\s]*([0-9]+(?:\.[0-9]+)?)").unwrap());
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)+").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+\b").unwrap());

pub fn now() -> f64 {
    chrono::Utc::now().timestamp_micros() as f64 / 1_000_000.0
}
pub fn detect_level(line: &str) -> &'static str {
    if let Some(c) = LEVEL.captures(line) {
        match c[1].to_ascii_lowercase().as_str() {
            "error" => "error",
            "warn" | "warning" => "warn",
            "debug" => "debug",
            "local" => "local",
            _ => "info",
        }
    } else if ERROR.is_match(line) {
        "error"
    } else {
        "info"
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub status: Status,
    pub pid: Option<u32>,
    pub uptime: f64,
    pub version: Option<String>,
    pub tps: Option<f64>,
    pub crash_count: u32,
    pub auto_restart: bool,
    pub backend: Backend,
    /// How this instance is launched; determines limit enforcement.
    pub deployment: Deployment,
    /// Configured limits, enforced only when deployment is systemd/docker.
    pub limits: ResourcePolicy,
    #[serde(skip)]
    pub started_at: Option<Instant>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Endstone,
    Vanilla,
}
impl Backend {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "endstone" => Ok(Self::Endstone),
            "vanilla" | "bds" => Ok(Self::Vanilla),
            other => bail!("unknown backend '{other}' (expected endstone or vanilla)"),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Endstone => "endstone",
            Self::Vanilla => "vanilla",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSettings {
    pub auto_restart: bool,
    pub auto_restart_delay: u64,
    pub max_crashes: u32,
    pub stop_timeout: u64,
    pub kill_timeout: u64,
    pub backend: Backend,
    /// Vanilla BDS registry version, or "latest" to track the newest release.
    pub vanilla_version: String,
}
impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            auto_restart: true,
            auto_restart_delay: 5,
            max_crashes: 3,
            stop_timeout: 8,
            kill_timeout: 4,
            backend: Backend::Endstone,
            vanilla_version: "latest".into(),
        }
    }
}
impl ServerSettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.auto_restart_delay == 0
            || self.stop_timeout == 0
            || self.kill_timeout == 0
            || self.stop_timeout > 600
            || self.kill_timeout > 600
            || self.auto_restart_delay > 3600
        {
            bail!("invalid grace period or restart delay");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsoleEvent {
    pub r#type: String,
    pub sequence: u64,
    pub stream_id: Uuid,
    pub line: String,
    pub level: String,
    pub timestamp: f64,
    pub source: String,
    pub continued: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ErrorStat {
    pub signature: String,
    pub count: u64,
    pub first_seen: f64,
    pub last_seen: f64,
}
struct ConsoleState {
    lines: VecDeque<Arc<ConsoleEvent>>,
    bytes: usize,
    sequence: u64,
    errors: BTreeMap<String, ErrorStat>,
    tps: Option<f64>,
}
#[derive(Clone)]
pub struct Console {
    inner: Arc<Mutex<ConsoleState>>,
    pub stream_id: Uuid,
    pub changed: watch::Sender<u64>,
    pub errors_changed: watch::Sender<u64>,
    max_lines: usize,
    max_bytes: usize,
    journal: Arc<tokio::sync::Mutex<Journal>>,
}
pub struct Replay {
    pub lines: Vec<Arc<ConsoleEvent>>,
    pub gap: Option<(u64, u64)>,
    pub latest: u64,
}
impl Console {
    pub fn new(config: &Config) -> Self {
        Self::with_journal_root(
            config.logs_dir.join("console"),
            config.console_max_lines,
            config.console_max_bytes,
            config.console_journal_bytes,
        )
    }
    /// Console ring for any managed process; the journal root names its spool directory.
    pub fn with_journal_root(
        journal_root: std::path::PathBuf,
        max_lines: usize,
        max_bytes: usize,
        journal_bytes: u64,
    ) -> Self {
        let stream_id = Uuid::new_v4();
        Self {
            journal: Arc::new(tokio::sync::Mutex::new(Journal::new(
                journal_root,
                stream_id,
                journal_bytes,
            ))),
            inner: Arc::new(Mutex::new(ConsoleState {
                lines: VecDeque::new(),
                bytes: 0,
                sequence: 0,
                errors: BTreeMap::new(),
                tps: None,
            })),
            stream_id,
            changed: watch::channel(0).0,
            errors_changed: watch::channel(0).0,
            max_lines,
            max_bytes,
        }
    }
    pub async fn publish(&self, line: String, source: &'static str, continued: bool) {
        let mut journal = self.journal.lock().await;
        let (event, sequence, level) = self.insert(line, source, continued);
        if let Err(e) = journal.append(&event).await {
            tracing::error!(%e, "console disk replay unavailable; memory replay remains active");
        }
        self.changed.send_replace(sequence);
        if level == "error" {
            self.errors_changed.send_replace(sequence);
        }
    }
    fn insert(
        &self,
        line: String,
        source: &'static str,
        continued: bool,
    ) -> (Arc<ConsoleEvent>, u64, &'static str) {
        let level = detect_level(&line);
        let timestamp = now();
        let mut s = self.inner.lock().expect("console mutex poisoned");
        if let Some(c) = TPS.captures(&line)
            && let Ok(value) = c[2].parse::<f64>()
        {
            // MSPT is milliseconds per tick, not TPS.
            s.tps = Some(if c[1].eq_ignore_ascii_case("mspt") {
                if value > 0.0 {
                    (1000.0 / value).min(20.0)
                } else {
                    20.0
                }
            } else {
                value
            });
        }
        if level == "error" {
            let lower = line.to_lowercase();
            let sig = DECIMAL.replace_all(&lower, "@");
            let sig = NUMBER.replace_all(&sig, "#");
            let signature: String = sig
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect();
            // Bound unique signatures as well as console lines.
            if s.errors.len() >= 1024
                && !s.errors.contains_key(&signature)
                && let Some(key) = s
                    .errors
                    .iter()
                    .min_by(|a, b| a.1.last_seen.total_cmp(&b.1.last_seen))
                    .map(|(k, _)| k.clone())
            {
                s.errors.remove(&key);
            }
            let entry = s.errors.entry(signature.clone()).or_insert(ErrorStat {
                signature,
                count: 0,
                first_seen: timestamp,
                last_seen: timestamp,
            });
            entry.count += 1;
            entry.last_seen = timestamp;
        }
        s.sequence += 1;
        let sequence = s.sequence;
        s.bytes += line.len();
        let event = Arc::new(ConsoleEvent {
            r#type: "console".into(),
            sequence,
            stream_id: self.stream_id,
            line,
            level: level.into(),
            timestamp,
            source: source.into(),
            continued,
        });
        s.lines.push_back(event.clone());
        while s.lines.len() > self.max_lines || s.bytes > self.max_bytes {
            if let Some(line) = s.lines.pop_front() {
                s.bytes -= line.line.len();
            }
        }
        (event, sequence, level)
    }
    pub fn replay(&self, after: Option<u64>, limit: usize) -> Replay {
        let s = self.inner.lock().expect("console mutex poisoned");
        let first = s.lines.front().map_or(s.sequence + 1, |l| l.sequence);
        let gap = after
            .filter(|n| n.saturating_add(1) < first)
            .map(|n| (n + 1, first - 1));
        Replay {
            lines: s
                .lines
                .iter()
                .filter(|l| after.is_none_or(|n| l.sequence > n))
                .take(limit)
                .cloned()
                .collect(),
            gap,
            latest: s.sequence,
        }
    }
    pub async fn replay_reliable(
        &self,
        after: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Replay> {
        let batch = self.replay(after, limit);
        if batch.gap.is_some()
            && let Some(after) = after
        {
            match self.journal.lock().await.replay(after, limit).await {
                Ok(Some(replay)) => return Ok(replay),
                Ok(None) => {}
                Err(e) => tracing::error!(%e, "disk replay failed; reporting memory retention gap"),
            }
        }
        Ok(batch)
    }
    /// The last `limit` events from the in-memory ring, oldest first. The
    /// REST counterpart of `replay` for one-shot readers (scripts, health
    /// checks); deeper history stays on the WebSocket + disk journal.
    pub fn tail(&self, limit: usize) -> Vec<Arc<ConsoleEvent>> {
        let s = self.inner.lock().expect("console mutex poisoned");
        let skip = s.lines.len().saturating_sub(limit);
        s.lines.iter().skip(skip).cloned().collect()
    }
    pub async fn flush(&self) -> anyhow::Result<()> {
        self.journal.lock().await.flush().await
    }
    pub fn errors(&self) -> Vec<ErrorStat> {
        let mut errors: Vec<_> = self
            .inner
            .lock()
            .expect("console mutex poisoned")
            .errors
            .values()
            .cloned()
            .collect();
        errors.sort_by_key(|s| std::cmp::Reverse(s.count));
        errors
    }
    pub fn tps(&self) -> Option<f64> {
        self.inner.lock().expect("console mutex poisoned").tps
    }
    pub fn reset_stats(&self) {
        let mut s = self.inner.lock().expect("console mutex poisoned");
        s.errors.clear();
        s.tps = None;
        self.errors_changed.send_replace(s.sequence);
    }
}

/// Splits oversized lines into bounded chunks without discarding their bytes. Invalid
/// UTF-8 is replaced for the JSON wire protocol. EOF flushes the unterminated last line.
pub(crate) async fn read_pipe<R: AsyncRead + Unpin>(
    pipe: R,
    console: Console,
    source: &'static str,
    max: usize,
) {
    let mut reader = BufReader::with_capacity(32 * 1024, pipe);
    let mut pending = Vec::with_capacity(4096);
    loop {
        let buf = match reader.fill_buf().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(%e, source, "pipe read failed");
                break;
            }
        };
        if buf.is_empty() {
            break;
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let take = newline
            .map_or(buf.len(), |i| i + 1)
            .min(max - pending.len());
        pending.extend_from_slice(&buf[..take]);
        reader.consume(take);
        let complete = pending.last() == Some(&b'\n');
        if complete || pending.len() == max {
            if complete {
                pending.pop();
                if pending.last() == Some(&b'\r') {
                    pending.pop();
                }
            }
            console
                .publish(
                    String::from_utf8_lossy(&pending).into_owned(),
                    source,
                    !complete,
                )
                .await;
            pending.clear();
        }
    }
    if !pending.is_empty() {
        console
            .publish(
                String::from_utf8_lossy(&pending).into_owned(),
                source,
                false,
            )
            .await;
    }
}

type Reply = oneshot::Sender<anyhow::Result<String>>;
enum Control {
    Action(String, Reply),
    Settings(ServerSettings, Reply),
    /// Adopt a systemd unit that is already active (panel restart).
    Adopt(Reply),
    Shutdown(Reply),
}
struct WriteCommand {
    text: String,
    reply: oneshot::Sender<anyhow::Result<()>>,
}

/// How console commands reach a running server: an owned child's stdin pipe
/// or the FIFO of a systemd unit service.
#[derive(Clone)]
enum CommandSink {
    Pipe(mpsc::Sender<WriteCommand>),
    Fifo(std::path::PathBuf),
}

async fn dispatch_command(sink: &CommandSink, text: &str) -> anyhow::Result<()> {
    match sink {
        CommandSink::Pipe(tx) => write_command(tx, text).await,
        CommandSink::Fifo(path) => tokio::time::timeout(Duration::from_secs(5), async {
            use tokio::io::AsyncWriteExt;
            // Open read+write: a write-only open blocks until a reader
            // appears, and the panel must never block on the server.
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
        .context("FIFO command timed out")?,
    }
}

/// One running instance, however it is hosted: a child process the panel
/// owns (native, a `docker attach` client, or a systemd user service).
struct Running {
    pid: u32,
    sink: CommandSink,
    kind: RunKind,
}

enum RunKind {
    /// A process the panel supervises directly, with piped stdio. For docker
    /// instances this process is the `docker logs -f` reader: no stdin child
    /// exists (commands go through the FIFO) and its exit means the container
    /// stopped streaming, not that the server is gone.
    Process {
        child: Child,
        stdin: Option<mpsc::Sender<WriteCommand>>,
        writer: Option<JoinHandle<()>>,
        readers: Vec<JoinHandle<()>>,
        /// Docker container name when the process reads a container's logs.
        container: Option<String>,
        /// Reports the moment the container stops running, independent of
        /// the logs reader's own lifetime.
        watch: Option<oneshot::Receiver<String>>,
    },
    /// A `omnibedrock-<id>.service` user unit owned by systemd. The panel
    /// attaches its journal and monitors its state but never parents the
    /// process, so the server survives panel restarts.
    Unit {
        id: String,
        exit: oneshot::Receiver<String>,
        monitor: JoinHandle<()>,
        readers: Vec<JoinHandle<()>>,
    },
}

/// Per-instance filesystem layout for the supervisor. The default instance
/// keeps the legacy global paths; other instances live under
/// `<data_dir>/instances/<id>/`.
#[derive(Clone, Debug)]
pub struct ServerPaths {
    pub settings_path: std::path::PathBuf,
    pub state_dir: std::path::PathBuf,
    pub journal_root: std::path::PathBuf,
}

/// How one instance is launched: its identity, deployment backend, limits,
/// port, and Docker options. Swappable while stopped via `set_launch`.
#[derive(Clone, Debug)]
pub struct LaunchContext {
    pub instance_id: String,
    /// Display name (unit description).
    pub instance_name: String,
    pub deployment: Deployment,
    pub resources: ResourcePolicy,
    pub port: u16,
    /// systemd only: keep the unit enabled for boot-time autostart.
    pub autostart: bool,
    /// Docker options (image override + extra mounts); `default()` for
    /// native/systemd instances.
    pub docker: DockerConfig,
}

impl Default for LaunchContext {
    fn default() -> Self {
        Self {
            instance_id: "default".into(),
            instance_name: "Default Server".into(),
            deployment: Deployment::Native,
            resources: ResourcePolicy::default(),
            port: 19132,
            autostart: false,
            docker: DockerConfig::default(),
        }
    }
}

#[derive(Clone)]
pub struct ServerManager {
    control: mpsc::Sender<Control>,
    commands: watch::Receiver<Option<CommandSink>>,
    pub state: watch::Receiver<Snapshot>,
    pub settings: watch::Receiver<ServerSettings>,
    launch: watch::Receiver<LaunchContext>,
    launch_tx: watch::Sender<LaunchContext>,
    pub console: Console,
    shutdown: CancellationToken,
}
impl ServerManager {
    /// Legacy single-server layout: everything derived from the global config.
    pub async fn new(config: Arc<Config>, shutdown: CancellationToken) -> anyhow::Result<Self> {
        Self::with_paths(
            config.clone(),
            shutdown,
            ServerPaths {
                settings_path: config.data_dir.join("server_settings.yaml"),
                state_dir: config.data_dir.clone(),
                journal_root: config.logs_dir.join("console"),
            },
            LaunchContext {
                docker: DockerConfig::default(),
                ..LaunchContext::default()
            },
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn with_paths(
        config: Arc<Config>,
        shutdown: CancellationToken,
        paths: ServerPaths,
        launch: LaunchContext,
    ) -> anyhow::Result<Self> {
        let settings: ServerSettings = match std::fs::read_to_string(&paths.settings_path) {
            Ok(s) => serde_yaml::from_str(&s).context("invalid server settings")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ServerSettings::default(),
            Err(e) => return Err(e.into()),
        };
        settings.validate()?;
        let snapshot = Snapshot {
            status: Status::Stopped,
            pid: None,
            uptime: 0.0,
            version: None,
            tps: None,
            crash_count: 0,
            auto_restart: settings.auto_restart,
            backend: settings.backend,
            deployment: launch.deployment,
            limits: launch.resources,
            started_at: None,
        };
        let (state_tx, state) = watch::channel(snapshot);
        let (settings_tx, settings_rx) = watch::channel(settings);
        let (commands_tx, commands) = watch::channel(None);
        let (launch_tx, launch_rx) = watch::channel(launch);
        let (control, control_rx) = mpsc::channel(32);
        let console = Console::with_journal_root(
            paths.journal_root.clone(),
            config.console_max_lines,
            config.console_max_bytes,
            config.console_journal_bytes,
        );
        let flush_console = console.clone();
        let flush_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! { _=flush_shutdown.cancelled()=>{let _=flush_console.flush().await; break;}, _=interval.tick()=>{if let Err(e)=flush_console.flush().await {tracing::error!(%e,"console journal flush failed");}} }
            }
        });
        let manager = Self {
            control,
            commands,
            state,
            settings: settings_rx,
            launch: launch_rx,
            launch_tx: launch_tx.clone(),
            console: console.clone(),
            shutdown: shutdown.clone(),
        };
        tokio::spawn(supervise(
            config,
            paths,
            control_rx,
            state_tx,
            settings_tx,
            launch_tx,
            commands_tx,
            console,
            shutdown,
        ));
        Ok(manager)
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.state.borrow().clone();
        s.backend = self.settings.borrow().backend;
        let launch = self.launch.borrow();
        s.deployment = launch.deployment;
        s.limits = launch.resources;
        s.uptime = s
            .started_at
            .filter(|_| s.status == Status::Running)
            .map_or(0.0, |i| i.elapsed().as_secs_f64());
        s.tps = self.console.tps();
        s
    }
    /// Swap the launch context (deployment/resources/port). Applies on the
    /// next start; callers reject changes while the instance runs.
    pub fn set_launch(&self, launch: LaunchContext) {
        self.launch_tx.send_replace(launch);
    }
    pub async fn action(&self, action: &str) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Action(action.into(), tx))
            .await
            .context("supervisor stopped")?;
        rx.await.context("supervisor stopped")?
    }
    pub async fn configure(&self, settings: ServerSettings) -> anyhow::Result<String> {
        settings.validate()?;
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Settings(settings, tx))
            .await
            .context("supervisor stopped")?;
        rx.await.context("supervisor stopped")?
    }
    pub async fn send_command(&self, command: &str) -> anyhow::Result<()> {
        if command.trim().is_empty() || command.len() > 8192 || command.contains(['\r', '\n', '\0'])
        {
            bail!("command must be a single nonempty line of at most 8192 bytes");
        }
        if self.snapshot().status != Status::Running {
            bail!("Server is not running");
        }
        let sink = self
            .commands
            .borrow()
            .clone()
            .context("Server is not running")?;
        dispatch_command(&sink, command).await
    }

    /// Adopt a systemd unit that is already active (for example after a
    /// panel restart). No-op for other deployments or inactive units.
    pub async fn adopt_if_active(&self) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Adopt(tx))
            .await
            .context("supervisor stopped")?;
        rx.await.context("supervisor stopped")?
    }
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.control
            .send(Control::Shutdown(tx))
            .await
            .context("supervisor stopped")?;
        let result = rx.await.context("supervisor stopped")?;
        self.shutdown.cancel();
        result.map(|_| ())
    }
}
async fn write_command(tx: &mpsc::Sender<WriteCommand>, command: &str) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (reply, rx) = oneshot::channel();
        tx.send(WriteCommand {
            text: command.into(),
            reply,
        })
        .await
        .context("stdin closed")?;
        rx.await.context("stdin closed")?
    })
    .await
    .context("stdin command timed out")?
}
fn base_argv_endstone(config: &Config) -> anyhow::Result<Vec<std::ffi::OsString>> {
    let Some(candidate) = crate::endstone::resolve_launch(config) else {
        anyhow::bail!(
            "Endstone is not installed — install the Endstone toolchain from \
             Settings → Server backend, or set ENDSTONE_BIN to an endstone executable"
        );
    };
    Ok(match candidate {
        crate::endstone::EndstoneLaunch::Binary(path) => {
            let mut argv = vec![path.into()];
            argv.push("--no-confirm".into());
            argv.push("-s".into());
            argv.push(config.bedrock_server_dir.clone().into());
            argv
        }
        crate::endstone::EndstoneLaunch::PythonModule(python) => {
            let mut argv: Vec<std::ffi::OsString> =
                vec![python.into(), "-m".into(), "endstone".into()];
            argv.push("--no-confirm".into());
            argv.push("-s".into());
            argv.push(config.bedrock_server_dir.clone().into());
            argv
        }
    })
}
fn vanilla_binary() -> &'static std::path::Path {
    if cfg!(windows) {
        std::path::Path::new("bedrock_server.exe")
    } else {
        std::path::Path::new("bedrock_server")
    }
}
fn base_argv_vanilla(config: &Config) -> anyhow::Result<Vec<std::ffi::OsString>> {
    let binary = config.bedrock_server_dir.join(vanilla_binary());
    if !binary.exists() {
        bail!(
            "vanilla bedrock_server binary not found at {}; install a version from Settings → Backend",
            binary.display()
        );
    }
    Ok(vec![binary.into()])
}

/// FIFO path for an instance's unit service commands.
fn unit_fifo(paths: &ServerPaths) -> std::path::PathBuf {
    paths.state_dir.join("stdin.fifo")
}

/// Spawn `journalctl --user -f` for one unit and pipe its output into the
/// console ring, so the panel's Console shows the service journal exactly
/// like a child's stdout.
fn spawn_journal_reader(
    unit: &str,
    console: &Console,
    max_line: usize,
) -> anyhow::Result<JoinHandle<()>> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new("journalctl")
        .args(["--user", "-u", unit, "-n", "200", "-o", "cat", "-f"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("attach journalctl for {unit}"))?;
    let stdout = child.stdout.take().context("journalctl stdout")?;
    Ok(tokio::spawn(read_pipe(
        stdout,
        console.clone(),
        "journal",
        max_line,
    )))
}

/// Watch a unit's state until it stops being active, then report how it
/// ended through the exit channel. systemd itself does not restart the unit
/// (`Restart=no`); the panel's supervisor owns restart policy uniformly.
fn spawn_unit_monitor(id: String, exit: oneshot::Sender<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut errors = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(750)).await;
            match crate::systemd::active_state(&id).await {
                Some(state) if ["active", "activating", "reloading"].contains(&state.as_str()) => {
                    errors = 0;
                }
                Some(state) => {
                    let status = crate::systemd::exit_status(&id)
                        .await
                        .map(|s| format!("exit status {s}"))
                        .unwrap_or_else(|| "no exit status reported".into());
                    let _ = exit.send(format!("unit {state} ({status})"));
                    return;
                }
                None => {
                    // systemctl failing repeatedly means the user manager is
                    // gone; the service is gone with it.
                    errors += 1;
                    if errors >= 5 {
                        let _ = exit.send("user manager unreachable".into());
                        return;
                    }
                }
            }
        }
    })
}

/// Attach console + monitoring to an already-active unit without (re)starting
/// it. Used right after `systemctl start` and to adopt units that kept
/// running across a panel restart. Refuses anything that is not actually
/// `active` — `systemctl show` happily reports `inactive` for units that do
/// not exist, which must never look like adoption success.
async fn attach_unit(
    id: &str,
    fifo: &std::path::Path,
    console: &Console,
    max_line: usize,
) -> anyhow::Result<Running> {
    let unit = crate::systemd::unit_name(id);
    if crate::systemd::active_state(id).await.as_deref() != Some("active") {
        bail!("unit {unit} is not active");
    }
    let pid = crate::systemd::main_pid(id)
        .await
        .context("unit is active but has no MainPID yet")?;
    let (exit_tx, exit_rx) = oneshot::channel();
    let monitor = spawn_unit_monitor(id.to_string(), exit_tx);
    let reader = spawn_journal_reader(&unit, console, max_line)?;
    Ok(Running {
        pid,
        sink: CommandSink::Fifo(fifo.to_path_buf()),
        kind: RunKind::Unit {
            id: id.to_string(),
            exit: exit_rx,
            monitor,
            readers: vec![reader],
        },
    })
}

/// Full `docker run` argv for an instance: detached, `-i` so the panel's
/// attach client can forward console commands to the server's stdin, and the
/// restart policy that makes Docker the instance's supervisor across crashes
/// and host reboots. The instance's server directory is mounted read-write at
/// /data (the SAME directory the panel uses); the image only provides the
/// runtime (see deploy/docker).
fn docker_run_args(
    config: &Config,
    settings: &ServerSettings,
    ctx: &LaunchContext,
    host_network: bool,
) -> Vec<std::ffi::OsString> {
    let name = crate::instances::container_name(&ctx.instance_id);
    let mut args: Vec<std::ffi::OsString> = vec![
        "docker".into(),
        "run".into(),
        "-d".into(),
        "--restart".into(),
        // The panel's auto_restart setting maps onto the daemon policy: off
        // means the container stays down after any exit.
        if settings.auto_restart {
            "unless-stopped"
        } else {
            "no"
        }
        .into(),
        "--name".into(),
        name.into(),
    ];
    if let Some(mb) = ctx.resources.memory_mb {
        args.push("--memory".into());
        args.push(format!("{mb}m").into());
        // Equal swap limit: the container cannot exceed its memory budget by swapping.
        args.push("--memory-swap".into());
        args.push(format!("{mb}m").into());
    }
    if let Some(cpus) = ctx.resources.cpus {
        args.push("--cpus".into());
        args.push(format!("{cpus}").into());
    }
    // Run as the panel's own uid/gid so server files written into the /data
    // bind mount stay owned by (and deletable by) the panel user. Bedrock's
    // ports are unprivileged, so no container needs root.
    #[cfg(unix)]
    {
        let uid = nix::unistd::getuid();
        let gid = nix::unistd::getgid();
        args.push("--user".into());
        args.push(format!("{}:{}", uid.as_raw(), gid.as_raw()).into());
    }
    if host_network {
        // Linux: the container shares the host network stack, so game ports
        // and ICE ranges behave exactly like a native process and the server
        // binds the instance's own registered port pair.
        args.push("--network".into());
        args.push("host".into());
    } else {
        // Docker Desktop (Windows/macOS): host networking is unavailable, so
        // the container keeps the stock port pair and it is mapped through.
        args.push("-p".into());
        args.push(format!("{}:19132/udp", ctx.port).into());
        args.push("-p".into());
        args.push(format!("{}:19133/udp", ctx.port + 1).into());
    }
    args.push("-v".into());
    args.push(
        format!(
            "{}:/data",
            config
                .bedrock_server_dir
                .to_string_lossy()
                .replace('\\', "/")
        )
        .into(),
    );
    for mount in &ctx.docker.extra_mounts {
        args.push("-v".into());
        args.push(mount.clone().into());
    }
    args.push("-w".into());
    args.push("/data".into());
    args.push(ctx.docker.image_for(settings.backend, config).into());
    // Container paths: the server dir is /data, and stdin comes from the
    // command FIFO the panel also writes from the host.
    let server_argv: Vec<std::ffi::OsString> = match settings.backend {
        Backend::Vanilla => vec!["./bedrock_server".into()],
        Backend::Endstone => vec![
            "python".into(),
            "-m".into(),
            "endstone".into(),
            "--no-confirm".into(),
            "-s".into(),
            "/data".into(),
        ],
    };
    args.extend(docker::command_override(&server_argv));
    args
}

/// Watch one running container via `docker inspect` and report the moment it
/// stops running. The attach client's exit alone is NOT a reliable container
/// exit signal: the daemon can restart the container under its restart policy,
/// which detaches clients while the container keeps going.
fn spawn_docker_monitor(container: String, exit: oneshot::Sender<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut errors = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(750)).await;
            match docker::state_and_exit(&container).await {
                Some((status, _code)) if status == "running" => {
                    errors = 0;
                }
                Some((status, code)) => {
                    let _ = exit.send(match code {
                        Some(c) => format!("container {status} (exit code {c})"),
                        None => format!("container {status}"),
                    });
                    return;
                }
                None => {
                    errors += 1;
                    if errors >= 5 {
                        let _ = exit.send("docker unreachable".into());
                        return;
                    }
                }
            }
        }
    })
}

/// Attach console + command plumbing to an already-running container (fresh
/// start, revive recheck, or adoption after a panel restart): spawn the
/// `docker logs -f` reader, the container state monitor, and point the
/// command sink at the FIFO inside the bind-mounted server directory. The
/// container itself must already be running.
async fn attach_docker(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    settings: &ServerSettings,
    container: &str,
) -> anyhow::Result<Running> {
    let fifo = docker::host_stdin_fifo(&config.bedrock_server_dir);
    crate::systemd::ensure_fifo(&fifo).await?;
    let mut running = spawn_process(
        config,
        console,
        ctx,
        settings.backend,
        docker::logs_argv(container, 200),
        Some(container.to_string()),
        false,
        Some(CommandSink::Fifo(fifo)),
    )
    .await?;
    let (exit_tx, exit_rx) = oneshot::channel();
    spawn_docker_monitor(container.to_string(), exit_tx);
    if let RunKind::Process { watch, .. } = &mut running.kind {
        *watch = Some(exit_rx);
    }
    Ok(running)
}

/// Attach to the instance's container when it is running; error otherwise.
/// Never starts or creates anything — revival is exclusively the daemon's
/// restart policy's job, so the panel cannot fight a manual `docker stop`.
async fn attach_running_container(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    settings: &ServerSettings,
) -> anyhow::Result<Running> {
    let container = crate::instances::container_name(&ctx.instance_id);
    match docker::state(&container).await.as_deref() {
        Some("running") => attach_docker(config, console, ctx, settings, &container).await,
        _ => anyhow::bail!("container {container} is not running"),
    }
}

/// Spawn one process the panel owns (a native server, or the `docker logs`
/// reader of a container) and wire its stdio into the console. `with_stdin`
/// is false for log readers; their sink comes from `sink_override`.
#[allow(clippy::too_many_arguments)]
async fn spawn_process(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    backend: Backend,
    argv: Vec<std::ffi::OsString>,
    container: Option<String>,
    with_stdin: bool,
    sink_override: Option<CommandSink>,
) -> anyhow::Result<Running> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if ctx.deployment != Deployment::Docker {
        // Native children inherit the server directory so vanilla BDS finds
        // its own files; containers run with -w inside the image.
        cmd.current_dir(&config.bedrock_server_dir);
    }
    configure_process(&mut cmd, with_stdin);
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "launch {} instance ({})",
            ctx.deployment.as_str(),
            argv[0].to_string_lossy()
        )
    })?;
    let pid = child.id().context("child has no PID")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let (tx, writer) = if with_stdin {
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let (tx, mut rx) = mpsc::channel::<WriteCommand>(config.command_queue_capacity);
        let writer = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                if cmd.reply.is_closed() {
                    continue;
                }
                let result = async {
                    stdin.write_all(cmd.text.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                let failed = result.is_err();
                let _ = cmd.reply.send(result);
                if failed {
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
    tracing::info!(
        pid,
        instance = %ctx.instance_id,
        backend = backend.as_str(),
        deployment = ctx.deployment.as_str(),
        container = ?container,
        "game server started"
    );
    let sink = match sink_override {
        Some(sink) => sink,
        None => CommandSink::Pipe(tx.clone().context("pipe sink without stdin child")?),
    };
    Ok(Running {
        pid,
        sink,
        kind: RunKind::Process {
            child,
            stdin: tx,
            writer,
            readers,
            container,
            watch: None,
        },
    })
}

async fn launch(
    config: &Config,
    paths: &ServerPaths,
    settings: &ServerSettings,
    console: &Console,
    ctx: &LaunchContext,
) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.bedrock_server_dir).await?;
    // All runtimes read server.properties from the server dir (Endstone
    // wraps BDS). Native/systemd and host-networked Linux containers pin the
    // instance's registered pair (P v4 / P+1 v6); port-mapped containers
    // (Docker Desktop) pin the container-internal stock pair 19132/19133 and
    // publish both through the host port mappings.
    match ctx.deployment {
        Deployment::Docker if !cfg!(target_os = "linux") => {
            crate::instances::enforce_instance_port(config, 19132).await;
        }
        _ => {
            crate::instances::enforce_instance_port(config, ctx.port).await;
        }
    }
    crate::instances::warn_transport_mismatch(config, console).await;
    // Heal pack-order versions for worlds imported before the sync existed
    // and surface still-missing pack uuids as one console line instead of
    // letting BDS log per-pack "was not found" noise.
    {
        let worlds = config.bedrock_server_dir.join("worlds");
        let sync = tokio::task::spawn_blocking(move || crate::addons::sync_all_worlds(&worlds))
            .await
            .unwrap_or_default();
        if !sync.is_empty() {
            console
                .publish(
                    format!("World pack sync: {}", sync.join("; ")),
                    "panel",
                    false,
                )
                .await;
        }
    }
    match ctx.deployment {
        Deployment::Native => {
            let base_argv = match settings.backend {
                Backend::Endstone => base_argv_endstone(config)?,
                Backend::Vanilla => base_argv_vanilla(config)?,
            };
            spawn_process(
                config,
                console,
                ctx,
                settings.backend,
                base_argv,
                None,
                true,
                None,
            )
            .await
        }
        Deployment::Docker => {
            // The image provides the runtime only; the BDS binary and every
            // server file come from the bind-mounted instance directory, so
            // the host needs no endstone toolchain for docker instances.
            let container = crate::instances::container_name(&ctx.instance_id);
            let host_network = cfg!(target_os = "linux");
            // The command FIFO must exist before the container's shell opens
            // it (the run command fails loudly otherwise).
            crate::systemd::ensure_fifo(&docker::host_stdin_fifo(&config.bedrock_server_dir))
                .await?;
            let image = ctx.docker.image_for(settings.backend, config);
            let dockerfile = match settings.backend {
                Backend::Vanilla => docker::DOCKERFILE_VANILLA,
                Backend::Endstone => docker::DOCKERFILE_ENDSTONE,
            };
            docker::ensure_image(config, &image, dockerfile, console).await?;
            match docker::state(&container).await.as_deref() {
                Some("running") => {
                    console
                        .publish(
                            format!(
                                "[LOCAL] Container {container} is already running; reattaching."
                            ),
                            "panel",
                            false,
                        )
                        .await;
                    attach_docker(config, console, ctx, settings, &container).await
                }
                _ => {
                    // A stopped container is recreated rather than
                    // `docker start`ed so a changed image or mount
                    // configuration takes effect on the next run.
                    docker::rm(&container).await;
                    let args = docker_run_args(config, settings, ctx, host_network);
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
                            "docker run failed: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        );
                    }
                    console
                        .publish(
                            format!(
                                "[LOCAL] Container {container} started (restart policy: {}).",
                                if settings.auto_restart {
                                    "unless-stopped"
                                } else {
                                    "no"
                                }
                            ),
                            "panel",
                            false,
                        )
                        .await;
                    attach_docker(config, console, ctx, settings, &container).await
                }
            }
        }
        Deployment::Systemd => {
            let base_argv = match settings.backend {
                Backend::Endstone => base_argv_endstone(config)?,
                Backend::Vanilla => base_argv_vanilla(config)?,
            };
            // Real user-service hosting: install the unit, let systemd parent
            // the process, then attach console and monitoring. The unit file
            // is regenerated every launch so moves/updates heal themselves.
            let exe = std::env::current_exe()
                .context("locate the panel executable for the unit ExecStart")?;
            let fifo = unit_fifo(paths);
            crate::systemd::ensure_fifo(&fifo).await?;
            let spec = crate::systemd::UnitSpec {
                instance_id: &ctx.instance_id,
                description: &ctx.instance_name,
                exe: &exe,
                fifo: &fifo,
                working_dir: &config.bedrock_server_dir,
                argv: &base_argv,
                resources: ctx.resources,
                stop_timeout: settings.stop_timeout,
                kill_timeout: settings.kill_timeout,
            };
            crate::systemd::install_unit(&ctx.instance_id, &crate::systemd::render_unit(&spec))
                .await?;
            crate::systemd::reset_failed(&ctx.instance_id).await;
            crate::systemd::start_unit(&ctx.instance_id).await?;
            let mut last_state = String::new();
            for _ in 0..100 {
                if let Some(state) = crate::systemd::active_state(&ctx.instance_id).await {
                    last_state = state;
                    if last_state == "active"
                        && let Ok(running) = attach_unit(
                            &ctx.instance_id,
                            &fifo,
                            console,
                            config.console_max_line_bytes,
                        )
                        .await
                    {
                        // The unit file exists now; sync its boot-time enable
                        // state to the instance record's autostart flag.
                        if let Err(e) =
                            crate::systemd::set_enabled(&ctx.instance_id, ctx.autostart).await
                        {
                            tracing::warn!(%e, "sync unit enable state failed");
                        }
                        tracing::info!(
                            instance = %ctx.instance_id,
                            backend = settings.backend.as_str(),
                            "game server started as user unit {}",
                            crate::systemd::unit_name(&ctx.instance_id)
                        );
                        return Ok(running);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            bail!(
                "systemd unit did not become active (state: {last_state}); \
                 check `journalctl --user -u {}`",
                crate::systemd::unit_name(&ctx.instance_id)
            )
        }
    }
}
fn configure_process(cmd: &mut Command, with_stdin: bool) {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PYTHONUNBUFFERED", "1");
    if with_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    #[cfg(unix)]
    cmd.process_group(0);
}
fn signal_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        if let Err(e) = killpg(
            Pid::from_raw(pid as i32),
            if force {
                Signal::SIGKILL
            } else {
                Signal::SIGTERM
            },
        ) && e != nix::errno::Errno::ESRCH
        {
            tracing::warn!(%e, "process group signal failed");
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}
/// Tear down the panel's own plumbing around a running instance WITHOUT
/// stopping a systemd service: journal/monitor tasks are detached so a unit
/// keeps running when the panel shuts down (that is the point of service
/// hosting). Process-based instances are cleaned up as before.
async fn drain(r: Running) {
    let pid = r.pid;
    match r.kind {
        RunKind::Process {
            child,
            writer,
            mut readers,
            ..
        } => {
            // A launcher can exit while its descendants retain the pipes.
            // Clean up its group. For a docker logs reader this only tears
            // down panel plumbing: the container keeps running (its restart
            // policy owns its lifetime).
            signal_tree(pid, true);
            drop(child);
            if let Some(writer) = writer {
                writer.abort();
                let _ = writer.await;
            }
            for mut reader in readers.drain(..) {
                if tokio::time::timeout(Duration::from_secs(30), &mut reader)
                    .await
                    .is_err()
                {
                    reader.abort();
                    let _ = reader.await;
                }
            }
        }
        RunKind::Unit {
            monitor,
            mut readers,
            ..
        } => {
            monitor.abort();
            let _ = monitor.await;
            for reader in readers.drain(..) {
                reader.abort();
                let _ = reader.await;
            }
        }
    }
}
/// Unified "how did it end" future for the supervisor's exit branch.
async fn wait_running(r: &mut Running) -> String {
    match &mut r.kind {
        RunKind::Process {
            child,
            container: Some(container),
            watch,
            ..
        } => {
            let container = container.clone();
            let reader = child.wait();
            let watch = watch.as_mut();
            tokio::select! {
                // The logs reader ended: either the container stopped (the
                // stream closes with it) or the daemon restarted the
                // container under its policy. Container state decides which.
                _ = reader => {
                    match docker::state_and_exit(&container).await {
                        Some((status, _)) if status == "running" => "reattach needed".to_string(),
                        Some((_, Some(0))) => "container exited (exit code 0)".to_string(),
                        Some((_, Some(code))) => format!("container exited (exit code {code})"),
                        _ => "container is gone".to_string(),
                    }
                }
                msg = async {
                    match watch {
                        Some(w) => w.await.unwrap_or_else(|_| "container watch ended".into()),
                        None => std::future::pending().await,
                    }
                } => msg,
            }
        }
        RunKind::Process { child, .. } => match child.wait().await {
            Ok(status) => format!("exit status {status}"),
            Err(e) => format!("wait failed: {e}"),
        },
        RunKind::Unit { exit, .. } => exit.await.unwrap_or_else(|_| "unit ended".into()),
    }
}
async fn terminate(r: &mut Running, settings: &ServerSettings, force: bool) -> anyhow::Result<()> {
    let stop_budget = Duration::from_secs(settings.stop_timeout);
    let kill_budget = Duration::from_secs(settings.kill_timeout);
    let pid = r.pid;
    let sink = r.sink.clone();
    match &mut r.kind {
        RunKind::Process {
            child,
            stdin,
            container,
            ..
        } => {
            // Docker stop ladder: graceful `stop` through the command FIFO
            // (identical to native), then ALWAYS `docker stop` — it stops a
            // still-running process AND records the manual stop that keeps
            // the restart policy from reviving the container after its own
            // exit (a bare process exit WOULD be revived). `docker kill` is
            // the force shortcut.
            if let Some(container) = container {
                if !force {
                    let _ = dispatch_command(&sink, "stop").await;
                    let _ = tokio::time::timeout(stop_budget, child.wait()).await;
                }
                if force {
                    docker::kill(container).await;
                } else if let Err(e) = docker::stop(container, settings.stop_timeout).await {
                    tracing::warn!(%e, container, "docker stop failed; escalating");
                    docker::kill(container).await;
                }
                // The logs reader exits once the container stops.
                if tokio::time::timeout(kill_budget, child.wait())
                    .await
                    .is_err()
                {
                    docker::kill(container).await;
                    let _ = tokio::time::timeout(kill_budget, child.wait()).await;
                }
                return Ok(());
            }
            if !force {
                let _ = write_command(
                    stdin
                        .as_ref()
                        .context("native process lost its stdin pipe")?,
                    "stop",
                )
                .await;
                if tokio::time::timeout(stop_budget, child.wait())
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                signal_tree(pid, false);
                #[cfg(not(unix))]
                child.start_kill()?;
                if tokio::time::timeout(kill_budget, child.wait())
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            signal_tree(pid, true);
            child.kill().await.context("kill child")?;
            Ok(())
        }
        RunKind::Unit { id, exit, .. } => {
            // Same ladder as owned processes: graceful `stop` command, then
            // systemctl stop (SIGTERM via the unit helper), then SIGKILL.
            if !force {
                let _ = dispatch_command(&sink, "stop").await;
                if tokio::time::timeout(stop_budget, &mut *exit).await.is_ok() {
                    return Ok(());
                }
            }
            let _ = crate::systemd::stop_unit(id).await;
            if tokio::time::timeout(kill_budget, &mut *exit).await.is_ok() {
                return Ok(());
            }
            let _ = crate::systemd::kill_unit(id).await;
            if tokio::time::timeout(kill_budget, &mut *exit).await.is_ok() {
                return Ok(());
            }
            bail!("systemd unit {id} did not stop")
        }
    }
}
async fn lock_state(state_dir: &std::path::Path, locked: bool) {
    let data = format!(
        "console:\n  state: {}\n  timestamp: {}\n",
        if locked { "locked" } else { "unlocked" },
        chrono::Utc::now().timestamp()
    );
    if let Err(e) = atomic_write(&state_dir.join("console_lock_state.yaml"), data.as_bytes()).await
    {
        tracing::error!(%e, "console lock persistence failed");
    }
}
#[allow(clippy::too_many_arguments)]
async fn supervise(
    config: Arc<Config>,
    paths: ServerPaths,
    mut controls: mpsc::Receiver<Control>,
    state: watch::Sender<Snapshot>,
    settings: watch::Sender<ServerSettings>,
    launch_tx: watch::Sender<LaunchContext>,
    commands: watch::Sender<Option<CommandSink>>,
    console: Console,
    shutdown: CancellationToken,
) {
    let mut running: Option<Running> = None;
    // Next automatic relaunch: the flag marks a docker RECHECK (attach only,
    // never create/start) vs a native relaunch.
    let mut restart_at: Option<(tokio::time::Instant, bool)> = None;
    lock_state(&paths.state_dir, false).await;
    loop {
        enum Wake {
            Control(Option<Control>),
            /// Human-readable description of how the instance ended.
            Exit(String),
            Restart,
            Shutdown,
        }
        let wake = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Wake::Shutdown,
            c = controls.recv() => Wake::Control(c),
            exit = async { match running.as_mut() { Some(r) => wait_running(r).await, None => std::future::pending().await } } => Wake::Exit(exit),
            _ = async { match restart_at { Some((at, _)) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Wake::Restart,
        };
        match wake {
            Wake::Shutdown | Wake::Control(None) => {
                commands.send_replace(None);
                let options = settings.borrow().clone();
                if let Some(mut r) = running.take() {
                    state.send_modify(|s| s.status = Status::Stopping);
                    if matches!(r.kind, RunKind::Unit { .. }) {
                        // A systemd service outlives the panel on purpose:
                        // detach and leave it running for the user manager.
                        tracing::info!(
                            "detaching unit instance at panel shutdown; service keeps running"
                        );
                    } else if matches!(
                        &r.kind,
                        RunKind::Process {
                            container: Some(_),
                            ..
                        }
                    ) {
                        // A docker container outlives the panel on purpose:
                        // detach and leave it to its restart policy.
                        tracing::info!(
                            "detaching container instance at panel shutdown; restart policy keeps it running"
                        );
                    } else if let Err(e) = terminate(&mut r, &options, false).await {
                        tracing::error!(%e, "shutdown stop failed");
                    }
                    drain(r).await;
                }
                lock_state(&paths.state_dir, false).await;
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                state.send_modify(|s| s.status = Status::Stopping);
                commands.send_replace(None);
                let options = settings.borrow().clone();
                let result = if let Some(mut r) = running.take() {
                    let result = if matches!(r.kind, RunKind::Unit { .. }) {
                        // Explicit supervisor shutdown detaches a systemd
                        // service without stopping it (panel restart).
                        tracing::info!("detaching unit instance; service keeps running");
                        Ok(())
                    } else if matches!(
                        &r.kind,
                        RunKind::Process {
                            container: Some(_),
                            ..
                        }
                    ) {
                        tracing::info!(
                            "detaching container instance; restart policy keeps it running"
                        );
                        Ok(())
                    } else {
                        terminate(&mut r, &options, false).await
                    };
                    drain(r).await;
                    result
                } else {
                    Ok(())
                };
                lock_state(&paths.state_dir, false).await;
                state.send_modify(|s| {
                    s.status = Status::Stopped;
                    s.pid = None;
                    s.started_at = None;
                });
                let _ = reply.send(result.map(|_| "Server stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Adopt(reply))) => {
                // After a panel restart, adopt a unit or container that is
                // still active instead of reporting the instance stopped.
                let launch_ctx = launch_tx.borrow().clone();
                let result = if running.is_some() {
                    Ok("Server is already running".into())
                } else if launch_ctx.deployment == Deployment::Docker {
                    state.send_modify(|s| s.status = Status::Starting);
                    let current = settings.borrow().clone();
                    match attach_running_container(&config, &console, &launch_ctx, &current).await {
                        Ok(r) => {
                            let pid = r.pid;
                            let container =
                                crate::instances::container_name(&launch_ctx.instance_id);
                            commands.send_replace(Some(r.sink.clone()));
                            state.send_modify(|s| {
                                s.status = Status::Running;
                                s.pid = Some(pid);
                                s.started_at = Some(Instant::now());
                                s.crash_count = 0;
                            });
                            running = Some(r);
                            lock_state(&paths.state_dir, true).await;
                            Ok(format!(
                                "Adopted running container {container} (PID: {pid})"
                            ))
                        }
                        Err(_) => {
                            state.send_modify(|s| s.status = Status::Stopped);
                            Ok("nothing to adopt".into())
                        }
                    }
                } else if launch_ctx.deployment != Deployment::Systemd {
                    Ok("nothing to adopt".into())
                } else {
                    state.send_modify(|s| s.status = Status::Starting);
                    let fifo = unit_fifo(&paths);
                    match attach_unit(
                        &launch_ctx.instance_id,
                        &fifo,
                        &console,
                        config.console_max_line_bytes,
                    )
                    .await
                    {
                        Ok(r) => {
                            let pid = r.pid;
                            commands.send_replace(Some(r.sink.clone()));
                            state.send_modify(|s| {
                                s.status = Status::Running;
                                s.pid = Some(pid);
                                s.started_at = Some(Instant::now());
                                s.crash_count = 0;
                            });
                            running = Some(r);
                            lock_state(&paths.state_dir, true).await;
                            Ok(format!("Adopted running unit (PID: {pid})"))
                        }
                        Err(e) => {
                            state.send_modify(|s| s.status = Status::Stopped);
                            Err(e)
                        }
                    }
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Settings(new, reply))) => {
                let result = async {
                    atomic_write(
                        &paths.settings_path,
                        serde_yaml::to_string(&new)?.as_bytes(),
                    )
                    .await?;
                    Ok::<_, anyhow::Error>("Server settings updated".into())
                }
                .await;
                if result.is_ok() {
                    if !new.auto_restart {
                        restart_at = None;
                        state.send_modify(|s| s.crash_count = 0);
                    }
                    state.send_modify(|s| {
                        s.auto_restart = new.auto_restart;
                        s.backend = new.backend;
                    });
                    settings.send_replace(new);
                }
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Action(action, reply))) => {
                if !["start", "stop", "kill", "restart"].contains(&action.as_str()) {
                    let _ = reply.send(Err(anyhow::anyhow!("Unknown action: {action}")));
                    continue;
                }
                restart_at = None;
                if action != "start" {
                    state.send_modify(|s| s.status = Status::Stopping);
                    commands.send_replace(None);
                    if let Some(mut r) = running.take() {
                        let options = settings.borrow().clone();
                        let result = terminate(&mut r, &options, action == "kill").await;
                        drain(r).await;
                        if let Err(e) = result {
                            tracing::warn!(%e, "stop required process cleanup");
                        }
                    }
                    state.send_modify(|s| {
                        s.status = Status::Stopped;
                        s.pid = None;
                        s.started_at = None;
                        s.crash_count = 0;
                    });
                    lock_state(&paths.state_dir, false).await;
                }
                let result = if action == "start" || action == "restart" {
                    if running.is_some() {
                        Ok("Server is already running".into())
                    } else {
                        state.send_modify(|s| {
                            s.status = Status::Starting;
                            s.crash_count = 0;
                        });
                        console.reset_stats();
                        let current = settings.borrow().clone();
                        let launch_ctx = launch_tx.borrow().clone();
                        match launch(&config, &paths, &current, &console, &launch_ctx).await {
                            Ok(r) => {
                                let pid = r.pid;
                                commands.send_replace(Some(r.sink.clone()));
                                state.send_modify(|s| {
                                    s.status = Status::Running;
                                    s.pid = Some(pid);
                                    s.started_at = Some(Instant::now());
                                });
                                running = Some(r);
                                lock_state(&paths.state_dir, true).await;
                                Ok(format!("Server started (PID: {pid})"))
                            }
                            Err(e) => {
                                state.send_modify(|s| s.status = Status::Stopped);
                                console
                                    .publish(format!("[LOCAL] Start failed: {e:#}"), "panel", false)
                                    .await;
                                Err(e)
                            }
                        }
                    }
                } else {
                    Ok(if action == "kill" {
                        "Server killed"
                    } else {
                        "Server stopped"
                    }
                    .into())
                };
                let _ = reply.send(result);
            }
            Wake::Exit(exit) => {
                commands.send_replace(None);
                if let Some(r) = running.take() {
                    drain(r).await;
                }
                let launch_ctx = launch_tx.borrow().clone();
                if launch_ctx.deployment == Deployment::Docker {
                    // Supervision split: the daemon's restart policy owns
                    // revival; the panel only re-attaches and reports.
                    if exit == "reattach needed" {
                        // The container is still running; only the panel's
                        // attach plumbing died. Re-attach without a crash.
                        console
                            .publish(
                                "[LOCAL] Container still running; reattaching console.".into(),
                                "panel",
                                false,
                            )
                            .await;
                        state.send_modify(|s| s.status = Status::Starting);
                        let current = settings.borrow().clone();
                        match attach_running_container(&config, &console, &launch_ctx, &current)
                            .await
                        {
                            Ok(r) => {
                                let pid = r.pid;
                                commands.send_replace(Some(r.sink.clone()));
                                state.send_modify(|s| {
                                    s.status = Status::Running;
                                    s.pid = Some(pid);
                                    s.started_at = Some(Instant::now());
                                });
                                running = Some(r);
                                lock_state(&paths.state_dir, true).await;
                            }
                            Err(e) => {
                                state.send_modify(|s| s.status = Status::Crashed);
                                console
                                    .publish(
                                        format!("[LOCAL] Reattach failed: {e:#}"),
                                        "panel",
                                        false,
                                    )
                                    .await;
                                // The container may be mid-restart or the
                                // daemon briefly gone; keep checking.
                                let delay = settings.borrow().auto_restart_delay.max(1);
                                restart_at = Some((
                                    tokio::time::Instant::now() + Duration::from_secs(delay),
                                    true,
                                ));
                            }
                        }
                    } else {
                        let clean = exit.ends_with("(exit code 0)");
                        state.send_modify(|s| {
                            s.status = if clean {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                            s.pid = None;
                            s.started_at = None;
                            if !clean {
                                s.crash_count += 1;
                            }
                        });
                        lock_state(&paths.state_dir, false).await;
                        let crashes = state.borrow().crash_count;
                        if clean {
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] {exit}; the container stays down unless its \
                                         restart policy revives it."
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                        } else {
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] {exit}, crash #{crashes}; the restart policy owns \
                                         revival and the panel reattaches once it runs again."
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                        }
                        // Recheck soon: attach when the daemon brought the
                        // container back, stay hands-off when it did not.
                        let delay = settings.borrow().auto_restart_delay.max(1);
                        restart_at = Some((
                            tokio::time::Instant::now() + Duration::from_secs(delay),
                            true,
                        ));
                    }
                    continue;
                }
                state.send_modify(|s| {
                    s.status = Status::Crashed;
                    s.pid = None;
                    s.started_at = None;
                    s.crash_count += 1;
                });
                lock_state(&paths.state_dir, false).await;
                let options = settings.borrow().clone();
                let crashes = state.borrow().crash_count;
                console
                    .publish(
                        format!("[LOCAL] Server exited unexpectedly ({exit}), crash #{crashes}"),
                        "panel",
                        false,
                    )
                    .await;
                if options.auto_restart && crashes <= options.max_crashes {
                    restart_at = Some((
                        tokio::time::Instant::now()
                            + Duration::from_secs(options.auto_restart_delay),
                        false,
                    ));
                }
            }
            Wake::Restart => {
                let (_at, attach_only) = restart_at
                    .take()
                    .unwrap_or((tokio::time::Instant::now(), false));
                state.send_modify(|s| s.status = Status::Starting);
                console.reset_stats();
                let current = settings.borrow().clone();
                let launch_ctx = launch_tx.borrow().clone();
                let result = if attach_only {
                    // Docker recheck: attach when the restart policy revived
                    // the container, never start or create anything.
                    attach_running_container(&config, &console, &launch_ctx, &current).await
                } else {
                    launch(&config, &paths, &current, &console, &launch_ctx).await
                };
                match result {
                    Ok(r) => {
                        let pid = r.pid;
                        commands.send_replace(Some(r.sink.clone()));
                        // crash_count is deliberately NOT reset here: the
                        // auto-restart budget counts across relaunches (only
                        // an explicit start/adopt starts a fresh streak).
                        state.send_modify(|s| {
                            s.status = Status::Running;
                            s.pid = Some(pid);
                            s.started_at = Some(Instant::now());
                        });
                        running = Some(r);
                        lock_state(&paths.state_dir, true).await;
                    }
                    Err(e) => {
                        // A recheck failure means the container is genuinely
                        // not running (the policy did not revive it): report
                        // stopped unless this was a real crash.
                        state.send_modify(|s| {
                            s.status = if attach_only && s.crash_count == 0 {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                        });
                        console
                            .publish(
                                format!("[LOCAL] Auto-restart failed: {e:#}"),
                                "panel",
                                false,
                            )
                            .await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod docker_args_tests {
    use super::*;
    use crate::instances::DockerConfig;

    fn ctx() -> LaunchContext {
        LaunchContext {
            instance_id: "test-instance".into(),
            deployment: Deployment::Docker,
            port: 20000,
            docker: DockerConfig {
                image: Some("omnibedrock-test:1".into()),
                extra_mounts: vec!["/srv/media:/data/media:ro".into()],
            },
            ..LaunchContext::default()
        }
    }

    fn plain(args: &[std::ffi::OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn detached_run_args_carry_policy_mounts_and_host_network() {
        let args = plain(&docker_run_args(
            &Config::default(),
            &ServerSettings::default(),
            &ctx(),
            true,
        ));
        assert_eq!(args[0], "docker");
        // Detached: the daemon owns the container; the panel only reads its
        // logs and writes its command FIFO.
        assert!(args.contains(&"-d".to_string()));
        assert!(!args.contains(&"-i".to_string()));
        let restart = args.iter().position(|a| a == "--restart").unwrap();
        assert_eq!(args[restart + 1], "unless-stopped");
        let name = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name + 1], "omnibedrock-test-instance");
        let net = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[net + 1], "host");
        assert!(
            !args.iter().any(|a| a == "-p"),
            "host network maps no ports"
        );
        let mount = args.iter().position(|a| a == "-v").unwrap();
        assert!(args[mount + 1].ends_with(":/data"));
        assert_eq!(
            args.iter()
                .filter(|a| a.as_str() == "/srv/media:/data/media:ro")
                .count(),
            1
        );
        let workdir = args.iter().position(|a| a == "-w").unwrap();
        assert_eq!(args[workdir + 1], "/data");
        let image = args.iter().position(|a| a == "omnibedrock-test:1").unwrap();
        // The command override follows the image and redirects stdin to the
        // FIFO both sides of the bind mount share.
        assert_eq!(
            args[image + 1..],
            [
                "sh",
                "-c",
                "exec python -m endstone --no-confirm -s /data <> /data/stdin.fifo"
            ]
        );
    }

    #[test]
    fn mapped_ports_replace_host_network_and_policy_follows_auto_restart() {
        let settings = ServerSettings {
            auto_restart: false,
            ..Default::default()
        };
        let args = plain(&docker_run_args(
            &Config::default(),
            &settings,
            &ctx(),
            false,
        ));
        assert!(!args.iter().any(|a| a == "--network"));
        let p1 = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p1 + 1], "20000:19132/udp");
        assert!(args.contains(&"20001:19133/udp".to_string()));
        let restart = args.iter().position(|a| a == "--restart").unwrap();
        assert_eq!(args[restart + 1], "no");
    }

    #[test]
    fn image_falls_back_to_the_config_default_per_runtime() {
        let config = Config::default();
        let mut c = ctx();
        c.docker.image = None;
        let args = plain(&docker_run_args(
            &config,
            &ServerSettings::default(),
            &c,
            true,
        ));
        assert!(args.contains(&config.docker_image_endstone));
    }

    #[test]
    fn limits_translate_to_container_flags() {
        let mut c = ctx();
        c.resources = ResourcePolicy {
            memory_mb: Some(512),
            cpus: Some(1.5),
        };
        let args = plain(&docker_run_args(
            &Config::default(),
            &ServerSettings::default(),
            &c,
            true,
        ));
        let memory = args.iter().position(|a| a == "--memory").unwrap();
        assert_eq!(args[memory + 1], "512m");
        assert!(args.contains(&"--memory-swap".to_string()));
        let cpus = args.iter().position(|a| a == "--cpus").unwrap();
        assert_eq!(args[cpus + 1], "1.5");
    }
}
