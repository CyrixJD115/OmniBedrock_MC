//! The public handle around the supervisor: `ServerManager` owns the control
//! channel, the state/settings/launch watch receivers, and the console ring,
//! and is the API the rest of the panel talks to.

use anyhow::{Context, bail};
use omnibedrock_core::config::Config;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::commands::{CommandSink, Control, dispatch_command};
use super::console::Console;
use super::supervisor::supervise;
use super::types::{LaunchContext, ServerPaths, ServerSettings, Snapshot, Status};
use crate::instances::DockerConfig;

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
