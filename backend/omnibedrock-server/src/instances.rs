//! Multi-instance management: instance records, resource policies, deployment
//! backends, per-instance service bundles, and the legacy→default migration.
//!
//! The panel used to manage exactly one server pointed at
//! `Config::bedrock_server_dir`. On boot that server becomes the `default`
//! instance and keeps every legacy path, so existing installations start
//! working unchanged. Instances created afterwards live under
//! `<data_dir>/instances/<id>/` with their own server files, settings,
//! console journal, and backups.
use crate::{
    backups::Backups,
    bds::Bds,
    broadcaster::Broadcaster,
    endstone::Endstone,
    server::{Backend, Console, LaunchContext, ServerManager, ServerPaths, Status},
};
use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use tokio::sync::RwLock as AsyncRwLock;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_INSTANCE: &str = "default";

/// Hard per-instance resource limits. `None` means unlimited. Enforcement
/// depends on the deployment backend; native processes are only monitored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourcePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
}

impl ResourcePolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(mb) = self.memory_mb {
            anyhow::ensure!(
                (128..=1_048_576).contains(&mb),
                "memory limit must be between 128 MiB and 1 TiB"
            );
        }
        if let Some(cpus) = self.cpus {
            anyhow::ensure!(
                (0.1..=256.0).contains(&cpus),
                "cpu limit must be between 0.1 and 256"
            );
        }
        Ok(())
    }
    /// systemd CPUQuota percent (1.00 CPU = 100%).
    pub fn cpu_quota_percent(&self) -> Option<u32> {
        self.cpus
            .map(|c| (c * 100.0).clamp(1.0, 1_000_000.0) as u32)
    }
    pub fn is_unlimited(&self) -> bool {
        self.memory_mb.is_none() && self.cpus.is_none()
    }
}

/// Aggregate ceilings for one panel user. `None` = unlimited. Enforced at
/// instance create/update time while the manager write lock is held, so
/// concurrent requests cannot race past the check.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UserQuotas {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
}

impl UserQuotas {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(n) = self.max_instances {
            anyhow::ensure!(n > 0, "max_instances must be at least 1");
        }
        if let Some(mb) = self.memory_mb {
            anyhow::ensure!(mb >= 128, "memory quota must be at least 128 MiB");
        }
        if let Some(c) = self.cpus {
            anyhow::ensure!(c >= 0.1, "cpu quota must be at least 0.1");
        }
        Ok(())
    }
    /// Check a user's aggregate allocation against these ceilings.
    /// `owned` are the records the user owns; `extra` is the delta being added.
    pub fn check(&self, owned: &[&InstanceRecord], extra: ResourcePolicy) -> anyhow::Result<()> {
        if let Some(max) = self.max_instances {
            anyhow::ensure!(
                owned.len() < max as usize,
                "instance quota reached (max {max})"
            );
        }
        let memory: u64 = owned
            .iter()
            .map(|r| r.resources.memory_mb.unwrap_or(0))
            .sum();
        let cpus: f64 = owned.iter().map(|r| r.resources.cpus.unwrap_or(0.0)).sum();
        if let Some(limit) = self.memory_mb {
            anyhow::ensure!(
                memory + extra.memory_mb.unwrap_or(0) <= limit,
                "memory quota exceeded: {memory} MiB allocated + {} MiB requested > {limit} MiB",
                extra.memory_mb.unwrap_or(0)
            );
        }
        if let Some(limit) = self.cpus {
            let total = cpus + extra.cpus.unwrap_or(0.0);
            anyhow::ensure!(
                total <= limit + 1e-9,
                "cpu quota exceeded: {cpus:.2} allocated + {:.2} requested > {limit:.2}",
                extra.cpus.unwrap_or(0.0)
            );
        }
        Ok(())
    }
}

/// How an instance's process is launched and its limits enforced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Deployment {
    /// Plain child process. Limits are advisory (monitoring only).
    #[default]
    Native,
    /// `systemd-run`-free hosting in a persistent user service unit
    /// (`omnibedrock-<id>.service`): real cgroup memory/CPU controls and the
    /// server survives panel restarts.
    Systemd,
    /// Docker container: limits enforced by the container runtime.
    Docker,
}

impl Deployment {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "native" => Ok(Self::Native),
            "systemd" => Ok(Self::Systemd),
            "docker" => Ok(Self::Docker),
            other => bail!("unknown deployment '{other}' (expected native, systemd, or docker)"),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Systemd => "systemd",
            Self::Docker => "docker",
        }
    }
    /// What a configured limit actually enforces on this backend.
    pub fn enforcement(self) -> &'static str {
        match self {
            Self::Native => "advisory",
            Self::Systemd => "cgroup",
            Self::Docker => "container",
        }
    }
    pub fn enforcement_description(self) -> &'static str {
        match self {
            Self::Native => {
                "Advisory only: the panel reports usage but the OS does not enforce limits."
            }
            Self::Systemd => {
                "Enforced by systemd cgroup controls (MemoryMax/CPUQuota) on this Linux host; the instance runs as a user service that survives panel restarts."
            }
            Self::Docker => {
                "Enforced by the container runtime (--memory/--cpus); the container is killed on OOM."
            }
        }
    }
}

/// Per-instance Docker options for the `docker` deployment backend. Optional
/// everywhere: instances that never touch Docker keep `None`, and every field
/// falls back to the panel-level defaults (see `Config::docker_image_*`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Image override; `None` uses the config default for the active runtime
    /// (omnibedrock-vanilla / omnibedrock-endstone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Extra `-v` bind mounts applied verbatim, one `host:container[:opts]`
    /// spec each. The server directory mount is implicit and always present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_mounts: Vec<String>,
}

impl DockerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(image) = &self.image {
            anyhow::ensure!(
                !image.trim().is_empty() && !image.contains(char::is_whitespace),
                "docker image must be a single non-empty reference"
            );
        }
        for spec in &self.extra_mounts {
            let parts: Vec<&str> = spec.split(':').collect();
            anyhow::ensure!(
                (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty()),
                "extra mount '{spec}' must look like host:container[:opts]"
            );
            anyhow::ensure!(
                !spec.chars().any(char::is_whitespace),
                "extra mount '{spec}' must not contain whitespace"
            );
            anyhow::ensure!(
                parts[1].starts_with('/'),
                "extra mount '{spec}' container path must be absolute"
            );
        }
        Ok(())
    }
    /// The image for one runtime: the per-instance override, else the config
    /// default for that backend.
    pub fn image_for(&self, backend: crate::server::Backend, config: &Config) -> String {
        self.image.clone().unwrap_or_else(|| match backend {
            crate::server::Backend::Vanilla => config.docker_image_vanilla.clone(),
            crate::server::Backend::Endstone => config.docker_image_endstone.clone(),
        })
    }
}

