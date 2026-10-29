//! One managed instance: its record plus every service bound to its paths
//! (server manager, BDS/Endstone watchers, broadcaster, backups, and the
//! lifecycle token).

use super::types::InstanceRecord;
use crate::{
    backups::Backups, bds::Bds, broadcaster::Broadcaster, endstone::Endstone, server::ServerManager,
};
use omnibedrock_core::config::Config;
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

/// One managed instance: its record plus every service bound to its paths.
pub struct Instance {
    pub record: RwLock<InstanceRecord>,
    pub config: Arc<Config>,
    pub server: ServerManager,
    pub bds: Bds,
    pub endstone: Endstone,
    /// Every instance broadcasts independently: own config.yml, account
    /// token cache, screenshots, and console under its broadcaster dir.
    pub broadcaster: Broadcaster,
    pub backups: Backups,
    pub token: CancellationToken,
    /// Serializes lifecycle operations (start/stop/install/backup) for this
    /// instance only; other instances are unaffected.
    pub operations: Arc<tokio::sync::Mutex<()>>,
}

impl Instance {
    pub fn record(&self) -> InstanceRecord {
        self.record.read().expect("instance record lock").clone()
    }
    /// Summary JSON: identity, runtime state, and configured limits.
    pub fn summary(&self) -> Value {
        let record = self.record();
        let snapshot = self.server.snapshot();
        json!({
            "id": record.id,
            "name": record.name,
            "owner": record.owner,
            "created_at": record.created_at,
            "port": record.port,
            "deployment": record.deployment.as_str(),
            "docker": record.docker,
            "enforcement": record.deployment.enforcement(),
            "resources": {
                "memory_mb": record.resources.memory_mb,
                "cpus": record.resources.cpus,
            },
            "access": record.access,
            "autostart": record.autostart,
            "backend": snapshot.backend,
            "status": snapshot.status,
            "uptime": snapshot.uptime,
            "pid": snapshot.pid,
        })
    }
}
