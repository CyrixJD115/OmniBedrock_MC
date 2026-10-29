//! Shared application state: service handles, instance resolution, and the
//! audit log.
use crate::auth::{Auth, User};
use omnibedrock_core::{
    config::Config,
    error::{ApiError, Result},
    files::atomic_write,
};
use omnibedrock_server::{
    curseforge::CurseForge,
    endstone::Endstone,
    instances::{DEFAULT_INSTANCE, Instance, InstanceManager},
    metrics::Metrics,
    server::ServerManager,
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    /// Every managed instance, including the migrated legacy `default`.
    pub instances: Arc<InstanceManager>,
    /// Handles to the default instance's services; kept for the single-server
    /// code paths and tests. Instance-scoped requests resolve through
    /// `resolve_instance` instead.
    pub server: ServerManager,
    pub auth: Auth,
    pub backups: omnibedrock_server::backups::Backups,
    pub metrics: Metrics,
    pub bds: omnibedrock_server::bds::Bds,
    pub endstone: Endstone,
    pub curseforge: CurseForge,
    pub addon_meta: omnibedrock_server::addon_cache::AddonMetaCache,
    pub shutdown: CancellationToken,
    pub operations: Arc<Mutex<()>>,
    pub edits: Arc<Mutex<()>>,
    pub(super) audit_entries: Arc<Mutex<VecDeque<Value>>>,
}
impl AppState {
    /// Bring up every service, logging each stage so slow boots are visible.
    #[allow(clippy::too_many_lines)]
    pub async fn new(config: Config, shutdown: CancellationToken) -> anyhow::Result<Self> {
        let started = std::time::Instant::now();
        let config = Arc::new(config);
        tracing::info!(
            data_dir = %config.data_dir.display(),
            logs_dir = %config.logs_dir.display(),
            backups_dir = %config.backups_dir.display(),
            "creating data directories"
        );
        let worlds_dir = config.bedrock_server_dir.join("worlds");
        let image_cache_dir = config.cache_dir.join("curseforge-images");
        futures_util::future::try_join_all([
            tokio::fs::create_dir_all(&config.data_dir),
            tokio::fs::create_dir_all(&config.logs_dir),
            tokio::fs::create_dir_all(&config.backups_dir),
            tokio::fs::create_dir_all(&worlds_dir),
            tokio::fs::create_dir_all(&image_cache_dir),
        ])
        .await?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "loading users, roles, and the signing key"
        );
        let auth = Auth::new(config.clone()).await?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            accounts = auth.users().await.as_array().map_or(0, Vec::len),
            login_enabled = auth.login_enabled(),
            "authentication ready"
        );
        // The legacy single server migrates into the `default` instance; its
        // services keep every legacy path so existing installs are unaffected.
        let instances = Arc::new(
            InstanceManager::new(config.clone(), auth.first_owner_username().await).await?,
        );
        let default = instances
            .get(DEFAULT_INSTANCE)
            .await
            .ok_or_else(|| anyhow::anyhow!("default instance missing after migration"))?;
        let server = default.server.clone();
        let bds = default.bds.clone();
        let endstone = default.endstone.clone();
        // Detected facts from the on-disk toolchain markers, not assumptions:
        // what each backend would actually launch with right now.
        let toolchain = endstone.installed().await;
        let bds_installed = bds.installed_version().await;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            endstone_launch = ?omnibedrock_server::endstone::resolve_launch(&config),
            endstone_toolchain = toolchain
                .as_ref()
                .and_then(|t| t["endstone_version"].as_str())
                .unwrap_or("not installed"),
            bds_version = bds_installed
                .as_ref()
                .and_then(|v| v["version"].as_str())
                .unwrap_or("not installed"),
            "toolchain status"
        );
        let curseforge = CurseForge::new(
            config.curseforge_api_key.expose_secret(),
            Some(image_cache_dir),
        );
        let addon_meta = omnibedrock_server::addon_cache::AddonMetaCache::new(&config.cache_dir);
        // A key saved from the Settings page overrides the .env value.
        if let Some(stored) = auth.curseforge_key_override().await {
            curseforge.set_api_key(&stored);
        }
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            key_configured = curseforge.configured(),
            "curseforge store ready"
        );
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            broadcaster_dir = %config.broadcaster_dir.display(),
            "broadcaster jar home ready (each instance runs its own broadcaster; Java probe runs per instance)"
        );
        let backups = default.backups.clone();
        let metrics = Metrics::start(instances.clone(), config.clone(), shutdown.clone());
        // Every instance runs its own backup scheduler against its own settings.
        for instance in instances.list().await {
            if instance.record().id != DEFAULT_INSTANCE {
                instance
                    .backups
                    .start_scheduler(instance.operations.clone());
            }
        }
        let audit = match tokio::fs::read_to_string(config.logs_dir.join("audit.jsonl")).await {
            Ok(s) => s
                .lines()
                .filter_map(|s| serde_json::from_str::<Value>(s).ok())
                .collect::<VecDeque<_>>(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(e) => return Err(e.into()),
        };
        let state = Self {
            config,
            instances,
            server,
            auth,
            backups,
            metrics,
            bds,
            endstone,
            curseforge,
            addon_meta,
            shutdown,
            operations: Arc::new(Mutex::new(())),
            edits: Arc::new(Mutex::new(())),
            audit_entries: Arc::new(Mutex::new(audit)),
        };
        default.backups.start_scheduler(default.operations.clone());
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "all services initialized"
        );
        Ok(state)
    }
    /// Resolve the instance a request targets: the `X-Instance` header value,
    /// or the migrated default. Access is checked at view level here; write
    /// endpoints re-check manage access individually.
    /// Instance-scoped permission gate for one resolved instance.
    pub async fn require_for(&self, u: &User, permission: &str, svc: &Instance) -> Result<()> {
        self.auth
            .require_instance(u, permission, &svc.record())
            .await
    }
    pub async fn resolve_instance(&self, id: Option<&str>, user: &User) -> Result<Arc<Instance>> {
        let id = id.unwrap_or(DEFAULT_INSTANCE);
        let instance = self
            .instances
            .get(id)
            .await
            .ok_or_else(|| ApiError::missing(format!("Instance '{id}' not found")))?;
        let record = instance.record();
        // The migrated default instance keeps the legacy panel's model: every
        // authenticated user reaches it and role permissions gate each
        // endpoint. Instances created afterwards are reachable by their owner,
        // users with an access entry, the panel owner, and holders of the
        // global INSTANCES_VIEW_ALL capability.
        if record.id != DEFAULT_INSTANCE && !self.auth.can_access_instance(user, &record).await {
            // Do not reveal whether the instance exists.
            return Err(ApiError::missing(format!("Instance '{id}' not found")));
        }
        Ok(instance)
    }
    /// Latest metrics sample for one instance in the legacy single-server shape.
    pub fn instance_metrics(&self, id: &str) -> Value {
        if id == DEFAULT_INSTANCE {
            return self.metrics.latest.borrow().clone();
        }
        self.metrics.all.borrow()["instances"]
            .get(id)
            .cloned()
            .unwrap_or(Value::Null)
    }
    pub async fn audit(
        &self,
        user: &User,
        action: &str,
        detail: &str,
        category: &str,
    ) -> Result<()> {
        self.audit_instance(user, action, detail, category, "")
            .await
    }
    pub async fn audit_instance(
        &self,
        user: &User,
        action: &str,
        detail: &str,
        category: &str,
        instance: &str,
    ) -> Result<()> {
        self.audit_raw(&user.username, action, detail, category, instance, "")
            .await
    }
    /// Audit entry with the client address, for request paths (login, the
    /// generic mutation trail) that actually observed one.
    pub async fn audit_raw(
        &self,
        username: &str,
        action: &str,
        detail: &str,
        category: &str,
        instance: &str,
        ip: &str,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let entry = json!({"timestamp":chrono::Utc::now().to_rfc3339(),"username":username,"action":action,"detail":detail,"category":category,"instance":instance,"ip":ip});
        let mut entries = self.audit_entries.lock().await;
        let path = self.config.logs_dir.join("audit.jsonl");
        let mut text = serde_json::to_vec(&entry).map_err(anyhow::Error::from)?;
        text.push(b'\n');
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        f.write_all(&text).await?;
        f.flush().await?;
        entries.push_back(entry);
        if entries.len() > 10000 {
            while entries.len() > 10000 {
                entries.pop_front();
            }
            let mut text = Vec::new();
            for e in entries.iter() {
                serde_json::to_writer(&mut text, e).map_err(anyhow::Error::from)?;
                text.push(b'\n');
            }
            drop(f);
            atomic_write(&path, &text).await?;
        }
        Ok(())
    }
}