/// Instance access is expressed with the SAME role catalog as global roles:
/// an entry assigns catalog roles to one user on this instance. Legacy files
/// store a two-level `level` field (manage|view); it migrates to the roles
/// `instance-manager` / `viewer` on load and is never written again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessEntry {
    pub user: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Legacy level field; consumed by `AccessEntry::normalized`.
    #[serde(default, skip_serializing)]
    pub level: Option<String>,
}
impl AccessEntry {
    pub fn roles(&self) -> &[String] {
        &self.roles
    }
    /// Map a legacy view/manage level onto catalog roles and drop the legacy
    /// field. No-op for entries already stored with roles.
    pub fn normalized(mut self) -> Self {
        if self.roles.is_empty()
            && let Some(level) = self.level.as_deref()
        {
            self.roles = match level.trim().to_ascii_lowercase().as_str() {
                "manage" => vec!["instance-manager".into()],
                _ => vec!["viewer".into()],
            };
        }
        self.level = None;
        self
    }
}

fn default_port() -> u16 {
    19132
}

/// Everything the panel persists about one instance. The running services are
/// built from this record; identity fields never change after creation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub created_at: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub deployment: Deployment,
    /// Docker options; only meaningful when `deployment` is Docker. Older
    /// records (and every native/systemd instance) carry `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerConfig>,
    #[serde(default)]
    pub resources: ResourcePolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<AccessEntry>,
    /// systemd only: start the unit at boot (`systemctl --user enable`).
    /// Requires linger for user units to run without a login session.
    #[serde(default)]
    pub autostart: bool,
}

impl InstanceRecord {
    /// The access entry for one user, if any. Entry membership grants
    /// visibility; the entry's roles determine what the user may do there.
    pub fn access_entry(&self, user: &str) -> Option<&AccessEntry> {
        self.access.iter().find(|a| a.user == user)
    }
    /// Drop the legacy level field from every entry (call after loading).
    pub fn normalized(mut self) -> Self {
        self.access = self
            .access
            .drain(..)
            .map(AccessEntry::normalized)
            .filter(|a| !a.roles.is_empty())
            .collect();
        self
    }
}

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

/// Probed once at boot and reported to the UI so deployment choices can only
/// reference backends that actually work on this host.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeployCapabilities {
    pub docker: Option<String>,
    /// Host OS as reported to the UI ("linux" | "windows" | "macos"); the
    /// instance dialog preselects the deployment default for this platform.
    pub platform: String,
    /// A systemd user manager is reachable (user-service instances can run).
    pub systemd: bool,
    /// Linger is on: user units keep running (and start at boot) without an
    /// active login session. Required for systemd autostart to mean anything.
    pub linger: bool,
}

