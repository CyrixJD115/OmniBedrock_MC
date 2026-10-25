//! The supervisor exclusively owns the child. Readers only mutate bounded console state.
//! No process wait or pipe operation blocks a Tokio worker thread.
use crate::instances::{Deployment, ResourcePolicy};
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
/// owns (native or a `docker run` client) or a systemd user service.
struct Running {
    pid: u32,
    sink: CommandSink,
    kind: RunKind,
}

enum RunKind {
    /// A process the panel supervises directly, with piped stdio.
    Process {
        child: Child,
        stdin: mpsc::Sender<WriteCommand>,
        writer: JoinHandle<()>,
        readers: Vec<JoinHandle<()>>,
        /// Docker container name when the process is a `docker run` client.
        container: Option<String>,
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
/// port, and docker images. Swappable while stopped via `set_launch`.
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
    pub docker_image_vanilla: String,
    pub docker_image_endstone: String,
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
            docker_image_vanilla: "omnibedrock-vanilla:latest".into(),
            docker_image_endstone: "omnibedrock-endstone:latest".into(),
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
                docker_image_vanilla: config.docker_image_vanilla.clone(),
                docker_image_endstone: config.docker_image_endstone.clone(),
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

/// Full `docker run` argv for an instance. The instance's server directory is
/// mounted at /data; the image provides the runtime (see deploy/docker).
fn docker_argv(
    config: &Config,
    settings: &ServerSettings,
    ctx: &LaunchContext,
) -> Vec<std::ffi::OsString> {
    let name = crate::instances::container_name(&ctx.instance_id);
    let mut args: Vec<std::ffi::OsString> = vec![
        "docker".into(),
        "run".into(),
        "-i".into(),
        "--rm".into(),
        "--name".into(),
        name.clone().into(),
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
    args.push("-p".into());
    args.push(format!("{}:19132/udp", ctx.port).into());
    args.push("-p".into());
    args.push(format!("{}:19133/udp", ctx.port + 1).into());
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
    args.push("-w".into());
    args.push("/data".into());
    let image = match settings.backend {
        Backend::Vanilla => &ctx.docker_image_vanilla,
        Backend::Endstone => &ctx.docker_image_endstone,
    };
    args.push(image.clone().into());
    args
}

/// Best-effort removal of a stale container before `docker run` reuses its
/// name (a killed panel can leave `--rm` containers behind).
async fn docker_rm_stale(container: &str) {
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("docker")
            .args(["rm", "-f", container])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
    )
    .await;
}

async fn docker_kill(container: &str) {
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("docker")
            .args(["kill", container])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
    )
    .await;
}

/// Runtime-only Dockerfile bodies for the two instance images, identical to
/// deploy/docker in the repository. Generated under the cache directory so
/// packaged installations (with no checkout around) can still build them.
const DOCKERFILE_VANILLA: &str = "\
# Generated by OmniBedrock; runtime libraries for the official BDS binary.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4 libzip4 curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /data
CMD [\"./bedrock_server\"]
";

const DOCKERFILE_ENDSTONE: &str = "\
# Generated by OmniBedrock; Endstone runtime on CPython 3.12.
FROM python:3.12-slim
RUN pip install --no-cache-dir endstone
WORKDIR /data
CMD [\"python\", \"-m\", \"endstone\", \"--no-confirm\", \"-s\", \"/data\"]
";

/// Make sure the configured image exists; when it does not, generate its
/// Dockerfile under the cache directory and build it, streaming progress
/// into the instance console. Custom (registry) images are never built —
/// building only ever targets the panel's own `omnibedrock-*` images.
async fn ensure_docker_image(
    config: &Config,
    image: &str,
    backend: Backend,
    console: &Console,
) -> anyhow::Result<()> {
    let inspect = tokio::process::Command::new("docker")
        .args(["image", "inspect", image])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    if inspect.is_ok_and(|s| s.success()) {
        return Ok(());
    }
    if !image.starts_with("omnibedrock-") {
        bail!(
            "image {image} is missing; pull it or build it manually \
             (the panel only auto-builds its own omnibedrock-* images)"
        );
    }
    let safe: String = image
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let context = config.cache_dir.join("docker").join(safe);
    tokio::fs::create_dir_all(&context).await?;
    let dockerfile = context.join("Dockerfile");
    let body = match backend {
        Backend::Vanilla => DOCKERFILE_VANILLA,
        Backend::Endstone => DOCKERFILE_ENDSTONE,
    };
    // Only ever (re)generate our own generated context; anything unexpected
    // in that directory is left alone and reported.
    if tokio::fs::try_exists(&dockerfile).await.unwrap_or(false) {
        let existing = tokio::fs::read_to_string(&dockerfile)
            .await
            .unwrap_or_default();
        anyhow::ensure!(
            existing.starts_with("# Generated by OmniBedrock"),
            "refusing to overwrite {}",
            dockerfile.display()
        );
    }
    tokio::fs::write(&dockerfile, body).await?;
    console
        .publish(
            format!("[LOCAL] Building missing image {image}…"),
            "panel",
            false,
        )
        .await;
    let build = tokio::time::timeout(
        Duration::from_secs(600),
        tokio::process::Command::new("docker")
            // No --progress=plain: minimal Docker installs ship the legacy
            // builder, which rejects the flag; it streams to stderr anyway.
            .args(["build"])
            .arg("-t")
            .arg(image)
            .arg(&context)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .context("docker build timed out (10 min)")?
    .context("failed to run docker build")?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    for line in output.lines().filter(|l| l.contains("ERROR")).take(20) {
        console
            .publish(format!("[LOCAL] {line}"), "panel", false)
            .await;
    }
    if !build.status.success() {
        bail!("docker build for {image} failed (see console for error lines)");
    }
    console
        .publish(format!("[LOCAL] Image {image} built."), "panel", false)
        .await;
    Ok(())
}

/// Spawn one process the panel owns (native instance or `docker run` client)
/// and wire its stdio into the console.
#[allow(clippy::too_many_arguments)]
async fn spawn_process(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    settings: &ServerSettings,
    argv: Vec<std::ffi::OsString>,
    container: Option<String>,
) -> anyhow::Result<Running> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if ctx.deployment != Deployment::Docker {
        // Native children inherit the server directory so vanilla BDS finds
        // its own files; docker uses -w inside the image.
        cmd.current_dir(&config.bedrock_server_dir);
    }
    configure_process(&mut cmd);
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
        backend = settings.backend.as_str(),
        deployment = ctx.deployment.as_str(),
        "game server started"
    );
    Ok(Running {
        pid,
        sink: CommandSink::Pipe(tx.clone()),
        kind: RunKind::Process {
            child,
            stdin: tx,
            writer,
            readers,
            container,
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
    // wraps BDS). Native/systemd pin the instance's registered pair
    // (P v4 / P+1 v6); docker pins the container-internal stock pair
    // 19132/19133 and publishes both through the host port mappings.
    match ctx.deployment {
        Deployment::Docker => {
            crate::instances::enforce_instance_port(config, 19132).await;
        }
        _ => {
            crate::instances::enforce_instance_port(config, ctx.port).await;
        }
    }
    crate::instances::warn_transport_mismatch(config, console).await;
    let base_argv = match settings.backend {
        Backend::Endstone => base_argv_endstone(config)?,
        Backend::Vanilla => base_argv_vanilla(config)?,
    };
    match ctx.deployment {
        Deployment::Native => spawn_process(config, console, ctx, settings, base_argv, None).await,
        Deployment::Docker => {
            let image = match settings.backend {
                Backend::Vanilla => &ctx.docker_image_vanilla,
                Backend::Endstone => &ctx.docker_image_endstone,
            };
            ensure_docker_image(config, image, settings.backend, console).await?;
            let container = crate::instances::container_name(&ctx.instance_id);
            docker_rm_stale(&container).await;
            spawn_process(
                config,
                console,
                ctx,
                settings,
                docker_argv(config, settings, ctx),
                Some(container),
            )
            .await
        }
        Deployment::Systemd => {
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
fn configure_process(cmd: &mut Command) {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PYTHONUNBUFFERED", "1");
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
            container,
            ..
        } => {
            // A launcher can exit while its descendants retain the pipes.
            // Clean up its group.
            signal_tree(pid, true);
            drop(child);
            // Killing the docker-run client does not stop the container; force it.
            if let Some(container) = &container {
                docker_kill(container).await;
            }
            writer.abort();
            let _ = writer.await;
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
            if !force {
                let _ = write_command(stdin, "stop").await;
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
            // Docker: killing the docker-run client leaves the container
            // running; the daemon must kill it before the name can be reused.
            if let Some(container) = container.clone() {
                docker_kill(&container).await;
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
    let mut restart_at: Option<tokio::time::Instant> = None;
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
            _ = async { match restart_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Wake::Restart,
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
                // After a panel restart, adopt a unit that is still active.
                let launch_ctx = launch_tx.borrow().clone();
                let result = if running.is_some() {
                    Ok("Server is already running".into())
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
                    restart_at = Some(
                        tokio::time::Instant::now()
                            + Duration::from_secs(options.auto_restart_delay),
                    );
                }
            }
            Wake::Restart => {
                restart_at = None;
                state.send_modify(|s| s.status = Status::Starting);
                console.reset_stats();
                let current = settings.borrow().clone();
                let launch_ctx = launch_tx.borrow().clone();
                match launch(&config, &paths, &current, &console, &launch_ctx).await {
                    Ok(r) => {
                        commands.send_replace(Some(r.sink.clone()));
                        state.send_modify(|s| {
                            s.status = Status::Running;
                            s.pid = Some(r.pid);
                            s.started_at = Some(Instant::now());
                        });
                        running = Some(r);
                        lock_state(&paths.state_dir, true).await;
                    }
                    Err(e) => {
                        state.send_modify(|s| s.status = Status::Crashed);
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
