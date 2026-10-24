//! The supervisor exclusively owns the child. Readers only mutate bounded console state.
//! No process wait or pipe operation blocks a Tokio worker thread.
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
    Shutdown(Reply),
}
struct WriteCommand {
    text: String,
    reply: oneshot::Sender<anyhow::Result<()>>,
}
struct Running {
    child: Child,
    pid: u32,
    stdin: mpsc::Sender<WriteCommand>,
    writer: JoinHandle<()>,
    readers: Vec<JoinHandle<()>>,
}
#[derive(Clone)]
pub struct ServerManager {
    control: mpsc::Sender<Control>,
    commands: watch::Receiver<Option<mpsc::Sender<WriteCommand>>>,
    pub state: watch::Receiver<Snapshot>,
    pub settings: watch::Receiver<ServerSettings>,
    pub console: Console,
    shutdown: CancellationToken,
}
impl ServerManager {
    pub async fn new(config: Arc<Config>, shutdown: CancellationToken) -> anyhow::Result<Self> {
        let settings_path = config.data_dir.join("server_settings.yaml");
        let settings: ServerSettings = match tokio::fs::read_to_string(&settings_path).await {
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
            started_at: None,
        };
        let (state_tx, state) = watch::channel(snapshot);
        let (settings_tx, settings_rx) = watch::channel(settings);
        let (commands_tx, commands) = watch::channel(None);
        let (control, control_rx) = mpsc::channel(32);
        let console = Console::new(&config);
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
            console: console.clone(),
            shutdown: shutdown.clone(),
        };
        tokio::spawn(supervise(
            config,
            control_rx,
            state_tx,
            settings_tx,
            commands_tx,
            console,
            shutdown,
        ));
        Ok(manager)
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.state.borrow().clone();
        s.backend = self.settings.borrow().backend;
        s.uptime = s
            .started_at
            .filter(|_| s.status == Status::Running)
            .map_or(0.0, |i| i.elapsed().as_secs_f64());
        s.tps = self.console.tps();
        s
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
        let tx = self
            .commands
            .borrow()
            .clone()
            .context("Server is not running")?;
        write_command(&tx, command).await
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
fn launch_endstone(config: &Config) -> anyhow::Result<Child> {
    let Some(candidate) = crate::endstone::resolve_launch(config) else {
        anyhow::bail!(
            "Endstone is not installed — install the Endstone toolchain from \
             Settings → Server backend, or set ENDSTONE_BIN to an endstone executable"
        );
    };
    let mut cmd = match &candidate {
        crate::endstone::EndstoneLaunch::Binary(path) => {
            let mut cmd = Command::new(path);
            cmd.arg("--no-confirm")
                .arg("-s")
                .arg(&config.bedrock_server_dir);
            cmd
        }
        crate::endstone::EndstoneLaunch::PythonModule(python) => {
            let mut cmd = Command::new(python);
            cmd.args(["-m", "endstone", "--no-confirm", "-s"])
                .arg(&config.bedrock_server_dir);
            cmd
        }
    };
    configure_process(&mut cmd);
    cmd.spawn()
        .with_context(|| format!("launch Endstone ({candidate:?})"))
}
fn vanilla_binary() -> &'static std::path::Path {
    if cfg!(windows) {
        std::path::Path::new("bedrock_server.exe")
    } else {
        std::path::Path::new("bedrock_server")
    }
}
fn launch_vanilla(config: &Config) -> anyhow::Result<Child> {
    let binary = config.bedrock_server_dir.join(vanilla_binary());
    if !binary.exists() {
        bail!(
            "vanilla bedrock_server binary not found at {}; install a version from Settings → Backend",
            binary.display()
        );
    }
    let mut cmd = Command::new(&binary);
    cmd.current_dir(&config.bedrock_server_dir);
    configure_process(&mut cmd);
    cmd.spawn()
        .with_context(|| format!("launch vanilla BDS ({})", binary.display()))
}
async fn launch(
    config: &Config,
    settings: &ServerSettings,
    console: &Console,
) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.bedrock_server_dir).await?;
    let mut child = match settings.backend {
        Backend::Endstone => launch_endstone(config)?,
        Backend::Vanilla => launch_vanilla(config)?,
    };
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
        backend = settings.backend.as_str(),
        "game server started"
    );
    Ok(Running {
        child,
        pid,
        stdin: tx,
        writer,
        readers,
    })
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
fn signal_tree(r: &Running, force: bool) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        if let Err(e) = killpg(
            Pid::from_raw(r.pid as i32),
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
    let _ = (r, force);
}
async fn drain(mut r: Running) {
    // A launcher can exit while its descendants retain the pipes. Clean up its group.
    signal_tree(&r, true);
    r.writer.abort();
    let _ = r.writer.await;
    for mut reader in r.readers.drain(..) {
        if tokio::time::timeout(Duration::from_secs(30), &mut reader)
            .await
            .is_err()
        {
            reader.abort();
            let _ = reader.await;
        }
    }
}
async fn terminate(r: &mut Running, settings: &ServerSettings, force: bool) -> anyhow::Result<()> {
    if !force {
        let _ = write_command(&r.stdin, "stop").await;
        if tokio::time::timeout(Duration::from_secs(settings.stop_timeout), r.child.wait())
            .await
            .is_ok()
        {
            return Ok(());
        }
        signal_tree(r, false);
        #[cfg(not(unix))]
        r.child.start_kill()?;
        if tokio::time::timeout(Duration::from_secs(settings.kill_timeout), r.child.wait())
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    signal_tree(r, true);
    r.child.kill().await.context("kill child")?;
    Ok(())
}
async fn lock_state(config: &Config, locked: bool) {
    let data = format!(
        "console:\n  state: {}\n  timestamp: {}\n",
        if locked { "locked" } else { "unlocked" },
        chrono::Utc::now().timestamp()
    );
    if let Err(e) = atomic_write(
        &config.data_dir.join("console_lock_state.yaml"),
        data.as_bytes(),
    )
    .await
    {
        tracing::error!(%e, "console lock persistence failed");
    }
}
#[allow(clippy::too_many_arguments)]
async fn supervise(
    config: Arc<Config>,
    mut controls: mpsc::Receiver<Control>,
    state: watch::Sender<Snapshot>,
    settings: watch::Sender<ServerSettings>,
    commands: watch::Sender<Option<mpsc::Sender<WriteCommand>>>,
    console: Console,
    shutdown: CancellationToken,
) {
    let mut running: Option<Running> = None;
    let mut restart_at: Option<tokio::time::Instant> = None;
    lock_state(&config, false).await;
    loop {
        enum Wake {
            Control(Option<Control>),
            Exit(std::io::Result<std::process::ExitStatus>),
            Restart,
            Shutdown,
        }
        let wake = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Wake::Shutdown,
            c = controls.recv() => Wake::Control(c),
            exit = async { match running.as_mut() { Some(r) => r.child.wait().await, None => std::future::pending().await } } => Wake::Exit(exit),
            _ = async { match restart_at { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Wake::Restart,
        };
        match wake {
            Wake::Shutdown | Wake::Control(None) => {
                commands.send_replace(None);
                let options = settings.borrow().clone();
                if let Some(mut r) = running.take() {
                    state.send_modify(|s| s.status = Status::Stopping);
                    if let Err(e) = terminate(&mut r, &options, false).await {
                        tracing::error!(%e, "shutdown stop failed");
                    }
                    drain(r).await;
                }
                lock_state(&config, false).await;
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                state.send_modify(|s| s.status = Status::Stopping);
                commands.send_replace(None);
                let options = settings.borrow().clone();
                let result = if let Some(mut r) = running.take() {
                    let result = terminate(&mut r, &options, false).await;
                    drain(r).await;
                    result
                } else {
                    Ok(())
                };
                lock_state(&config, false).await;
                state.send_modify(|s| {
                    s.status = Status::Stopped;
                    s.pid = None;
                    s.started_at = None;
                });
                let _ = reply.send(result.map(|_| "Server stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Settings(new, reply))) => {
                let result = async {
                    atomic_write(
                        &config.data_dir.join("server_settings.yaml"),
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
                    lock_state(&config, false).await;
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
                        match launch(&config, &current, &console).await {
                            Ok(r) => {
                                let pid = r.pid;
                                commands.send_replace(Some(r.stdin.clone()));
                                state.send_modify(|s| {
                                    s.status = Status::Running;
                                    s.pid = Some(pid);
                                    s.started_at = Some(Instant::now());
                                });
                                running = Some(r);
                                lock_state(&config, true).await;
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
                lock_state(&config, false).await;
                let options = settings.borrow().clone();
                let crashes = state.borrow().crash_count;
                console
                    .publish(
                        format!("[LOCAL] Server exited unexpectedly ({exit:?}), crash #{crashes}"),
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
                match launch(&config, &current, &console).await {
                    Ok(r) => {
                        commands.send_replace(Some(r.stdin.clone()));
                        state.send_modify(|s| {
                            s.status = Status::Running;
                            s.pid = Some(r.pid);
                            s.started_at = Some(Instant::now());
                        });
                        running = Some(r);
                        lock_state(&config, true).await;
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