/// Owns every instance, persists `instances.yaml`, and creates/removes
/// instances under a single write lock (quota checks run inside that lock).
pub struct InstanceManager {
    config: Arc<Config>,
    instances: AsyncRwLock<BTreeMap<String, Arc<Instance>>>,
    capabilities: DeployCapabilities,
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Valid instance ids: lowercase slugs so they are safe as directory names,
/// unit names, and container names.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Derive an instance id from a display name ("Dad's Server!" → "dads-server").
pub fn slug(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches(['-']).chars().take(48).collect()
}

impl InstanceManager {
    pub async fn new(config: Arc<Config>, default_owner: String) -> anyhow::Result<Self> {
        let capabilities = probe_capabilities().await;
        let registry = config.data_dir.join("instances.yaml");
        let mut records: BTreeMap<String, InstanceRecord> =
            match tokio::fs::read_to_string(&registry).await {
                Ok(text) => serde_yaml::from_str(&text)
                    .with_context(|| format!("invalid {}", registry.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => return Err(e.into()),
            };
        let mut changed = false;
        // Migrate legacy view/manage access levels onto catalog roles.
        for record in records.values_mut() {
            if record.access.iter().any(|a| a.level.is_some()) {
                *record = record.clone().normalized();
                changed = true;
            }
        }
        if !records.contains_key(DEFAULT_INSTANCE) {
            // Migration: the pre-multi-instance panel managed one server at
            // Config::bedrock_server_dir. It becomes the `default` instance and
            // keeps every legacy path; nothing moves on disk.
            records.insert(
                DEFAULT_INSTANCE.into(),
                InstanceRecord {
                    id: DEFAULT_INSTANCE.into(),
                    name: "Default Server".into(),
                    owner: default_owner,
                    created_at: timestamp(),
                    port: legacy_port(&config).await,
                    deployment: Deployment::Native,
                    docker: None,
                    resources: ResourcePolicy::default(),
                    access: vec![],
                    autostart: false,
                },
            );
            changed = true;
        }
        let mut instances = BTreeMap::new();
        for (id, record) in records {
            instances.insert(id, build_instance(&config, record).await?);
        }
        let manager = Self {
            config,
            instances: AsyncRwLock::new(instances),
            capabilities,
        };
        if changed {
            manager.save().await?;
        }
        Ok(manager)
    }

    pub fn capabilities(&self) -> &DeployCapabilities {
        &self.capabilities
    }

    async fn save(&self) -> anyhow::Result<()> {
        let instances = self.instances.read().await;
        let records: BTreeMap<String, InstanceRecord> = instances
            .iter()
            .map(|(id, i)| (id.clone(), i.record()))
            .collect();
        atomic_write(
            &self.config.data_dir.join("instances.yaml"),
            serde_yaml::to_string(&records)?.as_bytes(),
        )
        .await
    }

    /// Rewrite a renamed role across every instance's access entries so
    /// memberships survive role renames.
    pub async fn rename_access_role(&self, from: &str, to: &str) -> anyhow::Result<()> {
        let mut instances = self.instances.write().await;
        let mut changed = false;
        for inst in instances.values_mut() {
            let mut record = inst.record.read().expect("instance record lock").clone();
            let mut touched = false;
            for entry in record.access.iter_mut() {
                for role in entry.roles.iter_mut() {
                    if role == from {
                        *role = to.to_string();
                        touched = true;
                    }
                }
            }
            if touched {
                *inst.record.write().expect("instance record lock") = record;
                changed = true;
            }
        }
        drop(instances);
        if changed {
            self.save().await?;
        }
        Ok(())
    }

    /// Remove a deleted role from every access entry; entries left without
    /// roles grant nothing and are dropped.
    pub async fn strip_access_role(&self, name: &str) -> anyhow::Result<()> {
        let mut instances = self.instances.write().await;
        let mut changed = false;
        for inst in instances.values_mut() {
            let mut record = inst.record.read().expect("instance record lock").clone();
            let before = record.access.len();
            for entry in record.access.iter_mut() {
                entry.roles.retain(|r| r != name);
            }
            record.access.retain(|a| !a.roles.is_empty());
            if record.access.len() != before || record.access.iter().any(|a| a.user.is_empty()) {
                *inst.record.write().expect("instance record lock") = record;
                changed = true;
            }
        }
        drop(instances);
        if changed {
            self.save().await?;
        }
        Ok(())
    }

    /// Snapshot of every record, for quota sums and listings.
    pub async fn records(&self) -> Vec<InstanceRecord> {
        self.instances
            .read()
            .await
            .values()
            .map(|i| i.record())
            .collect()
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Instance>> {
        self.instances.read().await.get(id).cloned()
    }

    pub async fn list(&self) -> Vec<Arc<Instance>> {
        self.instances.read().await.values().cloned().collect()
    }

    /// Check that no other instance claims `port` and the OS can bind it.
    /// Takes a record snapshot so callers can hold the manager write lock.
    async fn ensure_port_free(
        &self,
        port: u16,
        ignore: Option<&str>,
        records: &[InstanceRecord],
    ) -> anyhow::Result<()> {
        for record in records {
            if Some(record.id.as_str()) != ignore && port_pairs_overlap(record.port, port) {
                bail!(
                    "ports {port}/{} are already used by instance '{}' ({}#{})",
                    port + 1,
                    record.id,
                    record.port,
                    record.port + 1
                );
            }
        }
        anyhow::ensure!(
            port_bindable(port).await && port_bindable(port + 1).await,
            "UDP ports {port}/{} are not available on this host (checked IPv4 and IPv6)",
            port + 1
        );
        Ok(())
    }

    /// Auto-allocate the lowest free port at or above 19132.
    async fn next_port(&self) -> u16 {
        let taken: Vec<u16> = self
            .instances
            .read()
            .await
            .values()
            .map(|i| i.record().port)
            .collect();
        for port in 19132u16.. {
            let pair_free = taken.iter().all(|t| !port_pairs_overlap(*t, port));
            if pair_free && port_bindable(port).await && port_bindable(port + 1).await {
                return port;
            }
        }
        19132
    }

    /// Create an instance. `quota_check` runs while the write lock is held so
    /// concurrent creates cannot both pass the same quota.
    pub async fn create(
        &self,
        mut record: InstanceRecord,
        quota_check: impl FnOnce(&[InstanceRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<Arc<Instance>> {
        record.resources.validate()?;
        if let Some(docker) = &record.docker {
            docker.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        match record.deployment {
            Deployment::Systemd if !self.capabilities.systemd => {
                bail!(
                    "systemd user scopes are not available on this host (Linux with a systemd user session required)"
                )
            }
            Deployment::Docker if self.capabilities.docker.is_none() => {
                bail!(
                    "Docker is not available on this host (is the docker CLI installed and the daemon running?)"
                )
            }
            _ => {}
        }
        let port = if record.port == 0 {
            self.next_port().await
        } else {
            record.port
        };
        record.port = port;
        let mut instances = self.instances.write().await;
        // Quota, id, and port checks all run under the write lock (snapshots,
        // never re-locking) so concurrent creates cannot race past them.
        if instances.contains_key(&record.id) {
            bail!("instance id '{}' already exists", record.id);
        }
        let snapshot: Vec<InstanceRecord> = instances.values().map(|i| i.record()).collect();
        quota_check(&snapshot)?;
        self.ensure_port_free(port, None, &snapshot).await?;
        let instance = build_instance(&self.config, record.clone()).await?;
        seed_server_properties(&instance.config, port).await;
        instances.insert(record.id.clone(), instance.clone());
        drop(instances);
        self.save().await?;
        Ok(instance)
    }

    /// Apply an updated record. Changes to deployment, resources, or port are
    /// rejected while the instance runs because they only apply at launch.
    pub async fn configure(
        &self,
        id: &str,
        next: InstanceRecord,
        quota_check: impl FnOnce(&[InstanceRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<Arc<Instance>> {
        next.resources.validate()?;
        if let Some(docker) = &next.docker {
            docker.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        let instance = self
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("instance '{id}' not found"))?;
        let current = instance.record();
        if next.id != current.id || next.owner != current.owner {
            bail!("instance id and owner cannot change");
        }
        let running = instance.server.snapshot().status == Status::Running;
        let relaunch_changed = next.deployment != current.deployment
            || next.docker != current.docker
            || next.resources != current.resources
            || next.port != current.port;
        if running && relaunch_changed {
            bail!("stop the instance before changing deployment, resources, or port");
        }
        if next.deployment == Deployment::Systemd && !self.capabilities.systemd {
            bail!("systemd user scopes are not available on this host");
        }
        if next.deployment == Deployment::Docker && self.capabilities.docker.is_none() {
            bail!("Docker is not available on this host");
        }
        if next.port != current.port {
            let snapshot = self.records().await;
            self.ensure_port_free(next.port, Some(id), &snapshot)
                .await?;
        }
        {
            let instances = self.instances.write().await;
            // Quota check against the projected post-change record set; the
            // stored record is only touched once it passes.
            let projected: Vec<InstanceRecord> = instances
                .values()
                .map(|i| {
                    let record = i.record();
                    if record.id == next.id {
                        next.clone()
                    } else {
                        record
                    }
                })
                .collect();
            quota_check(&projected)?;
            *instance.record.write().expect("instance record lock") = next.clone();
            instance.server.set_launch(launch_context(&next));
            instance
                .broadcaster
                .set_docker(broadcaster_docker_spec(&next, &self.config));
            if next.port != current.port {
                enforce_instance_port(&instance.config, next.port).await;
            }
        }
        self.save().await?;
        // systemd unit lifecycle: retiring the deployment removes the unit;
        // an autostart toggle syncs the enable state when the unit exists.
        if current.deployment == Deployment::Systemd && next.deployment != Deployment::Systemd {
            if let Err(e) = crate::systemd::remove_unit(&next.id).await {
                tracing::warn!(%e, "retiring the old systemd unit failed");
            }
        } else if next.deployment == Deployment::Systemd
            && next.autostart != current.autostart
            && crate::systemd::unit_path(&next.id)
                .map(|p| p.exists())
                .unwrap_or(false)
            && let Err(e) = crate::systemd::set_enabled(&next.id, next.autostart).await
        {
            tracing::warn!(%e, "syncing unit enable state failed");
        }
        Ok(instance)
    }

    /// Stop and remove an instance. `delete_data` also removes its server
    /// files, backups, and logs; the primary `default` instance is permanent.
    pub async fn remove(&self, id: &str, delete_data: bool) -> anyhow::Result<()> {
        if id == DEFAULT_INSTANCE {
            bail!("the default instance cannot be deleted; it holds the legacy server");
        }
        let instance = self
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("instance '{id}' not found"))?;
        let deployment = instance.record().deployment;
        if instance.server.snapshot().status != Status::Stopped {
            let _ = instance.server.action("kill").await;
        }
        let _ = instance.server.shutdown().await;
        if deployment == Deployment::Systemd
            && let Err(e) = crate::systemd::remove_unit(id).await
        {
            tracing::warn!(%e, "removing the instance's systemd unit failed");
        }
        instance.bds.shutdown().await;
        instance.endstone.shutdown().await;
        let _ = instance.broadcaster.shutdown().await;
        if deployment == Deployment::Docker {
            crate::docker::rm(&container_name(id)).await;
            crate::docker::rm(&format!("{}-broadcaster", container_name(id))).await;
        }
        instance.token.cancel();
        let removed = {
            let mut instances = self.instances.write().await;
            instances.remove(id)
        };
        if removed.is_some() {
            self.save().await?;
        }
        if delete_data {
            // The instance root (settings, console state, the unit FIFO) sits
            // directly under <data_dir>/instances/<id> for non-default
            // instances and is removed with the rest of its data.
            let root = self.config.data_dir.join("instances").join(id);
            for dir in [
                instance.config.bedrock_server_dir.clone(),
                instance.config.backups_dir.clone(),
                instance.config.logs_dir.clone(),
                root,
            ] {
                if (dir.starts_with(&self.config.data_dir)
                    || dir.starts_with(&self.config.backups_dir)
                    || dir.starts_with(&self.config.logs_dir))
                    && let Err(e) = tokio::fs::remove_dir_all(&dir).await
                {
                    tracing::warn!(%e, dir = %dir.display(), "instance data cleanup failed");
                }
            }
        }
        Ok(())
    }

    /// Stop every instance's services (panel shutdown).
    pub async fn shutdown_all(&self) {
        let instances = self.list().await;
        for instance in instances {
            let _ = instance.server.shutdown().await;
            instance.bds.shutdown().await;
            instance.endstone.shutdown().await;
            let _ = instance.broadcaster.shutdown().await;
            instance.token.cancel();
        }
    }
}

fn launch_context(record: &InstanceRecord) -> LaunchContext {
    LaunchContext {
        instance_id: record.id.clone(),
        instance_name: record.name.clone(),
        deployment: record.deployment,
        resources: record.resources,
        port: record.port,
        autostart: record.autostart,
        docker: record.docker.clone().unwrap_or_default(),
    }
}

/// The broadcaster sidecar spec for one record: a container only when the
/// instance runs on the docker backend AND the host can give the sidecar the
/// same network view as a native broadcaster (Linux host networking). Every
/// other combination keeps the native Java child process.
pub fn broadcaster_docker_spec(
    record: &InstanceRecord,
    config: &Config,
) -> Option<crate::broadcaster::DockerSpec> {
    (record.deployment == Deployment::Docker && cfg!(target_os = "linux")).then(|| {
        crate::broadcaster::DockerSpec {
            container: format!("{}-broadcaster", container_name(&record.id)),
            image: config.docker_image_broadcaster.clone(),
        }
    })
}

/// Build every service for one instance from its record. The default instance
/// keeps the legacy paths so pre-multi-instance data keeps working.
async fn build_instance(
    config: &Arc<Config>,
    record: InstanceRecord,
) -> anyhow::Result<Arc<Instance>> {
    let (instance_config, paths) = if record.id == DEFAULT_INSTANCE {
        (
            config.clone(),
            ServerPaths {
                settings_path: config.data_dir.join("server_settings.yaml"),
                state_dir: config.data_dir.clone(),
                journal_root: config.logs_dir.join("console"),
            },
        )
    } else {
        let root = config.data_dir.join("instances").join(&record.id);
        let mut instance_config = config.as_ref().clone();
        instance_config.bedrock_server_dir = root.join("server");
        instance_config.backups_dir = config.backups_dir.join(&record.id);
        instance_config.logs_dir = config.logs_dir.join("instances").join(&record.id);
        // Own broadcaster home per instance; the jar is shared from the
        // panel-level broadcaster dir (see Broadcaster::jar_path).
        instance_config.broadcaster_dir = root.join("broadcaster");
        let instance_config = Arc::new(instance_config);
        for dir in [
            instance_config.bedrock_server_dir.join("worlds"),
            instance_config.backups_dir.clone(),
            instance_config.logs_dir.clone(),
            root.clone(),
        ] {
            tokio::fs::create_dir_all(&dir).await?;
        }
        let settings_path = root.join("server_settings.yaml");
        let paths = ServerPaths {
            settings_path: settings_path.clone(),
            state_dir: root.clone(),
            journal_root: instance_config.logs_dir.join("console"),
        };
        // Instance-scoped backup settings; the default instance keeps the
        // legacy <data_dir>/backup_settings.yaml.
        if !settings_path.exists() {
            let _ = tokio::fs::write(
                &settings_path,
                serde_yaml::to_string(&crate::server::ServerSettings::default())?.as_bytes(),
            )
            .await;
        }
        (instance_config, paths)
    };
    let token = CancellationToken::new();
    let console = Console::with_journal_root(
        paths.journal_root.clone(),
        instance_config.console_max_lines,
        instance_config.console_max_bytes,
        instance_config.console_journal_bytes,
    );
    let server = ServerManager::with_paths(
        instance_config.clone(),
        token.clone(),
        paths,
        launch_context(&record),
    )?;
    let bds = Bds::new(instance_config.clone(), console.clone(), token.clone());
    let endstone = Endstone::new(instance_config.clone(), console.clone(), token.clone());
    // The default instance's broadcaster dir IS the panel dir, so its
    // pre-existing config.yml/cache keep working untouched.
    let broadcaster = Broadcaster::new(
        instance_config.clone(),
        token.clone(),
        (record.id != DEFAULT_INSTANCE).then(|| config.broadcaster_dir.clone()),
        broadcaster_docker_spec(&record, config),
    )?;
    let backups = if record.id == DEFAULT_INSTANCE {
        Backups::new(instance_config.clone(), server.clone(), token.clone()).await?
    } else {
        Backups::with_settings_path(
            instance_config.clone(),
            server.clone(),
            token.clone(),
            instance_config
                .data_dir
                .join("instances")
                .join(&record.id)
                .join("backup_settings.yaml"),
        )
        .await?
    };
    let instance = Arc::new(Instance {
        record: RwLock::new(record),
        config: instance_config,
        server,
        bds,
        endstone,
        broadcaster,
        backups,
        token,
        operations: Arc::new(tokio::sync::Mutex::new(())),
    });
    // A systemd unit or docker container keeps running across panel
    // restarts; reconnect console and monitoring instead of reporting the
    // instance stopped.
    if matches!(
        instance
            .record
            .read()
            .expect("instance record lock")
            .deployment,
        Deployment::Systemd | Deployment::Docker
    ) {
        match instance.server.adopt_if_active().await {
            Ok(message) if message.starts_with("Adopted") => {
                tracing::info!(instance = %instance.record.read().expect("instance record lock").id, "{message}")
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(%e, "unit adoption skipped"),
        }
    }
    Ok(instance)
}

/// The legacy server's UDP port from server.properties, or the Bedrock default.
async fn legacy_port(config: &Config) -> u16 {
    let properties = config.bedrock_server_dir.join("server.properties");
    if let Ok(text) = tokio::fs::read_to_string(&properties).await {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("server-port=")
                && let Ok(port) = value.trim().parse::<u16>()
            {
                return port;
            }
        }
    }
    19132
}

/// A fresh instance's server.properties. A Bedrock server listens on BOTH
/// stacks, and its IPv6 socket is dual-stack: `server-port` and
/// `server-portv6` must DIFFER or BDS collides with itself ("Port may be in
/// use" on a free port). An instance registered at port P therefore occupies
/// the pair P (IPv4) and P+1 (IPv6) — the same layout as a stock BDS install
/// (19132/19133). Beyond the ports, only keys whose BDS default is harmful
/// are pinned:
/// - `transport=raknet`: modern BDS defaults to nethernet, which ignores
///   `server-port` entirely and binds its own discovery port, so no client
///   could ever reach the advertised port.
/// - `allow-list=false`: BDS defaults it on, so an empty allowlist spams a
///   warning every boot and silently rejects every join.
pub fn default_properties_text(port: u16) -> String {
    format!(
        "# Generated by OmniBedrock; safe to edit by hand.\ntransport=raknet\nallow-list=false\nserver-port={port}\nserver-portv6={}\n",
        port + 1
    )
}

/// Rewrite/append both port properties in existing server.properties text,
/// preserving every other line, and add the compatibility defaults from
/// [`default_properties_text`] for keys that are entirely absent — an absent
/// key lets the BDS default win, which is exactly what breaks connections.
/// Explicit user values always win. (Pure half of `enforce_instance_port`.)
pub fn enforce_port_in_text(text: &str, port: u16) -> String {
    let mut seen_v4 = false;
    let mut seen_v6 = false;
    let mut seen_transport = false;
    let mut seen_allow_list = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("server-port=") {
                seen_v4 = true;
                format!("server-port={port}")
            } else if trimmed.starts_with("server-portv6=") {
                seen_v6 = true;
                format!("server-portv6={}", port + 1)
            } else {
                if trimmed.starts_with("transport=") {
                    seen_transport = true;
                } else if trimmed.starts_with("allow-list=") {
                    seen_allow_list = true;
                }
                line.to_string()
            }
        })
        .collect();
    if !seen_v4 {
        lines.push(format!("server-port={port}"));
    }
    if !seen_v6 {
        lines.push(format!("server-portv6={}", port + 1));
    }
    if !seen_transport {
        lines.push("transport=raknet".into());
    }
    if !seen_allow_list {
        lines.push("allow-list=false".into());
    }
    lines.join("\n") + "\n"
}

/// Do two instances' port pairs [P, P+1] overlap?
fn port_pairs_overlap(a: u16, b: u16) -> bool {
    a.abs_diff(b) <= 1
}

/// Seed a fresh instance's server.properties so its port matches the record.
async fn seed_server_properties(config: &Config, port: u16) {
    let path = config.bedrock_server_dir.join("server.properties");
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return;
    }
    let _ = tokio::fs::create_dir_all(&config.bedrock_server_dir).await;
    let _ = tokio::fs::write(&path, default_properties_text(port)).await;
}

/// First Endstone release whose stable NetherNet multiplexes every player
/// onto one forwarded port (TCP+UDP on server-port) instead of needing a
/// per-player UDP port from `server-udp-ports`.
pub const NETHERNET_MULTIPLEXER_VERSION: &str = "0.11.15";

/// True when an Endstone toolchain ships the NetherNet single-port
/// multiplexer (>= [`NETHERNET_MULTIPLEXER_VERSION`]). An unknown or empty
/// version never claims it. [`crate::endstone::version_cmp`] sorts newest
/// first, so "a is not older than the gate" reads as "not Greater".
pub fn endstone_has_multiplexer(version: &str) -> bool {
    crate::endstone::version_cmp(version, NETHERNET_MULTIPLEXER_VERSION)
        != std::cmp::Ordering::Greater
}

/// One `server-udp-ports` comma-separated entry.
#[derive(Debug, PartialEq)]
enum UdpPortEntry {
    Port(u16),
    Range(u16, u16),
    /// `[ip:]external:internal`; the ip is optional and may be IPv4, IPv6,
    /// or a hostname (everything before the last two colon groups).
    Mapping(Option<String>, u16, u16),
}

/// Parse one `server-udp-ports` entry. Syntax errors carry the reason;
/// backend/transport policy lives in [`validate_udp_ports_value`].
fn parse_udp_port_entry(entry: &str) -> Result<UdpPortEntry, String> {
    let port = |text: &str| -> Result<u16, String> {
        text.parse::<u16>()
            .map_err(|_| format!("'{text}' is not a UDP port (1-65535)"))
    };
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("empty entry".into());
    }
    if let Some((lo, hi)) = entry.split_once('-') {
        let (lo, hi) = (port(lo.trim())?, port(hi.trim())?);
        if lo > hi {
            return Err(format!("range {lo}-{hi} starts above its end"));
        }
        return Ok(UdpPortEntry::Range(lo, hi));
    }
    // `[ip:]external:internal`: split from the right so an IPv6 address
    // survives as the ip part. A range in either port slot ("ip:a-b:a-b",
    // which silently bound nothing on vanilla) fails the integer parse.
    let groups: Vec<&str> = entry.rsplit(':').collect();
    if groups.len() >= 3 {
        let internal = port(groups[0])?;
        let external = port(groups[1])?;
        let ip = groups[2..]
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join(":");
        return Ok(UdpPortEntry::Mapping(
            (!ip.is_empty()).then_some(ip),
            external,
            internal,
        ));
    }
    if groups.len() == 2 {
        let internal = port(groups[0])?;
        let external = port(groups[1])?;
        return Ok(UdpPortEntry::Mapping(None, external, internal));
    }
    Ok(UdpPortEntry::Port(port(entry)?))
}

/// Validate a `server-udp-ports` write against what the running backend
/// would actually do with it. Vanilla NetherNet binds the listed ports and
/// ranges directly (one UDP port per player slot); Endstone since 0.11.15
/// multiplexes everything onto server-port, so only the
/// `[ip:]external:internal` mapping with `internal == server-port` has any
/// effect — ranges and mismatched mappings are rejected loudly instead of
/// silently binding nothing. On `transport=raknet` the key is inert and only
/// the syntax is checked.
pub fn validate_udp_ports_value(
    backend: Backend,
    transport: &str,
    server_port: u16,
    value: &str,
) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(
            "server-udp-ports cannot be blank; delete the key instead of emptying it".into(),
        );
    }
    let nethernet = transport.eq_ignore_ascii_case("nethernet");
    for entry in value.split(',') {
        let parsed = parse_udp_port_entry(entry)
            .map_err(|e| format!("server-udp-ports entry '{}': {e}", entry.trim()))?;
        if !nethernet {
            continue;
        }
        match (backend, &parsed) {
            (Backend::Endstone, UdpPortEntry::Range(lo, hi)) => {
                return Err(format!(
                    "server-udp-ports range {lo}-{hi} does nothing on Endstone: its NetherNet \
                     multiplexer needs ONE port (TCP+UDP on server-port), not a range"
                ));
            }
            (Backend::Endstone, UdpPortEntry::Mapping(_, _, internal))
                if *internal != server_port =>
            {
                return Err(format!(
                    "server-udp-ports mapping internal port {internal} must equal server-port \
                     {server_port} — that is the one port Endstone multiplexes NetherNet onto"
                ));
            }
            (Backend::Vanilla, UdpPortEntry::Mapping(_, external, internal)) => {
                return Err(format!(
                    "server-udp-ports mapping {external}:{internal} is Endstone-only syntax: \
                     vanilla BDS binds the listed ports directly, and the ip:a-b:a-b range \
                     form silently binds nothing"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Launch-time transport guidance. The old blanket warning ("nethernet
/// ignores server-port") predates the Endstone single-port multiplexer and
/// is now wrong exactly when it mattered most: since Endstone 0.11.15
/// server-port IS the port to forward (TCP and UDP). Each backend/transport
/// combination gets the one line that is true for it.
pub async fn advise_transport(
    config: &Config,
    console: &Console,
    backend: Backend,
    endstone_version: Option<&str>,
) {
    let Ok(text) =
        tokio::fs::read_to_string(config.bedrock_server_dir.join("server.properties")).await
    else {
        return;
    };
    let mut transport = String::from("nethernet");
    let mut port: u16 = 19132;
    let mut max_players: u16 = 10;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("transport=") {
            let v = v.trim();
            if !v.is_empty() {
                transport = v.to_string();
            }
        } else if let Some(v) = line.strip_prefix("server-port=")
            && let Ok(p) = v.trim().parse()
        {
            port = p;
        } else if let Some(v) = line.strip_prefix("max-players=")
            && let Ok(m) = v.trim().parse()
        {
            max_players = m;
        }
    }
    let message = if transport.eq_ignore_ascii_case("raknet") {
        format!(
            "transport=raknet: fully updated Bedrock clients (1.26.60+) removed RakNet and \
             CANNOT join — forward UDP {port} only, and switch transport=nethernet in the \
             Properties editor unless staying on RakNet for older clients is deliberate"
        )
    } else if transport.eq_ignore_ascii_case("nethernet") {
        match backend {
            Backend::Endstone if endstone_version.is_some_and(endstone_has_multiplexer) => format!(
                "NetherNet single-port mode (Endstone {}): forward ONE port on the router — \
                 TCP AND UDP on {port} (server-port). No per-player ports needed.",
                endstone_version.unwrap_or(NETHERNET_MULTIPLEXER_VERSION),
            ),
            Backend::Endstone => format!(
                "transport=nethernet on Endstone {}: this toolchain has no NetherNet \
                 multiplexer, so game traffic needs one UDP port per player slot from \
                 server-udp-ports and ignores server-port — upgrade the Endstone toolchain \
                 to {NETHERNET_MULTIPLEXER_VERSION}+ for single-port mode",
                endstone_version.unwrap_or("unknown"),
            ),
            Backend::Vanilla => format!(
                "transport=nethernet on vanilla BDS: TCP {port} is signaling ONLY and game \
                 traffic needs one UDP port per player slot from server-udp-ports \
                 ({max_players} players = {max_players} ports) — home routers usually cannot \
                 forward port ranges. The Endstone backend multiplexes NetherNet onto one \
                 TCP+UDP port instead"
            ),
        }
    } else {
        format!(
            "transport={transport} is not a known value (raknet or nethernet); the server \
             will fall back to its default and clients may not reach it"
        )
    };
    console
        .publish(format!("NOTE: {message}"), "panel", false)
        .await;
}

/// Keep server.properties in sync with the configured port: both stacks.
pub async fn enforce_instance_port(config: &Config, port: u16) {
    let path = config.bedrock_server_dir.join("server.properties");
    let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let _ = atomic_write(&path, enforce_port_in_text(&text, port).as_bytes()).await;
}

/// A port is allocatable only when BOTH stacks can bind it. BDS holds IPv6
/// sockets (`server-portv6`), so an IPv4-only probe would happily hand out a
/// port another instance is already listening on over IPv6. Hosts without
/// any IPv6 (the loopback probe fails) fall back to the IPv4 verdict.
pub(crate) async fn port_bindable(port: u16) -> bool {
    if tokio::net::UdpSocket::bind(("0.0.0.0", port))
        .await
        .is_err()
    {
        return false;
    }
    match tokio::net::UdpSocket::bind(("::", port)).await {
        Ok(_) => true,
        // Bare address, no brackets: tuple ("[::1]", 0) would be treated as
        // a hostname and fail name lookup, breaking the fallback.
        Err(_) => tokio::net::UdpSocket::bind(("::1", 0u16)).await.is_err(),
    }
}

/// Docker container name for an instance id.
pub fn container_name(id: &str) -> String {
    format!("omnibedrock-{id}")
}

/// Judge aggregate instance allocations against host capacity minus the
/// configured reserve. Pure so the exact warning logic is unit-testable.
/// Returns (memory_warning, cpu_warning); each is None when healthy.
pub fn overcommit_status(
    host_memory_mb: f64,
    reserve_mb: u64,
    allocated_memory_mb: u64,
    cpu_count: usize,
    allocated_cpus: f64,
) -> (Option<String>, Option<String>) {
    let safe_memory = (host_memory_mb - reserve_mb as f64).max(0.0);
    let memory = if allocated_memory_mb as f64 > safe_memory {
        Some(format!(
            "allocated {allocated_memory_mb} MiB exceeds the safe budget of {:.0} MiB              (host {:.0} MiB − {reserve_mb} MiB reserve); instances may contend or hit OOM",
            safe_memory, host_memory_mb
        ))
    } else {
        None
    };
    let cpu = if allocated_cpus > cpu_count as f64 + 1e-9 {
        Some(format!(
            "allocated {allocated_cpus:.2} CPU across instances exceeds the host's              {cpu_count} cores; CPU limits are quotas and will throttle under load"
        ))
    } else {
        None
    };
    (memory, cpu)
}

/// Probe which deployment backends this host can actually use.
async fn probe_capabilities() -> DeployCapabilities {
    let docker = probe_docker().await;
    let systemd = if cfg!(unix) {
        crate::systemd::user_manager_available().await
    } else {
        false
    };
    let linger = systemd && crate::systemd::linger_enabled().await;
    DeployCapabilities {
        docker,
        systemd,
        linger,
        platform: std::env::consts::OS.into(),
    }
}

async fn probe_docker() -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("docker")
            .arg("version")
            .arg("--format")
            .arg("{{.Server.Version}}")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_policy_validation() {
        assert!(
            ResourcePolicy {
                memory_mb: Some(128),
                cpus: Some(0.1)
            }
            .validate()
            .is_ok()
        );
        assert!(
            ResourcePolicy {
                memory_mb: Some(64),
                cpus: None
            }
            .validate()
            .is_err()
        );
        assert!(
            ResourcePolicy {
                memory_mb: None,
                cpus: Some(0.01)
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn quota_check_sums_owned_instances() {
        let quotas = UserQuotas {
            max_instances: Some(2),
            memory_mb: Some(2048),
            cpus: Some(4.0),
        };
        let owned = [
            InstanceRecord {
                id: "a".into(),
                name: "A".into(),
                owner: "u".into(),
                created_at: String::new(),
                port: 1,
                deployment: Deployment::Native,
                docker: None,
                resources: ResourcePolicy {
                    memory_mb: Some(1024),
                    cpus: Some(2.0),
                },
                access: vec![],
                autostart: false,
            },
            InstanceRecord {
                id: "b".into(),
                name: "B".into(),
                owner: "u".into(),
                created_at: String::new(),
                port: 2,
                deployment: Deployment::Native,
                docker: None,
                resources: ResourcePolicy {
                    memory_mb: Some(512),
                    cpus: Some(1.0),
                },
                access: vec![],
                autostart: false,
            },
        ];
        let refs: Vec<&InstanceRecord> = owned.iter().collect();
        // Third instance exceeds the count quota.
        assert!(quotas.check(&refs, ResourcePolicy::default()).is_err());
        let quotas = UserQuotas {
            max_instances: None,
            memory_mb: Some(2048),
            cpus: Some(4.0),
        };
        // 1536 + 1024 > 2048 memory.
        assert!(
            quotas
                .check(
                    &refs,
                    ResourcePolicy {
                        memory_mb: Some(1024),
                        cpus: None
                    }
                )
                .is_err()
        );
        // Unlimited extras (None) cannot be summed, so they are rejected when
        // any quota is set: an unlimited instance against a finite budget is
        // nonsensical.
        let single = [owned[0].clone()];
        let single_refs: Vec<&InstanceRecord> = single.iter().collect();
        assert!(
            UserQuotas {
                max_instances: None,
                memory_mb: Some(2048),
                cpus: None
            }
            .check(&single_refs, ResourcePolicy::default())
            .is_ok()
        );
    }

    #[test]
    fn slug_and_id_rules() {
        assert_eq!(slug("Dad's Server!"), "dad-s-server");
        assert!(valid_id("dad-s-server"));
        assert!(!valid_id("Dad"));
        assert!(!valid_id(""));
        assert!(!valid_id("-x"));
        assert!(!valid_id("x-"));
    }

    #[test]
    fn deployment_parse_and_enforcement() {
        assert_eq!(Deployment::parse("docker").unwrap(), Deployment::Docker);
        assert_eq!(Deployment::parse("SYSTEMD").unwrap(), Deployment::Systemd);
        assert!(Deployment::parse("podman").is_err());
        assert_eq!(Deployment::Native.enforcement(), "advisory");
        assert_eq!(Deployment::Systemd.enforcement(), "cgroup");
        assert_eq!(Deployment::Docker.enforcement(), "container");
    }

    #[test]
    fn record_access_rules() {
        let record = InstanceRecord {
            id: "x".into(),
            name: "X".into(),
            owner: "me".into(),
            created_at: String::new(),
            port: 1,
            deployment: Deployment::Native,
            docker: None,
            resources: ResourcePolicy::default(),
            access: vec![
                AccessEntry {
                    user: "friend".into(),
                    roles: vec!["instance-manager".into()],
                    level: None,
                },
                AccessEntry {
                    user: "dad".into(),
                    roles: vec!["viewer".into()],
                    level: None,
                },
            ],
            autostart: false,
        };
        assert_eq!(
            record.access_entry("friend").map(|a| a.roles.as_slice()),
            Some(&["instance-manager".to_string()][..])
        );
        assert!(record.access_entry("dad").is_some());
        assert!(record.access_entry("stranger").is_none());
    }

    #[test]
    fn legacy_access_levels_migrate_to_roles() {
        // Files written before the unified role system carry a two-level
        // `level` field; loading maps manage/view onto catalog roles.
        let legacy: InstanceRecord = serde_yaml::from_str(
            "id: x\nname: X\nowner: me\ncreated_at: ''\naccess:\n  - user: friend\n    level: manage\n  - user: dad\n    level: view\n",
        )
        .unwrap();
        assert!(legacy.access.iter().any(|a| a.level.is_some()));
        let record = legacy.normalized();
        assert!(record.access.iter().all(|a| a.level.is_none()));
        let friend = record.access_entry("friend").unwrap();
        assert_eq!(friend.roles, vec!["instance-manager".to_string()]);
        let dad = record.access_entry("dad").unwrap();
        assert_eq!(dad.roles, vec!["viewer".to_string()]);
        // Round-trip: saved files no longer contain the legacy field.
        let text = serde_yaml::to_string(&record).unwrap();
        assert!(!text.contains("level:"));
    }

    #[test]
    fn port_properties_rewrite_preserves_other_lines() {
        // BDS's dual-stack v6 socket requires DIFFERENT v4/v6 ports; an
        // instance at P occupies the pair P (v4) and P+1 (v6).
        let fresh = default_properties_text(19140);
        assert_eq!(
            fresh,
            "# Generated by OmniBedrock; safe to edit by hand.\ntransport=raknet\nallow-list=false\nserver-port=19140\nserver-portv6=19141\n"
        );
        // Existing file: both keys updated, everything else untouched.
        let existing = "level-name=old\nserver-port=19132\nserver-portv6=19133\nview-distance=32\n";
        let updated = enforce_port_in_text(existing, 19140);
        assert!(updated.contains("server-port=19140\n"));
        assert!(updated.contains("server-portv6=19141\n"));
        assert!(!updated.contains("19132"));
        assert!(!updated.contains("19133"));
        assert!(updated.contains("level-name=old"));
        assert!(updated.contains("view-distance=32"));
        // Missing keys are appended, not dropped.
        let partial = enforce_port_in_text("level-name=x\n", 25565);
        assert!(partial.contains("server-port=25565"));
        assert!(partial.contains("server-portv6=25566"));
        assert!(partial.contains("transport=raknet"));
        assert!(partial.contains("allow-list=false"));
        // A deliberately chosen transport or allow-list survives enforcement.
        let chosen = enforce_port_in_text("transport=nethernet\nallow-list=true\n", 25565);
        assert!(chosen.contains("transport=nethernet"));
        assert!(chosen.contains("allow-list=true"));
    }

    #[test]
    fn port_pairs_never_overlap() {
        assert!(port_pairs_overlap(19132, 19132));
        assert!(port_pairs_overlap(19132, 19133));
        assert!(port_pairs_overlap(19133, 19132));
        assert!(!port_pairs_overlap(19132, 19134));
        assert!(!port_pairs_overlap(19140, 19132));
    }

    #[tokio::test]
    async fn transport_guidance_matches_backend_and_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            bedrock_server_dir: dir.path().join("server"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.bedrock_server_dir).unwrap();
        let console = Console::with_journal_root(dir.path().join("journal"), 128, 65536, 1 << 20);
        // No properties file yet: no noise.
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        assert!(console.tail(10).is_empty());
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=raknet\nserver-port=19140\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        // Each publish appends; tail is cumulative, so check the newest line.
        let tail = console.tail(10);
        assert_eq!(tail.len(), 1, "{:?}", tail);
        assert!(tail[0].line.contains("raknet"), "{}", tail[0].line);
        assert!(tail[0].line.contains("CANNOT join"), "{}", tail[0].line);
        // Endstone 0.11.15 + nethernet: the single-port recipe, not a warning.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=nethernet\nserver-port=24474\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 2, "{:?}", tail);
        assert!(
            tail[1].line.contains("single-port") && tail[1].line.contains("24474"),
            "{}",
            tail[1].line
        );
        assert!(
            !tail[1].line.contains("ignores server-port"),
            "{}",
            tail[1].line
        );
        // Endstone 0.11.14 + nethernet: the multiplexer is not there yet.
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.14")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 3, "{:?}", tail);
        assert!(
            tail[2].line.contains("no NetherNet multiplexer"),
            "{}",
            tail[2].line
        );
        // Vanilla + nethernet: per-player ports warning with the max count.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=nethernet\nserver-port=19140\nmax-players=12\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Vanilla, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 4, "{:?}", tail);
        assert!(
            tail[3].line.contains("signaling ONLY") && tail[3].line.contains("12"),
            "{}",
            tail[3].line
        );
        // An unknown transport value is called out, never silently accepted.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=wat\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 5, "{:?}", tail);
        assert!(tail[4].line.contains("transport=wat"), "{}", tail[4].line);
    }

    #[test]
    fn udp_ports_validation_follows_backend_matrix() {
        use validate_udp_ports_value as v;
        // Syntax errors are backend-independent.
        assert!(v(Backend::Endstone, "nethernet", 24474, "").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "  ").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474,").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474-24470").is_err());
        // The 3-part RANGE form binds nothing on any backend: hard reject.
        assert!(
            v(
                Backend::Vanilla,
                "nethernet",
                19132,
                "1.2.3.4:19132-19134:19132-19134"
            )
            .is_err()
        );
        // Endstone nethernet: single mapping (with optional ip), no ranges.
        // internal must equal server-port; the advertised external may differ.
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "1.2.3.4:20000:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "[::1]:24474:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "20000-20100").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "1.2.3.4:24474:20000").is_err());
        // Bare ports are a harmless 0.11.11-era artifact.
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474").is_ok());
        // Vanilla nethernet: ports and ranges bind directly; the mapping
        // form is Endstone-only syntax.
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132-19142").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132,19134-19136").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132:19132").is_err());
        // On raknet the key is inert — only the syntax is checked.
        assert!(v(Backend::Endstone, "raknet", 19132, "20000-20100").is_ok());
        assert!(v(Backend::Vanilla, "raknet", 19132, "1.2.3.4:1:2").is_ok());
    }

    #[test]
    fn endstone_multiplexer_version_gate() {
        assert!(endstone_has_multiplexer("0.11.15"));
        assert!(endstone_has_multiplexer("0.11.16"));
        assert!(endstone_has_multiplexer("0.12.0"));
        assert!(!endstone_has_multiplexer("0.11.14"));
        assert!(!endstone_has_multiplexer("0.9.3"));
        assert!(!endstone_has_multiplexer(""));
        assert!(!endstone_has_multiplexer("unknown"));
    }

    #[test]
    fn overcommit_warnings_respect_reserve() {
        // 8192 total − 1024 reserve = 7168 safe; 7000 fits, 8000 warns.
        let (none, cpu_none) = overcommit_status(8192.0, 1024, 7000, 8, 7.5);
        assert!(none.is_none() && cpu_none.is_none());
        let (warn, _) = overcommit_status(8192.0, 1024, 8000, 8, 7.5);
        assert!(warn.unwrap().contains("exceeds the safe budget"));
        // Reserve larger than the host: everything warns, never a negative budget.
        let (warn, _) = overcommit_status(512.0, 4096, 128, 4, 0.5);
        assert!(warn.unwrap().contains("exceeds the safe budget"));
        // CPU overcommit is independent of memory health.
        let (_, cpu) = overcommit_status(65536.0, 1024, 1024, 4, 6.0);
        assert!(cpu.unwrap().contains("exceeds the host's"));
    }

    #[test]
    fn docker_config_validation() {
        let ok = DockerConfig {
            image: Some("omnibedrock-vanilla:1.21".into()),
            extra_mounts: vec!["/a:/b:ro".into(), "/c:/d".into()],
        };
        assert!(ok.validate().is_ok());
        assert!(DockerConfig::default().validate().is_ok());
        // Image must be a single reference.
        assert!(
            DockerConfig {
                image: Some("two words".into()),
                extra_mounts: vec![]
            }
            .validate()
            .is_err()
        );
        // Mounts need host:container, no whitespace, absolute container path.
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["/a:b c".into()]
            }
            .validate()
            .is_err()
        );
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["onlyhost".into()]
            }
            .validate()
            .is_err()
        );
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["/a:relative".into()]
            }
            .validate()
            .is_err()
        );
        // The per-instance override beats the config default per runtime.
        let config = Config::default();
        assert_eq!(
            ok.image_for(crate::server::Backend::Vanilla, &config),
            "omnibedrock-vanilla:1.21"
        );
        assert_eq!(
            DockerConfig::default().image_for(crate::server::Backend::Endstone, &config),
            config.docker_image_endstone
        );
    }

    #[test]
    fn record_serde_docker_field_is_optional() {
        let record: InstanceRecord = serde_yaml::from_str(
            "id: a\nname: A\nowner: o\ncreated_at: 'x'\ndeployment: docker\ndocker:\n  image: omnibedrock-vanilla:1\n  extra_mounts:\n    - /a:/b:ro\n",
        )
        .unwrap();
        let docker = record.docker.expect("docker options parsed");
        assert_eq!(docker.image.as_deref(), Some("omnibedrock-vanilla:1"));
        assert_eq!(docker.extra_mounts, vec!["/a:/b:ro".to_string()]);
        // Records written before the docker backend keep deserializing.
        let legacy: InstanceRecord =
            serde_yaml::from_str("id: a\nname: A\nowner: o\ncreated_at: 'x'\n").unwrap();
        assert!(legacy.docker.is_none());
        assert_eq!(legacy.deployment, Deployment::Native);
    }

    #[test]
    fn record_serde_roundtrip_with_defaults() {
        // Older instances.yaml entries lack deployment/resources; they must
        // still deserialize as native/unlimited.
        let record: InstanceRecord =
            serde_yaml::from_str("id: a\nname: A\nowner: o\ncreated_at: '2026-01-01T00:00:00Z'\n")
                .unwrap();
        assert_eq!(record.deployment, Deployment::Native);
        assert_eq!(record.port, 19132);
        assert!(record.resources.is_unlimited());
    }
}
