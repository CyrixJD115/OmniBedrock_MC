//! Shared supervisor data types: run status, state snapshots, backend
//! selection, per-instance settings, console events, error statistics,
//! instance filesystem paths, and the swappable launch context.

use crate::instances::{Deployment, DockerConfig, ResourcePolicy};
use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

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
