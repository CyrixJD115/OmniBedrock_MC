//! The instance registry: `InstanceManager` owns every instance, persists
//! `instances.yaml`, and performs create/configure/remove under a single
//! write lock (quota, id, and port checks all run inside that lock).

use super::{
    builder::{broadcaster_docker_spec, build_instance, launch_context},
    capabilities::DeployCapabilities,
    capabilities::probe_capabilities,
    consts::DEFAULT_INSTANCE,
    helpers::container_name,
    instance::Instance,
    ports::{
        enforce_instance_port, legacy_port, port_bindable, port_pairs_overlap,
        seed_server_properties,
    },
    types::{Deployment, InstanceRecord, ResourcePolicy},
};
use crate::server::{Backend, Status};
use anyhow::{Context, bail};
use omnibedrock_core::{config::Config, files::atomic_write};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock as AsyncRwLock;

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Owns every instance, persists `instances.yaml`, and creates/removes
/// instances under a single write lock (quota checks run inside that lock).
pub struct InstanceManager {
    config: Arc<Config>,
    instances: AsyncRwLock<BTreeMap<String, Arc<Instance>>>,
    capabilities: DeployCapabilities,
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

    /// Create an instance. `backend` seeds the NetherNet-aware defaults in
    /// server.properties (the per-instance settings file is written by the
    /// API only after this returns). `quota_check` runs while the write lock
    /// is held so concurrent creates cannot both pass the same quota.
    pub async fn create(
        &self,
        mut record: InstanceRecord,
        backend: Backend,
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
        seed_server_properties(&instance.config, port, backend).await;
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
        {
            let instances = self.instances.write().await;
            // Quota and port checks run under the write lock against the
            // projected post-change record set (the same discipline as
            // `create`), so two concurrent edits cannot both pass with the
            // same port; the stored record is only touched once they pass.
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
            if next.port != current.port {
                self.ensure_port_free(next.port, Some(id), &projected)
                    .await?;
            }
            *instance.record.write().expect("instance record lock") = next.clone();
            instance.server.set_launch(launch_context(&next));
            instance
                .broadcaster
                .set_docker(broadcaster_docker_spec(&next, &self.config));
            if next.port != current.port {
                let backend = instance.server.settings.borrow().backend;
                enforce_instance_port(&instance.config, next.port, backend).await;
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
