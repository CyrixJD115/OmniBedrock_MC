//! Shared broadcaster data types: the status snapshot, device-code records,
//! relay-config analysis results, and ping health the API surfaces, plus the
//! supervisor's control-message plumbing (Control commands and the Running
//! process handle).

use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

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
    pub(super) fn unknown() -> Self {
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

pub(super) type Reply = oneshot::Sender<anyhow::Result<String>>;
pub(super) enum Control {
    Start(Reply),
    Stop(Reply),
    Restart(Reply),
    /// Stop a foreign broadcaster for this instance, then start a panel-owned
    /// one in its place.
    TakeOver(Reply),
    Command(String, oneshot::Sender<anyhow::Result<()>>),
    Shutdown(Reply),
}
pub(super) struct Running {
    pub(super) child: Child,
    pub(super) pid: u32,
    /// The stdin writer task; `None` for the docker sidecar (its commands go
    /// through the FIFO, and the child is only a `docker logs` reader).
    pub(super) stdin: Option<mpsc::Sender<String>>,
    pub(super) writer: Option<tokio::task::JoinHandle<()>>,
    pub(super) readers: Vec<tokio::task::JoinHandle<()>>,
    /// Set when the child reads the sidecar container's logs.
    pub(super) container: Option<String>,
    /// Sidecar command FIFO on the host (inside the mounted dir).
    pub(super) fifo: Option<PathBuf>,
}
