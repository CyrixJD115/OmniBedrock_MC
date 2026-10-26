//! API v1 compatibility layer. Services own process, auth, backup and filesystem logic.
use crate::{
    auth::{Auth, INSTANCE_PERMISSIONS, User},
    websocket,
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use omnibedrock_core::{
    config::Config,
    error::{ApiError, Result},
    files::{self, atomic_write, component, safe_path},
};
use omnibedrock_server::{
    addons::{self, ArchiveKind, InstalledPack},
    backups::{BackupRequest, Hook, Hooks},
    bds,
    curseforge::{self, CLASSES, CurseForge},
    endstone::Endstone,
    instances::{
        AccessEntry, DEFAULT_INSTANCE, Deployment, Instance, InstanceManager, InstanceRecord,
        ResourcePolicy,
    },
    metrics::Metrics,
    server::{Backend, ServerManager, ServerSettings, Status},
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_util::{io::ReaderStream, sync::CancellationToken};
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

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
    audit_entries: Arc<Mutex<VecDeque<Value>>>,
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
        use tokio::io::AsyncWriteExt;
        let entry = json!({"timestamp":chrono::Utc::now().to_rfc3339(),"username":user.username,"action":action,"detail":detail,"category":category,"instance":instance,"ip":""});
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
pub fn router(state: AppState) -> anyhow::Result<Router> {
    let origins = state
        .config
        .cors_origins
        .iter()
        .map(|s| s.parse::<axum::http::HeaderValue>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let api = Router::new()
        // Machine-readable endpoint inventory, public like /health so tools
        // and agents can discover the API without a source dive.
        .route("/openapi.json", get(|| async { openapi_doc() }))
        .route("/console/ws", get(websocket::console_upgrade))
        .route("/performance/ws", get(websocket::metrics_upgrade))
        .route("/backups/ws", get(websocket::backups_upgrade))
        .route("/logs/ws", get(websocket::logs_upgrade))
        .route("/broadcaster/ws", get(websocket::broadcaster_upgrade))
        .route("/{*path}", any(dispatch));
    let prefix = state.config.api_prefix.clone();
    Ok(Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","version":env!("CARGO_PKG_VERSION")})) }),
        )
        .nest(&prefix, api)
        // Everything that is not the API serves the embedded panel UI.
        .fallback(crate::bundle::serve)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request| tracing::info_span!("http", method=%request.method(), path=request.uri().path())))
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .allow_credentials(true),
        )
        .with_state(state))
}
fn value_string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| ApiError::bad(format!("'{key}' must be a string")))
}
fn optional_string<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v[key].as_str().unwrap_or(default)
}
fn data_response(value: Value) -> Response {
    Json(value).into_response()
}
/// The static endpoint inventory served at `{prefix}/openapi.json`. Kept as a
/// checked-in document (not generated) so each entry can carry the real
/// permission gate and body keys; update it alongside `dispatch_authenticated`.
fn openapi_doc() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        include_str!("../openapi.json"),
    )
        .into_response()
}
fn success(message: impl Into<String>) -> Value {
    json!({"success":true,"message":message.into()})
}
async fn dispatch(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    request: Request,
) -> Result<Response> {
    if state.shutdown.is_cancelled() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Panel is shutting down",
        ));
    }
    let method = request.method().clone();
    let path = path.trim_matches('/');
    let segments = path.split('/').collect::<Vec<_>>();
    // Optional instance scoping for every endpoint; absent means the default.
    // Downloads pass it as a query param because browsers cannot set headers.
    let instance_id = request
        .headers()
        .get("x-instance")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| query.get("instance").cloned());
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned);
    // Addon archives stream straight to a temp file so large .mcpack/.mcaddon
    // uploads never sit whole in memory.
    if path == "addons/upload" && method == Method::POST {
        let user = state.auth.resolve_user(header_token.as_deref()).await?;
        state.auth.require(&user, "ADDONS_MANAGE").await?;
        let svc = state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?;
        return upload_addons(state, svc, user, query, request.into_body()).await;
    }
    let bytes = to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| ApiError::bad("Request body too large"))?;
    // Screenshot uploads are raw image bytes, not the JSON envelope every other
    // endpoint uses, so they are handled before the generic body parse.
    if path == "broadcaster/screenshot" && method == Method::PUT {
        let user = state.auth.resolve_user(header_token.as_deref()).await?;
        let svc = state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?;
        state.require_for(&user, "BROADCASTER_MANAGE", &svc).await?;
        let account = query.get("account").map_or(Ok("main"), |a| component(a))?;
        if bytes.is_empty() {
            return Err(ApiError::bad("Screenshot body is required"));
        }
        let jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
        let png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        if !jpeg && !png {
            return Err(ApiError::bad("Screenshot must be a JPEG or PNG image"));
        }
        let destination = svc.broadcaster.screenshot_path(account);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        atomic_write(&destination, &bytes).await?;
        state
            .audit_instance(
                &user,
                "broadcaster.screenshot_update",
                account,
                "broadcaster",
                &svc.record().id,
            )
            .await
            .ok();
        return Ok(data_response(success(
            "Screenshot uploaded; Xbox may take a few minutes to refresh it",
        )));
    }
    let body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).map_err(|e| ApiError::bad(format!("Invalid JSON: {e}")))?
    };
    if path == "auth/login" && method == Method::POST {
        return Ok(data_response(
            state
                .auth
                .authenticate(
                    value_string(&body, "username")?.into(),
                    value_string(&body, "password")?.into(),
                )
                .await?,
        ));
    }
    // Public, unauthenticated: the login screen needs this before it can render.
    if path == "settings/panel" && method == Method::GET {
        return Ok(data_response(state.auth.panel_info()));
    }
    let query_allowed = method == Method::GET
        && ((segments.first() == Some(&"backups") && segments.last() == Some(&"download"))
            || path == "broadcaster/screenshot"
            || path == "addons/icon"
            || path == "store/image");
    let token = header_token.or_else(|| {
        if query_allowed {
            query.get("token").cloned()
        } else {
            None
        }
    });
    let user = state.auth.resolve_user(token.as_deref()).await?;
    let svc = state
        .resolve_instance(instance_id.as_deref(), &user)
        .await?;
    let result =
        dispatch_authenticated(&state, &svc, &user, &method, &segments, &query, &body).await?;
    // Never include passwords, JWTs, or full edited documents in the audit trail.
    if method != Method::GET && result.status().is_success() {
        let (action, detail, category) = audit_action(&segments, &method, &body);
        if let Err(e) = state
            .audit_instance(
                &user,
                &action,
                &detail,
                category,
                instance_id.as_deref().unwrap_or_default(),
            )
            .await
        {
            tracing::error!(%e,"Action succeeded but audit persistence failed");
        }
    }
    Ok(result)
}
fn audit_action(p: &[&str], method: &Method, b: &Value) -> (String, String, &'static str) {
    let detail = p.join("/");
    match p {
        ["server", "action"] => (
            format!("server:{}", optional_string(b, "action", "unknown")),
            detail,
            "server",
        ),
        ["server", "backend", "install"] => (
            "server.bds_install".into(),
            optional_string(b, "version", "latest").into(),
            "server",
        ),
        ["server", "backend", "endstone", "install"] => (
            "server.endstone_install".into(),
            optional_string(b, "version", "latest").into(),
            "server",
        ),
        ["server", "backend", "switch"] => (
            format!(
                "server.backend:{}",
                optional_string(b, "backend", "unknown")
            ),
            optional_string(b, "version", "").into(),
            "server",
        ),
        ["broadcaster", "action"] => (
            format!("broadcaster:{}", optional_string(b, "action", "unknown")),
            detail,
            "broadcaster",
        ),
        ["broadcaster", "command"] => (
            "broadcaster.command".into(),
            optional_string(b, "command", "").into(),
            "broadcaster",
        ),
        ["broadcaster", "accounts"] => (
            format!(
                "broadcaster.account_{}",
                optional_string(b, "action", "unknown")
            ),
            optional_string(b, "id", "").into(),
            "broadcaster",
        ),
        ["broadcaster", "config"] => ("broadcaster.config_update".into(), detail, "broadcaster"),
        ["players", "action"] => (
            format!("player:{}", optional_string(b, "action", "unknown")),
            optional_string(b, "target", "").into(),
            "player",
        ),
        ["console", "command"] => (
            "console.command".into(),
            optional_string(b, "command", "").into(),
            "console",
        ),
        ["auth", "users", ..] => (
            format!(
                "auth.user_{}",
                if *method == Method::POST {
                    "create"
                } else if *method == Method::DELETE {
                    "delete"
                } else {
                    "update"
                }
            ),
            detail,
            "auth",
        ),
        ["properties", "raw"] => ("properties.raw_update".into(), detail, "properties"),
        ["properties", key] => (
            format!("properties.update:{key}"),
            optional_string(b, "value", "").into(),
            "properties",
        ),
        ["backups", "create"] => (
            "backup.run".into(),
            optional_string(b, "world", "").into(),
            "backup",
        ),
        ["backups", "settings"] => ("backup.settings_update".into(), detail, "backup"),
        ["backups", "scheduler"] => ("backup.scheduler_update".into(), detail, "backup"),
        ["backups", _, _, "restore-to-world"] => {
            ("backup.restore_to_world".into(), detail, "backup")
        }
        ["backups", ..] => (
            format!("backup.{}", method.as_str().to_lowercase()),
            detail,
            "backup",
        ),
        ["settings", ..] => ("settings.update".into(), detail, "settings"),
        ["addons", action] => (
            format!("addons.{}", action.replace('-', "_")),
            detail,
            "addons",
        ),
        ["addons", "order", ..] => ("addons.reorder".into(), detail, "addons"),
        ["store", "install"] => (
            format!(
                "store.install:{}",
                optional_string(b, "project_slug", "unknown")
            ),
            detail,
            "addons",
        ),
        ["instances", id, "action"] => (
            format!(
                "instance:{}:{}",
                id,
                optional_string(b, "action", "unknown")
            ),
            detail,
            "instances",
        ),
        ["instances", ..] => (
            format!(
                "instance.{}",
                if *method == Method::POST {
                    "create"
                } else if *method == Method::DELETE {
                    "delete"
                } else {
                    "update"
                }
            ),
            detail,
            "instances",
        ),
        ["host"] => ("host.view".into(), detail, "instances"),
        ["files", ..] => (
            format!("files.{}", method.as_str().to_lowercase()),
            detail,
            "files",
        ),
        _ => (
            format!(
                "{}:{}",
                p.first().copied().unwrap_or("general"),
                method.as_str().to_lowercase()
            ),
            detail,
            "general",
        ),
    }
}
#[allow(clippy::too_many_lines)]
async fn dispatch_authenticated(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let write = m == Method::PUT;
    let create = m == Method::POST;
    let delete = m == Method::DELETE;
    let data = match p {
        ["auth", "me"] if read => s.auth.me(u).await,
        ["auth", "users"] if read => {
            s.auth.require(u, "USERS_VIEW").await?;
            s.auth.users().await
        }
        ["auth", "users"] if create => {
            s.auth.require(u, "USERS_CREATE").await?;
            s.auth
                .mutate_user(u, value_string(b, "username")?, Some(b.clone()), true)
                .await?
        }
        ["auth", "users", name] if write || delete => {
            s.auth
                .require(u, if delete { "USERS_DELETE" } else { "USERS_EDIT" })
                .await?;
            s.auth
                .mutate_user(u, name, if delete { None } else { Some(b.clone()) }, false)
                .await?
        }
        // Role metadata is readable by every signed-in user: the UI renders
        // role chips with names and colors across pages. Mutations are gated.
        ["roles"] if read => s.auth.roles().await,
        ["roles", name] if read => s.auth.role(name).await?,
        ["roles"] if create => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (role, renamed) = s
                .auth
                .mutate_role(u, value_string(b, "name")?, Some(b.clone()), true)
                .await?;
            if let Some(old) = renamed {
                s.instances
                    .rename_access_role(&old, role["name"].as_str().unwrap_or_default())
                    .await?;
            }
            s.audit(
                u,
                "roles.create",
                role["name"].as_str().unwrap_or("role"),
                "settings",
            )
            .await?;
            role
        }
        ["roles", name] if write => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (role, renamed) = s.auth.mutate_role(u, name, Some(b.clone()), false).await?;
            if let Some(old) = renamed {
                s.instances
                    .rename_access_role(&old, role["name"].as_str().unwrap_or_default())
                    .await?;
            }
            s.audit(u, "roles.update", name, "settings").await?;
            role
        }
        ["roles", name] if delete => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (result, _) = s.auth.mutate_role(u, name, None, false).await?;
            // Memberships in instance access entries are dropped with the role.
            s.instances.strip_access_role(name).await?;
            s.audit(u, "roles.delete", name, "settings").await?;
            result
        }
        ["server", "status"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            json!(svc.server.snapshot())
        }
        ["server", "backend"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            backend_status(svc).await
        }
        ["server", "backend", "versions"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            serde_json::to_value(s.bds.registry().await?).map_err(anyhow::Error::from)?
        }
        ["server", "backend", "install"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            install_bds(svc, optional_string(b, "version", "latest")).await?
        }
        ["server", "backend", "endstone", "install"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            install_endstone(svc, b["version"].as_str().map(str::to_string)).await?
        }
        ["server", "backend", "switch"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            switch_backend(svc, b).await?
        }
        ["server", "action"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            let action = value_string(b, "action")?.to_owned();
            // Starting on Endstone with no launcher available bootstraps the
            // panel toolchain inline instead of failing or crash-looping.
            // Copy the backend out: the settings read guard is not Send and
            // must not live across the bootstrap awaits.
            let backend = svc.server.settings.borrow().backend;
            let bootstrap = if action == "start" {
                match backend {
                    Backend::Vanilla => ensure_vanilla_installed(svc).await,
                    Backend::Endstone => ensure_endstone_toolchain(svc).await,
                }
            } else {
                Ok(())
            };
            if let Err(e) = bootstrap {
                json!({"success": false, "message": e.to_string()})
            } else {
                let server = svc.server.clone();
                let lock = svc.operations.clone().lock_owned().await;
                let result = tokio::spawn(async move {
                    let _lock = lock;
                    server.action(&action).await
                })
                .await
                .map_err(anyhow::Error::from)?;
                match result {
                    Ok(message) => success(message),
                    Err(e) => json!({"success":false,"message":e.to_string()}),
                }
            }
        }
        ["console", "command"] if create => {
            s.require_for(u, "CONSOLE_SEND", svc).await?;
            svc.server
                .send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("")
        }
        ["console", "tail"] if read => {
            s.require_for(u, "CONSOLE_VIEW", svc).await?;
            let limit = q
                .get("lines")
                .map_or(Ok(200), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid 'lines'"))?;
            if !(1..=2000).contains(&limit) {
                return Err(ApiError::bad("'lines' must be 1..2000"));
            }
            let events = svc.server.console.tail(limit);
            if q.get("format").map(String::as_str) == Some("text") {
                let body = events
                    .iter()
                    .map(|e| e.line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok((
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    body + "\n",
                )
                    .into_response());
            }
            let owned = events
                .iter()
                .map(|e| (**e).clone())
                .collect::<Vec<omnibedrock_server::server::ConsoleEvent>>();
            json!({
                "instance": svc.record().id,
                "latest": owned.last().map(|e| e.sequence).unwrap_or(0),
                "events": owned,
            })
        }
        ["performance", "metrics"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            s.instance_metrics(&svc.record().id)
        }
        ["players"] if read => {
            s.require_for(u, "PLAYERS_VIEW", svc).await?;
            list_players(svc).await?
        }
        ["players", "action"] if create => {
            let action = value_string(b, "action")?;
            s.require_for(
                u,
                match action {
                    "kick" => "PLAYERS_KICK",
                    "ban" | "pardon" => "PLAYERS_BAN",
                    "op" | "deop" => "PLAYERS_OP",
                    _ => return Err(ApiError::bad("Invalid player action")),
                },
                svc,
            )
            .await?;
            let target = value_string(b, "target")?;
            if target.contains(['\n', '\r', '\0', '"']) || target.is_empty() {
                return Err(ApiError::bad("Invalid player name"));
            }
            let reason = optional_string(b, "reason", "");
            let command = format!(
                "{action} \"{target}\"{}",
                if action == "kick" && !reason.is_empty() {
                    format!(" {reason}")
                } else {
                    String::new()
                }
            );
            svc.server
                .send_command(&command)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!("Sent '{command}' to server"))
        }
        ["settings"] if read => {
            s.auth.require(u, "SETTINGS_VIEW").await?;
            let mut v = serde_json::to_value(s.config.as_ref()).map_err(anyhow::Error::from)?;
            if let Some(v) = v.as_object_mut() {
                v.remove("endstone_python");
                v.remove("endstone_bin");
            }
            v
        }
        ["settings", "server"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            let mut v = json!(svc.server.settings.borrow().clone());
            v["crash_count"] = json!(svc.server.snapshot().crash_count);
            v
        }
        ["settings", "panel"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            let enabled = b["login_enabled"]
                .as_bool()
                .ok_or_else(|| ApiError::bad("'login_enabled' must be a boolean"))?;
            s.auth.set_login_enabled(enabled).await?;
            success(if enabled {
                "Login required"
            } else {
                "Login disabled; the panel now serves every visitor as Admin"
            })
        }
        ["settings", "curseforge"] if read => {
            s.auth.require(u, "SETTINGS_VIEW").await?;
            let hint = s.auth.curseforge_key_hint().await;
            json!({
                "configured": s.curseforge.configured(),
                "key_hint": hint,
                "source": if hint.is_some() { "settings" } else { "env" },
            })
        }
        ["settings", "curseforge"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            let key = b
                .get("api_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            s.auth.set_curseforge_key(key).await?;
            // An empty key clears the override and falls back to the .env value.
            if key.is_empty() {
                s.curseforge
                    .set_api_key(s.config.curseforge_api_key.expose_secret());
            } else {
                s.curseforge.set_api_key(key);
            }
            json!({
                "success": true,
                "message": if key.is_empty() {
                    "CurseForge API key cleared; falling back to CURSEFORGE_API_KEY from .env"
                } else {
                    "CurseForge API key saved — the Store picks it up immediately"
                },
                "configured": s.curseforge.configured(),
            })
        }
        ["settings", "server"] if write => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            let mut v = json!(svc.server.settings.borrow().clone());
            let obj = b
                .as_object()
                .ok_or_else(|| ApiError::bad("Settings must be an object"))?;
            for (key, val) in obj {
                if !val.is_null() && v.get(key).is_some() {
                    v[key] = val.clone();
                }
            }
            let options: ServerSettings =
                serde_json::from_value(v).map_err(|e| ApiError::bad(e.to_string()))?;
            options
                .validate()
                .map_err(|e| ApiError::bad(e.to_string()))?;
            success(svc.server.configure(options).await?)
        }
        ["audit"] if read => {
            s.auth.require(u, "AUDIT_VIEW").await?;
            let limit = q
                .get("limit")
                .map_or(Ok(100), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid limit"))?;
            if !(1..=500).contains(&limit) {
                return Err(ApiError::bad("Limit must be 1..500"));
            }
            let offset = q
                .get("offset")
                .map_or(Ok(0), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid offset"))?;
            json!(
                s.audit_entries
                    .lock()
                    .await
                    .iter()
                    .rev()
                    .filter(|e| q.get("username").is_none_or(|v| e["username"] == *v)
                        && q.get("category").is_none_or(|v| e["category"] == *v)
                        && q.get("action")
                            .is_none_or(|v| e["action"].as_str().is_some_and(|a| a.contains(v))))
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>()
            )
        }
        ["properties"] if read => {
            s.require_for(u, "PROPERTIES_VIEW", svc).await?;
            json!(files::properties(&properties_text(svc).await?))
        }
        ["properties", "raw"] if read => {
            s.require_for(u, "PROPERTIES_VIEW", svc).await?;
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                properties_text(svc).await?,
            )
                .into_response());
        }
        ["properties", "raw"] if write => {
            s.require_for(u, "PROPERTIES_EDIT", svc).await?;
            let _lock = s.edits.lock().await;
            // Writes must land in the SAME file the reads come from: the
            // X-Instance-scoped server dir, not the panel-level default.
            atomic_write(
                &safe_path(
                    &svc.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                value_string(b, "text")?.as_bytes(),
            )
            .await?;
            success("Properties saved")
        }
        ["properties", key] if write => {
            s.require_for(u, "PROPERTIES_EDIT", svc).await?;
            let value = value_string(b, "value")?;
            files::validate_property(key, value)?;
            let _lock = s.edits.lock().await;
            let text = properties_text(svc).await?;
            let mut entries = files::properties(&text);
            let mut found = false;
            for e in &mut entries {
                if e["key"] == *key {
                    e["value"] = json!(value);
                    found = true;
                }
            }
            if !found {
                // A key missing from the file is appended, not rejected, so
                // every property stays configurable even before the server
                // has ever written it.
                entries.push(json!({
                    "key": key,
                    "value": value,
                    "comment": "",
                    "inline_comment": ""
                }));
            }
            let text = entries
                .iter()
                .map(|e| {
                    if e["comment"].as_str().is_some_and(|s| !s.is_empty()) {
                        e["comment"].as_str().unwrap_or_default().to_string()
                    } else if e["key"] == "" {
                        String::new()
                    } else {
                        let comment = e["inline_comment"].as_str().unwrap_or_default();
                        format!(
                            "{}={}{}",
                            e["key"].as_str().unwrap_or_default(),
                            e["value"].as_str().unwrap_or_default(),
                            if comment.is_empty() {
                                String::new()
                            } else {
                                format!(" {comment}")
                            }
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            atomic_write(
                &safe_path(
                    &svc.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                text.as_bytes(),
            )
            .await?;
            success(format!("'{key}' updated to '{value}'"))
        }
        ["worlds"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            json!(files::dirs(&svc.config.bedrock_server_dir.join("worlds")).await?)
        }
        ["worlds", world] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            component(world)?;
            let path = safe_path(
                &svc.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            if !tokio::fs::try_exists(&path).await? {
                return Err(ApiError::missing("World not found"));
            }
            let name = world.to_string();
            tokio::task::spawn_blocking(move || {
                let mut size = 0;
                let mut count = 0;
                for entry in walkdir::WalkDir::new(&path).follow_links(false) {
                    let entry = entry.map_err(anyhow::Error::from)?;
                    if entry.file_type().is_file() {
                        size += entry.metadata().map_err(anyhow::Error::from)?.len();
                        count += 1;
                    }
                }
                Ok::<_, anyhow::Error>(
                    json!({"name":name,"path":path,"size_bytes":size,"files":count}),
                )
            })
            .await
            .map_err(anyhow::Error::from)??
        }
        ["worlds", world, "contents"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            component(world)?;
            let path = safe_path(
                &svc.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            tokio::task::spawn_blocking(move || files::world_tree(&path))
                .await
                .map_err(anyhow::Error::from)??
        }
        ["files"] if read => {
            s.require_for(u, "FILES_VIEW", svc).await?;
            list_files(s).await?
        }
        ["files", filename] if read || write || delete => {
            s.auth
                .require(u, if read { "FILES_VIEW" } else { "FILES_EDIT" })
                .await?;
            let path = config_file_path(s, filename).await?;
            if read {
                json!({"name":filename,"content":tokio::fs::read_to_string(path).await?})
            } else {
                let _lock = s.edits.lock().await;
                if write {
                    atomic_write(&path, value_string(b, "content")?.as_bytes()).await?;
                } else {
                    tokio::fs::remove_file(path).await?;
                }
                success("File updated")
            }
        }
        ["addons", ..] => {
            return addons(s, svc, u, m, p, q, b).await;
        }
        ["store", ..] => {
            return store(s, svc, u, m, p, q, b).await;
        }
        ["backups", ..] => {
            return backups(s, svc, u, m, p, q, b).await;
        }
        ["broadcaster", ..] => {
            return broadcaster(s, svc, u, m, p, q, b).await;
        }
        // ---- Multi-instance management ----
        ["instances"] if read => {
            // Every user sees the instances they can access.
            let mut list = Vec::new();
            for instance in s.instances.list().await {
                let record = instance.record();
                if record.id != DEFAULT_INSTANCE && !s.auth.can_access_instance(u, &record).await {
                    continue;
                }
                let mut item = instance.summary();
                item["usage"] = usage_json(&s.instance_metrics(&record.id));
                list.push(item);
            }
            json!(list)
        }
        ["instances"] if create => {
            s.auth.require(u, "INSTANCES_CREATE").await?;
            create_instance(s, u, b).await?
        }
        ["instances", id] if read => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let mut item = instance.summary();
            item["usage"] = usage_json(&s.instance_metrics(&instance.record().id));
            // Live scan: pack uuids referenced by world order files or pack
            // dependencies with nothing on disk, so the UI can warn before a
            // boot logs per-pack "was not found" noise.
            let warnings = tokio::task::spawn_blocking({
                let worlds = instance.config.bedrock_server_dir.join("worlds");
                move || omnibedrock_server::addons::missing_dependencies(&worlds)
            })
            .await
            .unwrap_or_default();
            if !warnings.is_empty() {
                item["pack_warnings"] = json!(warnings);
            }
            item
        }
        ["instances", id] if write => {
            let instance = s.resolve_instance(Some(id), u).await?;
            update_instance(s, u, &instance, b).await?
        }
        ["instances", id] if delete => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let record = instance.record();
            let may_delete = s.auth.is_owner(u).await
                || record.owner == u.username
                || s.auth.require(u, "INSTANCES_DELETE").await.is_ok();
            if !may_delete {
                return Err(ApiError::forbidden(
                    "Only the instance owner, a panel owner, or a holder of INSTANCES_DELETE may delete it",
                ));
            }
            if q.get("confirm").map(String::as_str) != Some("yes") {
                return Err(ApiError::bad(
                    "Pass ?confirm=yes to delete; this cannot be undone",
                ));
            }
            let keep_data = q.get("keep_data").is_some_and(|v| v == "1" || v == "true");
            s.instances
                .remove(id, !keep_data)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!(
                "Instance '{id}' removed{}",
                if keep_data {
                    " (data kept on disk)"
                } else {
                    ""
                }
            ))
        }
        ["instances", id, "action"] if create => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let record = instance.record();
            if !s.auth.can_access_instance(u, &record).await {
                return Err(ApiError::forbidden(
                    "You can view this instance but cannot control its runtime",
                ));
            }
            s.require_for(u, "SERVER_MANAGE", &instance).await?;
            let action = value_string(b, "action")?.to_owned();
            let backend = instance.server.settings.borrow().backend;
            let bootstrap = if action == "start" {
                match backend {
                    Backend::Vanilla => ensure_vanilla_installed(&instance).await,
                    Backend::Endstone => ensure_endstone_toolchain(&instance).await,
                }
            } else {
                Ok(())
            };
            if let Err(e) = bootstrap {
                json!({"success": false, "message": e.to_string()})
            } else {
                let server = instance.server.clone();
                let lock = instance.operations.clone().lock_owned().await;
                let result = tokio::spawn(async move {
                    let _lock = lock;
                    server.action(&action).await
                })
                .await
                .map_err(anyhow::Error::from)?;
                match result {
                    Ok(message) => success(message),
                    Err(e) => json!({"success":false,"message":e.to_string()}),
                }
            }
        }
        ["host"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            let mut host = s.metrics.all.borrow()["host"].clone();
            host["capabilities"] = json!(s.instances.capabilities());
            let records = s.instances.records().await;
            let allocated_memory: u64 = records
                .iter()
                .map(|r| r.resources.memory_mb.unwrap_or(0))
                .sum();
            let allocated_cpus: f64 = records
                .iter()
                .map(|r| r.resources.cpus.unwrap_or(0.0))
                .sum();
            host["allocated"] = json!({
                "memory_mb": allocated_memory,
                "cpus": allocated_cpus,
                "instances": records.len(),
            });
            // Overcommitment: aggregate allocations vs capacity minus the
            // configurable reserve. Warnings are advisory — per-instance
            // limits stay enforced regardless.
            let reserve = s.config.host_reserve_mb;
            let (memory, cpu) = omnibedrock_server::instances::overcommit_status(
                host["memory_total_mb"].as_f64().unwrap_or(0.0),
                reserve,
                allocated_memory,
                host["cpu_count"].as_u64().unwrap_or(0) as usize,
                allocated_cpus,
            );
            host["reserve_mb"] = json!(reserve);
            host["overcommit"] = json!({
                "memory_exceeded": memory.is_some(),
                "memory_warning": memory,
                "cpu_exceeded": cpu.is_some(),
                "cpu_warning": cpu,
            });
            host
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}

fn usage_json(metrics: &Value) -> Value {
    json!({
        "cpu_percent": metrics["cpu_percent"].as_f64().unwrap_or(0.0),
        "memory_mb": metrics["memory_mb"].as_f64().unwrap_or(0.0),
        "disk_mb": metrics["disk_mb"].as_f64().unwrap_or(0.0),
        "usage_source": metrics["usage_source"].as_str().unwrap_or("unknown"),
    })
}

/// Create an instance after checking quotas, port availability, and the
/// deployment backend's availability on this host.
async fn create_instance(s: &AppState, u: &User, b: &Value) -> Result<Value> {
    let name = value_string(b, "name")?.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad("name must be 1..64 characters"));
    }
    omnibedrock_core::files::component(&name).map_err(|e| ApiError::bad(e.message))?;
    let backend = Backend::parse(value_string(b, "backend")?)?;
    let deployment = Deployment::parse(optional_string(b, "deployment", "native"))?;
    let resources = ResourcePolicy {
        memory_mb: b.get("memory_mb").and_then(Value::as_u64),
        cpus: b.get("cpus").and_then(Value::as_f64),
    };
    resources
        .validate()
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let port = match b.get("port") {
        None | Some(Value::Null) => 0u16,
        Some(v) => v
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| ApiError::bad("'port' must be a UDP port number (0 = auto)"))?,
    };
    let id = omnibedrock_server::instances::slug(&name);
    if !omnibedrock_server::instances::valid_id(&id) {
        return Err(ApiError::bad(format!(
            "could not derive a valid instance id from '{name}'"
        )));
    }
    let record = InstanceRecord {
        id,
        name,
        owner: u.username.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        port,
        deployment,
        resources,
        access: vec![],
        autostart: b["autostart"].as_bool().unwrap_or(false),
    };
    // Quotas apply to everyone except the panel owner, who sets them.
    let quotas = u.quotas.clone();
    let is_owner_role = s.auth.is_owner(u).await;
    let username = u.username.clone();
    let extra = record.resources;
    let instance = s
        .instances
        .create(record, move |records| {
            if is_owner_role {
                return Ok(());
            }
            let owned: Vec<&InstanceRecord> =
                records.iter().filter(|r| r.owner == username).collect();
            quotas.check(&owned, extra)
        })
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?;
    // Seed the instance's runtime settings (backend + version) now that the
    // per-instance settings file exists.
    let mut settings = ServerSettings {
        backend,
        ..ServerSettings::default()
    };
    if let Some(version) = b["vanilla_version"].as_str() {
        omnibedrock_core::files::component(version).map_err(|e| ApiError::bad(e.message))?;
        settings.vanilla_version = version.into();
    }
    if let Err(e) = instance.server.configure(settings).await {
        return Err(ApiError::conflict(format!(
            "instance created but saving its settings failed: {e}"
        )));
    }
    let mut item = instance.summary();
    item["usage"] = usage_json(&Value::Null);
    Ok(item)
}

/// Update an instance's editable fields. Deployment, resources, and port
/// changes require the instance to be stopped (enforced by the manager).
async fn update_instance(
    s: &AppState,
    u: &User,
    instance: &Arc<Instance>,
    b: &Value,
) -> Result<Value> {
    let record = instance.record();
    let actor_is_owner = s.auth.is_owner(u).await;
    let is_instance_owner = record.owner == u.username;
    // Resource/limit changes are a granular permission (INSTANCES_LIMITS,
    // grantable globally or through an instance share) on top of the owner
    // paths; the quota check below still runs for everyone. Access-list
    // management additionally accepts INSTANCES_SHARE.
    let can_edit_limits = actor_is_owner
        || is_instance_owner
        || s.auth.require(u, "INSTANCES_LIMITS").await.is_ok()
        || s.auth
            .instance_permissions(u, &record)
            .await
            .iter()
            .any(|p| p == "INSTANCES_LIMITS");
    let can_edit_access = can_edit_limits || s.auth.require(u, "INSTANCES_SHARE").await.is_ok();
    let touches_limits = ["name", "port", "deployment", "memory_mb", "cpus"]
        .iter()
        .any(|k| b.get(*k).is_some());
    if b.get("access").is_some() && !can_edit_access {
        return Err(ApiError::forbidden(
            "Only the instance owner, a panel owner, or a holder of INSTANCES_SHARE may manage instance access",
        ));
    }
    if touches_limits && !can_edit_limits {
        return Err(ApiError::forbidden(
            "Only the instance owner, a panel owner, or a holder of INSTANCES_LIMITS may change its resources",
        ));
    }
    if !touches_limits && b.get("access").is_none() {
        return Err(ApiError::bad("Nothing to update"));
    }
    let mut next = record.clone();
    if let Some(name) = b["name"].as_str() {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::bad("name must be 1..64 characters"));
        }
        omnibedrock_core::files::component(name).map_err(|e| ApiError::bad(e.message))?;
        next.name = name.into();
    }
    if let Some(port) = b["port"].as_u64() {
        let port =
            u16::try_from(port).map_err(|_| ApiError::bad("'port' must be a UDP port number"))?;
        next.port = port;
    }
    if let Some(deployment) = b["deployment"].as_str() {
        next.deployment =
            Deployment::parse(deployment).map_err(|e| ApiError::bad(e.to_string()))?;
    }
    if let Some(autostart) = b["autostart"].as_bool() {
        if autostart && next.deployment != Deployment::Systemd {
            return Err(ApiError::bad(
                "autostart applies to systemd instances only (it enables the user unit)",
            ));
        }
        next.autostart = autostart;
    }
    if b.get("memory_mb").is_some() {
        next.resources.memory_mb = b["memory_mb"].as_u64();
    }
    if b.get("cpus").is_some() {
        next.resources.cpus = b["cpus"].as_f64();
    }
    if let Some(access) = b["access"].as_array() {
        let mut entries = Vec::new();
        for entry in access {
            let user = entry["user"]
                .as_str()
                .ok_or_else(|| ApiError::bad("access entries need a user"))?;
            omnibedrock_core::files::component(user).map_err(|e| ApiError::bad(e.message))?;
            // Roles reference the shared catalog; the legacy two-level field is
            // still accepted from old clients and maps the same way as files.
            let mut roles: Vec<String> = match entry.get("roles") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(Value::String(role)) => vec![role.clone()],
                _ => match entry["level"].as_str().unwrap_or("view") {
                    "manage" => vec!["instance-manager".into()],
                    _ => vec!["viewer".into()],
                },
            };
            roles.sort();
            roles.dedup();
            if roles.is_empty() {
                continue;
            }
            // Delegation guard: without the owner sentinel you cannot grant an
            // instance capability you do not hold on this very instance. Only
            // instance-scoped permissions are effective here, so only those
            // are checked.
            if !actor_is_owner {
                let granted: Vec<String> = s
                    .auth
                    .roles()
                    .await
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter(|r| roles.contains(&r["name"].as_str().unwrap_or_default().to_string()))
                    .flat_map(|r| r["permissions"].as_array().cloned().unwrap_or_default())
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .filter(|p| INSTANCE_PERMISSIONS.contains(&p.as_str()))
                    .collect();
                let held = s.auth.instance_permissions(u, &record).await;
                if let Some(missing) = granted.iter().find(|p| !held.contains(p)) {
                    return Err(ApiError::forbidden(format!(
                        "You cannot grant {perm} on this instance — you do not hold it yourself",
                        perm = missing
                    )));
                }
            }
            entries.push(AccessEntry {
                user: user.into(),
                roles,
                level: None,
            });
        }
        next.access = entries;
    }
    next.resources
        .validate()
        .map_err(|e| ApiError::bad(e.to_string()))?;
    // A resource increase must respect the instance owner's quota (panel
    // owner exempt); the check runs inside the manager's write lock.
    let quotas = s.auth.user_quotas(&next.owner).await.unwrap_or_default();
    let is_panel_owner = actor_is_owner;
    let owner = next.owner.clone();
    let target_id = next.id.clone();
    s.instances
        .configure(&record.id, next, move |records| {
            if is_panel_owner {
                return Ok(());
            }
            let memory: u64 = records
                .iter()
                .filter(|r| r.owner == owner)
                .map(|r| r.resources.memory_mb.unwrap_or(0))
                .sum();
            let cpus: f64 = records
                .iter()
                .filter(|r| r.owner == owner)
                .map(|r| r.resources.cpus.unwrap_or(0.0))
                .sum();
            if let Some(limit) = quotas.memory_mb
                && memory > limit
            {
                return Err(anyhow::anyhow!(format!(
                    "memory quota exceeded: {memory} MiB allocated > {limit} MiB"
                )));
            }
            if let Some(limit) = quotas.cpus
                && cpus > limit + 1e-9
            {
                return Err(anyhow::anyhow!(format!(
                    "cpu quota exceeded: {cpus:.2} allocated > {limit:.2}"
                )));
            }
            Ok(())
        })
        .await
        .map_err(|e| ApiError::forbidden(e.to_string()))?;
    let instance = s
        .instances
        .get(&target_id)
        .await
        .ok_or_else(|| ApiError::missing("Instance vanished"))?;
    let mut item = instance.summary();
    item["usage"] = usage_json(&s.instance_metrics(&target_id));
    Ok(item)
}
async fn properties_text(svc: &Arc<Instance>) -> Result<String> {
    match tokio::fs::read_to_string(
        safe_path(
            &svc.config.bedrock_server_dir,
            FsPath::new("server.properties"),
        )
        .await?,
    )
    .await
    {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}
async fn config_file_path(s: &AppState, name: &str) -> Result<PathBuf> {
    component(name)?;
    if [
        "users.yaml",
        "roles.yaml",
        "jwt_secret",
        "console_lock_state.yaml",
        "backup_settings.yaml",
        "server_settings.yaml",
    ]
    .contains(&name)
    {
        return Err(ApiError::forbidden(
            "Edit this file through its dedicated settings API",
        ));
    }
    if !["ini", "yaml", "yml", "json", "txt"].contains(
        &FsPath::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or(""),
    ) {
        return Err(ApiError::bad("Unsupported file extension"));
    }
    safe_path(&s.config.data_dir, FsPath::new(name)).await
}
async fn list_files(s: &AppState) -> Result<Value> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(&s.config.data_dir).await?;
    while let Some(e) = entries.next_entry().await? {
        let name = e.file_name().to_string_lossy().to_string();
        if config_file_path(s, &name).await.is_err() || !e.file_type().await?.is_file() {
            continue;
        }
        let m = e.metadata().await?;
        let modified = m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        result.push(json!({"name":name,"path":e.path(),"size":m.len(),"modified":modified}));
    }
    result.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!(result))
}
async fn list_players(svc: &Arc<Instance>) -> Result<Value> {
    if svc.server.snapshot().status != Status::Running {
        return Ok(json!({"players":[],"count":0}));
    }
    let console = &svc.server.console;
    let mut changes = console.changed.subscribe();
    let mut cursor = Some(console.replay(None, 0).latest);
    svc.server.send_command("list").await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let batch = console.replay(cursor, 256);
        if batch.gap.is_some() {
            return Err(ApiError::conflict(
                "Player response exceeded console retention",
            ));
        }
        for line in batch.lines {
            cursor = Some(line.sequence);
            if line.line.to_lowercase().contains("players online") {
                let names = line
                    .line
                    .rsplit_once(':')
                    .map(|(_, names)| {
                        names
                            .split(',')
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map(|name| json!({"name":name}))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                return Ok(json!({"count":names.len(),"players":names}));
            }
        }
        if tokio::time::timeout_at(deadline, changes.changed())
            .await
            .is_err()
        {
            return Ok(json!({"players":[],"count":0}));
        }
    }
}
async fn addons(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let create = m == Method::POST;
    s.auth
        .require(u, if read { "ADDONS_VIEW" } else { "ADDONS_MANAGE" })
        .await?;
    let root = svc.config.bedrock_server_dir.join("worlds");
    let _lock = if read {
        None
    } else {
        Some(s.edits.lock().await)
    };
    let pack_path = |value: String| {
        let root = root.clone();
        async move {
            let path = FsPath::new(&value);
            let relative = if path.is_absolute() {
                let canonical = tokio::fs::canonicalize(&root).await?;
                path.strip_prefix(canonical)
                    .map_err(|_| ApiError::bad("Addon path outside worlds"))?
                    .to_path_buf()
            } else {
                let lexical_root = &root;
                if let Ok(p) = path.strip_prefix(lexical_root) {
                    p.to_path_buf()
                } else {
                    path.to_path_buf()
                }
            };
            let path = safe_path(&root, &relative).await?;
            let parts = relative.components().collect::<Vec<_>>();
            if parts.len() != 3
                || !["behavior_packs", "resource_packs"]
                    .iter()
                    .any(|name| parts[1].as_os_str() == *name)
            {
                return Err(ApiError::bad("Invalid addon path"));
            }
            Ok::<_, ApiError>(path)
        }
    };
    let data = match p {
        ["addons"] if read => {
            // Force a full re-walk with ?refresh=1; otherwise packs whose
            // fingerprint is unchanged come straight from the parse cache.
            let refresh = q.contains_key("refresh");
            let mut result = json!({"behavior_packs":[],"resource_packs":[]});
            let mut live: HashSet<String> = HashSet::new();
            for world in files::dirs(&root).await? {
                let world_path = safe_path(&root, FsPath::new(&world)).await?;
                for pack_type in ["behavior_packs", "resource_packs"] {
                    let path = safe_path(&root, &PathBuf::from(&world).join(pack_type)).await?;
                    if !tokio::fs::try_exists(&path).await? {
                        continue;
                    }
                    for name in files::dirs(&path).await? {
                        let pack =
                            safe_path(&root, &PathBuf::from(&world).join(pack_type).join(&name))
                                .await?;
                        let pack_string = pack.to_string_lossy().into_owned();
                        live.insert(pack_string.clone());
                        let fingerprint =
                            omnibedrock_server::addon_cache::pack_fingerprint(&pack).await;
                        if !refresh
                            && let Some(item) =
                                s.addon_meta.lookup(&pack_string, &fingerprint).await
                        {
                            result[pack_type]
                                .as_array_mut()
                                .expect("pack array")
                                .push(item);
                            continue;
                        }
                        let manifest = read_manifest(&pack).await.ok();
                        let store_meta = omnibedrock_server::addons::read_pack_metadata(&pack);
                        let source = store_meta
                            .as_ref()
                            .and_then(|m| m["source"].as_str())
                            .unwrap_or("manual")
                            .to_owned();
                        let stats = tokio::task::spawn_blocking({
                            let pack = pack.clone();
                            move || omnibedrock_server::addons::pack_stats(&pack)
                        })
                        .await
                        .map_err(anyhow::Error::from)?;
                        // Marketplace packs put lang keys like `pack.name` in
                        // the manifest; resolve them from texts/*.lang.
                        let texts = omnibedrock_server::addons::read_texts(&pack).await;
                        let header = manifest.as_ref().and_then(|m| m.get("header"));
                        let display_name = omnibedrock_server::addons::resolve_lang(
                            &texts,
                            header.and_then(|h| h["name"].as_str()),
                        )
                        .unwrap_or_else(|| name.clone());
                        let description = omnibedrock_server::addons::resolve_lang(
                            &texts,
                            header.and_then(|h| h["description"].as_str()),
                        );
                        let item = json!({"name":display_name,"description":description,"path":pack,"world":world,"pack_type":pack_type,"uuid":manifest.as_ref().map(|v|v["header"]["uuid"].clone()).unwrap_or(json!("")),"version":manifest.as_ref().map(|v|v["header"]["version"].clone()).unwrap_or(json!([])),"valid":manifest.is_some(),"manifest":manifest,"size_bytes":stats.0,"modified":stats.1,"icon":stats.2,"icon_version":fingerprint,"source":source,"store":store_meta});
                        s.addon_meta
                            .store(pack_string, fingerprint, item.clone())
                            .await;
                        result[pack_type]
                            .as_array_mut()
                            .expect("pack array")
                            .push(item);
                    }
                }
                let _ = world_path;
            }
            s.addon_meta.retain_paths(&live).await;
            // No-op unless something changed (miss, prune, or forced refresh).
            s.addon_meta.flush().await;
            result
        }
        ["addons", "icon"] if read => {
            let raw = q
                .get("path")
                .cloned()
                .ok_or_else(|| ApiError::bad("Path required"))?;
            let pack = pack_path(raw).await?;
            let mut found = None;
            for candidate in ["pack_icon.png", "pack_icon.jpg", "Pack_Icon.png"] {
                let icon = pack.join(candidate);
                if tokio::fs::try_exists(&icon).await? {
                    found = Some((
                        icon,
                        if candidate.ends_with(".jpg") {
                            "image/jpeg"
                        } else {
                            "image/png"
                        },
                    ));
                    break;
                }
            }
            let Some((icon, content_type)) = found else {
                return Err(ApiError::missing("Pack has no icon"));
            };
            let file = tokio::fs::File::open(&icon).await?;
            let size = file.metadata().await?.len();
            // Icons are addressed with a fingerprint version param, so a
            // changed pack gets a new URL and a long max-age stays safe.
            return Ok((
                [
                    (header::CONTENT_TYPE, content_type.to_string()),
                    (header::CONTENT_LENGTH, size.to_string()),
                    (header::CACHE_CONTROL, "public, max-age=604800".to_string()),
                ],
                Body::from_stream(ReaderStream::new(file)),
            )
                .into_response());
        }
        ["addons", "delete"] if create => {
            let raw = value_string(b, "path")?.to_owned();
            let pack = pack_path(raw).await?;
            let canonical_root = tokio::fs::canonicalize(&root).await?;
            let world = pack
                .strip_prefix(&canonical_root)
                .map_err(|_| ApiError::bad("Addon path outside worlds"))?
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_default();
            let uuid = read_manifest(&pack)
                .await
                .ok()
                .and_then(|m| m["header"]["uuid"].as_str().map(str::to_owned))
                .unwrap_or_default();
            let name = pack
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            omnibedrock_server::addons::delete_pack(&pack, &root.join(&world), &uuid)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            s.audit(u, "addons.delete", &format!("{world}/{name}"), "addons")
                .await
                .ok();
            json!({"success":true,"message":format!("Deleted addon '{name}'")})
        }
        ["addons", "manifest"] if read => {
            let path = pack_path(
                q.get("path")
                    .cloned()
                    .ok_or_else(|| ApiError::bad("Path required"))?,
            )
            .await?;
            read_manifest(&path).await?
        }
        ["addons", "manifest"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            if !b["manifest"].is_object() {
                return Err(ApiError::bad("Manifest must be an object"));
            }
            write_manifest(&path, &b["manifest"]).await?;
            success("Manifest updated")
        }
        ["addons", "rename"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let name = component(value_string(b, "new_name")?)?;
            let destination = path
                .parent()
                .ok_or_else(|| ApiError::bad("Invalid addon"))?
                .join(name);
            if tokio::fs::try_exists(&destination).await? {
                return Err(ApiError::conflict("An addon with that name already exists"));
            }
            tokio::fs::rename(path, &destination).await?;
            json!({"success":true,"new_path":destination,"message":"Addon renamed"})
        }
        ["addons", "randomize-uuid"] if *m == Method::POST => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let mut manifest = read_manifest(&path).await?;
            let uuid = uuid::Uuid::new_v4().to_string();
            if !manifest["header"].is_object() {
                manifest["header"] = json!({});
            }
            manifest["header"]["uuid"] = json!(uuid);
            write_manifest(&path, &manifest).await?;
            json!({"success":true,"uuid":uuid,"message":"UUID randomized"})
        }
        ["addons", "change-uuid"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let uuid = value_string(b, "uuid")?;
            uuid::Uuid::parse_str(uuid).map_err(|_| ApiError::bad("Invalid UUID"))?;
            let mut manifest = read_manifest(&path).await?;
            if !manifest["header"].is_object() {
                manifest["header"] = json!({});
            }
            manifest["header"]["uuid"] = json!(uuid);
            write_manifest(&path, &manifest).await?;
            success("UUID updated")
        }
        ["addons", "order", world, pack_type] if read || *m == Method::PUT => {
            component(world)?;
            if !["behavior_packs", "resource_packs"].contains(pack_type) {
                return Err(ApiError::bad("Invalid pack type"));
            }
            let path = safe_path(
                &root,
                &PathBuf::from(world).join(format!("world_{pack_type}.json")),
            )
            .await?;
            if read {
                match tokio::fs::read_to_string(path).await {
                    Ok(text) => {
                        serde_json::from_str(&text).map_err(|e| ApiError::bad(e.to_string()))?
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!([]),
                    Err(e) => return Err(e.into()),
                }
            } else {
                let uuids = b["uuids"]
                    .as_array()
                    .ok_or_else(|| ApiError::bad("uuids must be an array"))?;
                let world_dir = safe_path(&root, &PathBuf::from(world)).await?;
                let installed =
                    tokio::task::spawn_blocking(move || addons::manifest_versions(&world_dir))
                        .await
                        .unwrap_or_default();
                let mut order = Vec::new();
                let previous = tokio::fs::read_to_string(&path)
                    .await
                    .ok()
                    .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
                    .unwrap_or_default();
                for uid in uuids {
                    let uid = uid.as_str().ok_or_else(|| ApiError::bad("Invalid UUID"))?;
                    uuid::Uuid::parse_str(uid).map_err(|_| ApiError::bad("Invalid UUID"))?;
                    // BDS resolves packs by (uuid, exact version), so entries
                    // must carry the installed manifest's version — a fresh
                    // or stale entry would be silently dropped at boot.
                    let mut entry = previous
                        .iter()
                        .find(|v| v["pack_id"] == uid)
                        .cloned()
                        .unwrap_or(json!({"pack_id": uid}));
                    if let Some(version) = installed.get(uid) {
                        entry["version"] = json!(version);
                    } else if entry.get("version").is_none() {
                        entry["version"] = json!([1, 0, 0]);
                    }
                    order.push(entry);
                }
                atomic_write(
                    &path,
                    serde_json::to_string_pretty(&order)
                        .map_err(anyhow::Error::from)?
                        .as_bytes(),
                )
                .await?;
                success("Pack order updated")
            }
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}
async fn read_manifest(path: &FsPath) -> Result<Value> {
    let path = safe_path(path, FsPath::new("manifest.json")).await?;
    let text = tokio::fs::read_to_string(path).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| ApiError::bad(e.to_string()))?;
    if !v.is_object() {
        return Err(ApiError::bad("Manifest must be an object"));
    }
    Ok(v)
}
async fn write_manifest(path: &FsPath, value: &Value) -> Result<()> {
    let path = safe_path(path, FsPath::new("manifest.json")).await?;
    atomic_write(
        &path,
        serde_json::to_string_pretty(value)
            .map_err(anyhow::Error::from)?
            .as_bytes(),
    )
    .await?;
    Ok(())
}
async fn backups(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let create = m == Method::POST;
    let write = m == Method::PUT;
    let data = match p {
        ["backups", "worlds"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            json!(svc.backups.worlds().await?)
        }
        ["backups"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            let backups = svc.backups.list(q.get("world").cloned(), false).await?;
            json!({"total":backups.len(),"backups":backups})
        }
        ["backups", "trash"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            json!(svc.backups.list(q.get("world").cloned(), true).await?)
        }
        ["backups", "create"] if create => {
            s.require_for(u, "BACKUPS_CREATE", svc).await?;
            let r: BackupRequest =
                serde_json::from_value(b.clone()).map_err(|e| ApiError::bad(e.to_string()))?;
            let lock = svc.operations.clone().try_lock_owned().map_err(|_| {
                ApiError::conflict(
                    "A server operation or backup is already running on this instance",
                )
            })?;
            let service = svc.backups.clone();
            tokio::spawn(async move {
                let _lock = lock;
                service.run(r).await
            })
            .await
            .map_err(anyhow::Error::from)??
        }
        ["backups", "restore", world, filename] if create => {
            s.require_for(u, "BACKUPS_RESTORE", svc).await?;
            svc.backups.move_trash(world, filename, true).await?;
            success(format!("Restored {filename}"))
        }
        ["backups", world, filename] if *m == Method::DELETE => {
            s.require_for(u, "BACKUPS_DELETE", svc).await?;
            svc.backups.move_trash(world, filename, false).await?;
            success(format!("Moved {filename} to trash"))
        }
        ["backups", world, filename, "restore-to-world"] if create => {
            s.require_for(u, "BACKUPS_RESTORE", svc).await?;
            let lock = svc.operations.clone().try_lock_owned().map_err(|_| {
                ApiError::conflict(
                    "A server operation or backup is already running on this instance",
                )
            })?;
            let service = svc.backups.clone();
            let world = world.to_string();
            let filename = filename.to_string();
            let message = format!(
                "World '{world}' restored from '{filename}'. Server was stopped — start it manually when ready."
            );
            tokio::spawn(async move {
                let _lock = lock;
                service.restore_world(world, filename).await
            })
            .await
            .map_err(anyhow::Error::from)??;
            success(message)
        }
        ["backups", world, filename, "download"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            let path = svc.backups.path(world, filename, false).await?;
            let file = tokio::fs::File::open(path).await?;
            let size = file.metadata().await?.len();
            // RFC 5987 filename encoding avoids header injection from imported archives.
            let encoded = filename
                .bytes()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || b"-._".contains(&c) {
                        (c as char).to_string()
                    } else {
                        format!("%{c:02X}")
                    }
                })
                .collect::<String>();
            return Ok((
                [
                    (header::CONTENT_TYPE, "application/zip".to_string()),
                    (header::CONTENT_LENGTH, size.to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename*=UTF-8''{encoded}"),
                    ),
                ],
                Body::from_stream(ReaderStream::new(file)),
            )
                .into_response());
        }
        ["backups", "settings"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            svc.backups.settings()
        }
        ["backups", "settings"] if write => {
            s.require_for(u, "SETTINGS_EDIT", svc).await?;
            svc.backups.update_settings(b.clone()).await?;
            success("Backup settings saved")
        }
        ["backups", "scheduler"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            svc.backups.settings()["auto"].clone()
        }
        ["backups", "scheduler"] if write => {
            s.require_for(u, "SETTINGS_EDIT", svc).await?;
            svc.backups.update_settings(json!({"auto":b})).await?;
            success("Backup scheduler updated")
        }
        ["backups", "test-command"] if create => {
            s.require_for(u, "BACKUPS_CREATE", svc).await?;
            let h: Hook = serde_json::from_value(b["entry"].clone())
                .map_err(|e| ApiError::bad(e.to_string()))?;
            Hooks {
                before: vec![h.clone()],
                after: vec![],
            }
            .validate(false)?;
            let text = h
                .value
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| h.value.to_string());
            match h.r#type.as_str() {
                "send" => {
                    s.require_for(u, "CONSOLE_SEND", svc).await?;
                    let output = s
                        .backups
                        .send_capture(&text)
                        .await
                        .map_err(|e| ApiError::conflict(e.to_string()))?;
                    json!({"kind":"send","output":format!("Sent to console: {text}\n{}",output.join("\n")),"exit_code":0})
                }
                "comment" => json!({"kind":"comment","output":"Comment (ignored)","exit_code":0}),
                "wait" => {
                    json!({"kind":"wait","output":format!("Wait directive: {text}s (not executed in test)"),"exit_code":0})
                }
                "command" => {
                    s.require_for(u, "SETTINGS_EDIT", svc).await?;
                    match svc.backups.shell(&text, "test", 10).await {
                        Ok((code, output)) => {
                            json!({"kind":"command","output":output,"exit_code":code})
                        }
                        Err(e) => json!({"kind":"command","output":e.to_string(),"exit_code":-1}),
                    }
                }
                _ => return Err(ApiError::bad("Invalid command type")),
            }
        }
        ["backups", "include-items"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            let world = q
                .get("world")
                .ok_or_else(|| ApiError::bad("World required"))?;
            component(world)?;
            let path = safe_path(
                &svc.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            let mut items = Vec::new();
            let mut names = std::collections::BTreeSet::new();
            if tokio::fs::try_exists(&path).await? {
                let mut entries = tokio::fs::read_dir(path).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let name = entry.file_name().to_string_lossy().to_string();
                    names.insert(name.clone());
                    items.push(json!({"name":name,"is_dir":entry.file_type().await?.is_dir()}));
                }
            }
            for extra in [
                "behavior_packs",
                "resource_packs",
                "db",
                "level.dat",
                "levelname.txt",
                "world_behavior_packs.json",
                "world_resource_packs.json",
            ] {
                if !names.contains(extra) {
                    items.push(json!({"name":extra,"is_dir":(["behavior_packs","resource_packs","db"].contains(&extra))}));
                }
            }
            json!({"items":items})
        }
        ["backups", "folders"] if read => {
            s.require_for(u, "BACKUPS_VIEW", svc).await?;
            let base = q.get("base").map(String::as_str).unwrap_or("");
            let path = if base.is_empty() {
                tokio::fs::canonicalize(&svc.config.backups_dir).await?
            } else {
                safe_path(&svc.config.backups_dir, FsPath::new(base)).await?
            };
            let mut result = Vec::new();
            for name in files::dirs(&path).await? {
                result.push(json!({"name":name,"path":FsPath::new(base).join(&name)}));
            }
            json!({"base":base,"dirs":result})
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}

async fn backend_status(svc: &Arc<Instance>) -> Value {
    let settings = svc.server.settings.borrow().clone();
    let installed = svc.bds.installed_version().await;
    // Only surface a cached registry here; fetching stays on the versions endpoint.
    let latest = svc.bds.registry().await.ok().map(|r| r.release.latest);
    let binary_present = svc
        .config
        .bedrock_server_dir
        .join(if cfg!(windows) {
            "bedrock_server.exe"
        } else {
            "bedrock_server"
        })
        .exists();
    let install = serde_json::to_value(&*svc.bds.state.borrow()).unwrap_or(Value::Null);
    let endstone_installed = svc.endstone.installed().await;
    let endstone_binary_present =
        omnibedrock_server::endstone::endstone_binary(&svc.config).exists();
    let endstone_install =
        serde_json::to_value(&*svc.endstone.state.borrow()).unwrap_or(Value::Null);
    json!({
        "backend": settings.backend.as_str(),
        "vanilla_version": settings.vanilla_version,
        "running": svc.server.snapshot().status == Status::Running,
        "installed": installed,
        "latest": latest,
        "binary_present": binary_present,
        "install": install,
        "endstone": {
            "installed": endstone_installed,
            "binary_present": endstone_binary_present,
            "install": endstone_install,
        },
    })
}
async fn install_bds(svc: &Arc<Instance>, version: &str) -> Result<Value> {
    if svc.bds.state.borrow().phase != "idle" {
        return Err(ApiError::conflict("A BDS install is already running"));
    }
    if version != "latest" {
        let registry = svc.bds.registry().await?;
        if !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
    }
    let receiver = svc.bds.install(version).await?;
    let console = svc.server.console.clone();
    tokio::spawn(async move {
        match receiver.await {
            Ok(Ok(message)) => {
                console
                    .publish(format!("[LOCAL] {message}"), "panel", false)
                    .await;
            }
            result => {
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "installer stopped".into());
                tracing::error!(%error, "BDS install failed");
                console
                    .publish(
                        format!("[LOCAL] BDS install failed: {error}"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    });
    Ok(success(format!(
        "Installing BDS {version}; progress appears in the console"
    )))
}
/// Make sure a vanilla BDS binary exists before starting on the vanilla
/// backend; when nothing is installed, run the download inline so a plain
/// Start just works instead of failing with a "go install it" error.
async fn ensure_vanilla_installed(svc: &Arc<Instance>) -> Result<()> {
    let binary = svc.config.bedrock_server_dir.join(if cfg!(windows) {
        "bedrock_server.exe"
    } else {
        "bedrock_server"
    });
    if binary.exists() {
        return Ok(());
    }
    if svc.bds.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "A BDS install is already running; start the server again once it finishes",
        ));
    }
    let version = svc.server.settings.borrow().vanilla_version.clone();
    let requested = if version.is_empty() {
        "latest".to_owned()
    } else {
        version
    };
    svc.server
        .console
        .publish(
            format!("[LOCAL] Vanilla BDS is not installed — installing {requested} first…"),
            "panel",
            false,
        )
        .await;
    let receiver = svc
        .bds
        .install(&requested)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    match receiver.await {
        Ok(Ok(message)) => {
            // The installer may have written its own default properties;
            // re-pin the instance's registered port (both stacks).
            omnibedrock_server::instances::enforce_instance_port(&svc.config, svc.record().port)
                .await;
            svc.server
                .console
                .publish(format!("[LOCAL] {message}"), "panel", false)
                .await;
            Ok(())
        }
        Ok(Err(e)) => Err(ApiError::conflict(format!("BDS install failed: {e}"))),
        Err(_) => Err(ApiError::conflict("BDS installer stopped")),
    }
}

/// Make sure an Endstone launch candidate exists before starting on the
/// Endstone backend. When nothing is installed, run the toolchain bootstrap
/// inline (uv + managed CPython + Endstone) so a plain Start just works.
async fn ensure_endstone_toolchain(svc: &Arc<Instance>) -> Result<()> {
    let config = svc.config.clone();
    let ready = tokio::task::spawn_blocking(move || {
        omnibedrock_server::endstone::resolve_launch(&config).is_some()
    })
    .await
    .map_err(anyhow::Error::from)?;
    if ready {
        return Ok(());
    }
    if svc.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running; start the server again once it finishes",
        ));
    }
    svc.server
        .console
        .publish(
            "[LOCAL] Endstone toolchain not found — installing uv + Python + Endstone first…"
                .to_string(),
            "panel",
            false,
        )
        .await;
    let receiver = svc
        .endstone
        .install(None)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    match receiver.await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(ApiError::conflict(format!(
            "Endstone toolchain install failed: {e}"
        ))),
        Err(_) => Err(ApiError::conflict("Endstone toolchain installer stopped")),
    }
}
async fn install_endstone(svc: &Arc<Instance>, version: Option<String>) -> Result<Value> {
    if let Some(v) = &version {
        component(v)?;
    }
    if svc.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running",
        ));
    }
    let receiver = svc.endstone.install(version.clone()).await?;
    let console = svc.server.console.clone();
    let requested = version.unwrap_or_else(|| "latest".into());
    tokio::spawn(async move {
        match receiver.await {
            Ok(Ok(message)) => {
                console
                    .publish(format!("[LOCAL] {message}"), "panel", false)
                    .await;
            }
            result => {
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "installer stopped".into());
                tracing::error!(%error, "Endstone toolchain install failed");
                console
                    .publish(
                        format!("[LOCAL] Endstone install failed: {error}"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    });
    Ok(success(format!(
        "Installing Endstone toolchain ({requested}); progress appears in the console"
    )))
}
async fn switch_backend(svc: &Arc<Instance>, b: &Value) -> Result<Value> {
    let backend = Backend::parse(value_string(b, "backend")?)?;
    let mut settings = svc.server.settings.borrow().clone();
    if let Some(version) = b["version"].as_str() {
        component(version)?;
        let registry = svc.bds.registry().await?;
        if version != "latest" && !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
        settings.vanilla_version = version.into();
    }
    // Both runtimes install on demand, so a switch never dead-ends with a
    // "go install it first" error (mirrors the plain-Start bootstrap).
    if backend == Backend::Vanilla {
        ensure_vanilla_installed(svc).await?;
    }
    if backend == Backend::Endstone {
        ensure_endstone_toolchain(svc).await?;
    }
    let console = svc.server.console.clone();
    console
        .publish(
            format!(
                "[LOCAL] Switching '{}' to the {} backend — worlds, allowlist and \
                 player permissions are kept (both runtimes share the server directory)",
                svc.record().name,
                backend.as_str()
            ),
            "panel",
            false,
        )
        .await;
    let was_running = svc.server.snapshot().status == Status::Running;
    let lock = svc.operations.clone().lock_owned().await;
    if was_running && let Err(e) = svc.server.action("stop").await {
        drop(lock);
        return Err(ApiError::conflict(format!(
            "Stop before switch failed: {e}"
        )));
    }
    settings.backend = backend;
    let configured = svc.server.configure(settings).await;
    let restarted = if was_running {
        svc.server.action("start").await
    } else {
        Ok(String::new())
    };
    drop(lock);
    configured.map_err(|e| ApiError::conflict(e.to_string()))?;
    restarted.map_err(|e| ApiError::conflict(e.to_string()))?;
    console
        .publish(
            format!("[LOCAL] Backend switched to {}", backend.as_str()),
            "panel",
            false,
        )
        .await;
    Ok(success(format!(
        "Backend switched to {}; worlds and player permissions are kept; {}",
        backend.as_str(),
        if was_running {
            "server restarted on the new backend"
        } else {
            "applies on next start"
        }
    )))
}
async fn broadcaster(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let create = m == Method::POST;
    let bc = &svc.broadcaster;
    let data = match p {
        ["broadcaster", "status"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            json!({
                "status": bc.snapshot(),
                "jar": bc.jar_path(),
                "jar_exists": bc.jar_path().exists(),
                "java": bc.java.borrow().clone(),
                "config_exists": bc.config_path().exists(),
                "dir": bc.dir(),
                "accounts": bc.accounts().await,
                "device_codes": bc.device_codes(),
            })
        }
        ["broadcaster", "action"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let action = value_string(b, "action")?;
            if !["start", "stop", "restart"].contains(&action) {
                return Err(ApiError::bad("Invalid broadcaster action"));
            }
            success(
                bc.action(action)
                    .await
                    .map_err(|e| ApiError::conflict(e.to_string()))?,
            )
        }
        ["broadcaster", "command"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            bc.send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("Command sent")
        }
        ["broadcaster", "config"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                bc.read_config().await?,
            )
                .into_response());
        }
        ["broadcaster", "config"] if *m == Method::PUT => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let _lock = s.edits.lock().await;
            bc.write_config(value_string(b, "text")?)
                .await
                .map_err(|e| ApiError::bad(e.to_string()))?;
            success("Config saved; restart the broadcaster to apply")
        }
        ["broadcaster", "accounts"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            bc.accounts().await
        }
        ["broadcaster", "accounts"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let action = value_string(b, "action")?;
            if !["add", "remove"].contains(&action) {
                return Err(ApiError::bad("Invalid account action"));
            }
            let id = value_string(b, "id")?;
            component(id)?;
            bc.send_command(&format!("accounts {action} {id}"))
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!(
                "Sent 'accounts {action} {id}'; watch the console for the sign-in code"
            ))
        }
        ["broadcaster", "device-code"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            json!(bc.device_codes())
        }
        ["broadcaster", "screenshot"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            let account = q.get("account").map_or(Ok("main"), |a| component(a))?;
            let path = bc.screenshot_path(account);
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| ApiError::missing("No screenshot uploaded"))?;
            let size = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file);
            return Ok((
                [
                    (header::CONTENT_TYPE, "image/jpeg".to_string()),
                    (header::CONTENT_LENGTH, size.to_string()),
                ],
                Body::from_stream(stream),
            )
                .into_response());
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}

/// Stream an addon archive to a temp file, then install every pack it holds.
/// Handles `.mcpack` (single), `.mcaddon` (bundle), and `.zip` uploads.
async fn upload_addons(
    s: AppState,
    svc: Arc<Instance>,
    u: User,
    q: HashMap<String, String>,
    body: axum::body::Body,
) -> Result<Response> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    // The target world is only required for pack archives; world archives
    // (maps) become new worlds and ignore it.
    let world = q.get("world").cloned();
    let forced = match q.get("type").map(String::as_str).unwrap_or("auto") {
        "auto" => None,
        "behavior_packs" => Some("behavior_packs"),
        "resource_packs" => Some("resource_packs"),
        other => return Err(ApiError::bad(format!("Invalid pack type '{other}'"))),
    };
    let root = svc.config.bedrock_server_dir.join("worlds");
    if let Some(world) = &world {
        component(world)?;
        if !tokio::fs::try_exists(safe_path(&root, FsPath::new(world)).await?).await? {
            return Err(ApiError::missing("World not found"));
        }
    }
    let temp = std::env::temp_dir().join(format!("omnibedrock-addon-{}.zip", uuid::Uuid::new_v4()));
    const CAP: u64 = 512 * 1024 * 1024;
    let mut written: u64 = 0;
    let mut looks_like_zip = false;
    let mut file = tokio::fs::File::create(&temp).await?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::bad(format!("Upload interrupted: {e}")))?;
        if !looks_like_zip {
            looks_like_zip = chunk.starts_with(b"PK");
            if !looks_like_zip {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(ApiError::bad(
                    "Upload must be a .mcpack, .mcaddon, or .zip archive",
                ));
            }
        }
        written += chunk.len() as u64;
        if written > CAP {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(ApiError::bad("Archive exceeds the 512 MiB upload limit"));
        }
        file.write_all(&chunk).await?;
    }
    if !looks_like_zip {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(ApiError::bad("Upload body is empty"));
    }
    file.flush().await?;
    drop(file);
    let _lock = s.edits.lock().await;
    // Classify before installing: world archives import as new worlds, pack
    // archives install into the target world.
    let detect_target = temp.clone();
    let detected = tokio::task::spawn_blocking(move || addons::detect_archive(&detect_target))
        .await
        .map_err(anyhow::Error::from)?;
    let kind = match detected {
        Ok(kind) => kind,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(ApiError::conflict(e.to_string()));
        }
    };
    let result: anyhow::Result<Vec<InstalledPack>> = match kind {
        ArchiveKind::World => {
            let install_root = root.clone();
            let archive = temp.clone();
            tokio::task::spawn_blocking(move || {
                addons::install_world(&archive, &install_root, None).map(|pack| vec![pack])
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|inner| inner)
        }
        ArchiveKind::Addon => {
            let Some(world) = world.clone() else {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(ApiError::bad("World required"));
            };
            let install_root = root.clone();
            let archive = temp.clone();
            let upload_world = world;
            tokio::task::spawn_blocking(move || {
                addons::install_archive(&archive, &install_root, &upload_world, forced)
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|inner| inner)
        }
        ArchiveKind::Unknown => {
            let _ = tokio::fs::remove_file(&temp).await;
            // Same status the install path historically returned for zips
            // without a manifest, so existing clients keep their contract.
            return Err(ApiError::conflict(
                "archive holds neither a pack manifest nor a level.dat world",
            ));
        }
    };
    let _ = tokio::fs::remove_file(&temp).await;
    match result {
        Ok(installed) => {
            // Tag every freshly uploaded pack or world as a manual install so
            // the organizer can always tell Store content from local uploads.
            for pack in &installed {
                let dir = if pack.pack_type == "world" {
                    root.join(&pack.name)
                } else {
                    root.join(world.as_deref().unwrap_or_default())
                        .join(pack.pack_type)
                        .join(&pack.name)
                };
                let metadata = json!({
                    "source": "manual",
                    "installed_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Err(e) =
                    omnibedrock_server::addons::write_pack_metadata(&dir, metadata).await
                {
                    tracing::warn!(error = %e, pack = %pack.name, "failed to tag manual upload");
                }
            }
            // A world import: sync its pack orders to the versions the
            // bundled manifests actually declare and warn about packs the
            // world references but that are not on disk.
            if let Some(world_pack) = installed.iter().find(|p| p.pack_type == "world") {
                let world_dir = root.join(&world_pack.name);
                let report = tokio::task::spawn_blocking(move || {
                    omnibedrock_server::addons::sync_world_packs(&world_dir)
                })
                .await
                .unwrap_or_default();
                if !report.missing.is_empty() {
                    svc.server
                        .console
                        .publish(
                            format!(
                                "Imported world '{}': {} pack-order version(s) synced to installed manifests; missing pack uuids: {}",
                                world_pack.name,
                                report.synced,
                                report.missing.join(", ")
                            ),
                            "panel",
                            false,
                        )
                        .await;
                }
            }
            let names = installed
                .iter()
                .map(|p| format!("{} ({})", p.name, p.pack_type))
                .collect::<Vec<_>>()
                .join(", ");
            let scope = match &world {
                Some(world) => format!("{world}: {names}"),
                None => names.clone(),
            };
            s.audit(&u, "addons.upload", &scope, "addons").await.ok();
            Ok(data_response(json!({
                "success": true,
                "installed": installed,
                "message": format!("Installed {} pack(s): {}", installed.len(), names),
            })))
        }
        Err(e) => Err(ApiError::conflict(e.to_string())),
    }
}

/// CurseForge Store: browse, inspect, and install Bedrock content. Reads need
/// STORE_VIEW; installs are writes and need STORE_INSTALL.
async fn store(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let create = m == Method::POST;
    s.auth
        .require(u, if read { "STORE_VIEW" } else { "STORE_INSTALL" })
        .await?;
    let cf = &s.curseforge;
    let data = match p {
        ["store", "search"] if read => {
            let page = query_number(q, "page", 1)?.max(1);
            let page_size = query_number(q, "pageSize", 20)?.clamp(1, 50);
            let class = q
                .get("class")
                .map(String::as_str)
                .filter(|c| !c.is_empty() && *c != "all");
            let sort = q.get("sort").map(String::as_str).unwrap_or("relevancy");
            let version = q
                .get("version")
                .map(String::as_str)
                .filter(|v| !v.is_empty());
            let category = match q.get("category").map(String::as_str) {
                Some(c) if !c.is_empty() => Some(
                    c.parse::<u64>()
                        .map_err(|_| ApiError::bad("Invalid category id"))?,
                ),
                _ => None,
            };
            let (results, pagination) = cf
                .search(
                    class,
                    q.get("q").map(String::as_str).unwrap_or(""),
                    page,
                    page_size,
                    sort,
                    version,
                    category,
                )
                .await
                .map_err(store_error)?;
            json!({"results": results, "pagination": pagination, "page": page, "pageSize": page_size})
        }
        ["store", "meta"] if read => {
            // One bootstrap call: classes, sub-categories grouped per class,
            // and the Bedrock game versions for filter dropdowns.
            let (categories, versions) =
                tokio::try_join!(cf.categories(), cf.versions()).map_err(store_error)?;
            let classes: Value = json!(
                CLASSES
                    .iter()
                    .map(|(slug, id)| json!({"slug": slug, "id": id}))
                    .collect::<Vec<_>>()
            );
            let mut grouped: std::collections::BTreeMap<String, Vec<&curseforge::CfCategory>> =
                std::collections::BTreeMap::new();
            for category in &categories {
                // Sub-categories carry their class id; root classes do not.
                if let Some(class) = category.class_id {
                    grouped.entry(class.to_string()).or_default().push(category);
                }
            }
            let grouped: Value = Value::Object(
                grouped
                    .into_iter()
                    .map(|(class, list)| {
                        (
                            class,
                            json!(
                                list.iter()
                                    .map(|c| json!({"id": c.id, "name": c.name}))
                                    .collect::<Vec<_>>()
                            ),
                        )
                    })
                    .collect(),
            );
            json!({"classes": classes, "categories": grouped, "versions": versions})
        }
        ["store", "project", id] if read => {
            let id = parse_store_id(id)?;
            let project = cf.project(id).await.map_err(store_error)?;
            serde_json::to_value(project).map_err(anyhow::Error::from)?
        }
        ["store", "project", id, "description"] if read => {
            let id = parse_store_id(id)?;
            let description = cf.description(id).await.map_err(store_error)?;
            json!({"description": description})
        }
        ["store", "project", id, "files"] if read => {
            let id = parse_store_id(id)?;
            let page = query_number(q, "page", 1)?.max(1);
            let page_size = query_number(q, "pageSize", 20)?.clamp(1, 50);
            let version = q
                .get("version")
                .map(String::as_str)
                .filter(|v| !v.is_empty());
            let (files, total) = cf
                .files(id, page, page_size, version)
                .await
                .map_err(store_error)?;
            json!({"files": files, "total": total, "page": page, "pageSize": page_size})
        }
        ["store", "image"] if read => {
            let raw = q
                .get("url")
                .cloned()
                .ok_or_else(|| ApiError::bad("url required"))?;
            let (bytes, content_type) = cf.fetch_image(&raw).await.map_err(store_error)?;
            // Thumbnails are content-stable; cache hard in the browser on top
            // of the server-side disk cache.
            return Ok(Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, bytes.len().to_string())
                .header(
                    header::CACHE_CONTROL,
                    "public, max-age=604800, stale-while-revalidate=86400",
                )
                .body(Body::from(bytes))
                .map_err(anyhow::Error::from)?
                .into_response());
        }
        ["store", "install"] if create => install_from_store(s, svc, b).await?,
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}

fn parse_store_id(raw: &str) -> Result<u64> {
    raw.parse::<u64>()
        .map_err(|_| ApiError::bad("Invalid project id"))
}

fn query_number(q: &HashMap<String, String>, key: &str, default: u64) -> Result<u64> {
    match q.get(key) {
        Some(v) if !v.is_empty() => v
            .parse::<u64>()
            .map_err(|_| ApiError::bad(format!("Invalid {key}"))),
        _ => Ok(default),
    }
}

/// Surface CurseForge failures as actionable client errors instead of 500s.
fn store_error(e: anyhow::Error) -> ApiError {
    let text = e.to_string();
    if text.contains("CURSEFORGE_API_KEY") {
        ApiError::bad(text)
    } else {
        tracing::warn!(error = %text, "CurseForge request failed");
        ApiError::new(StatusCode::BAD_GATEWAY, text)
    }
}

/// Download a Store file, extract it safely, install it, and tag every pack
/// it produced with curseforge provenance metadata. Packs land in the chosen
/// world; world archives (maps) become new worlds alongside the existing ones.
async fn install_from_store(s: &AppState, svc: &Arc<Instance>, b: &Value) -> Result<Value> {
    let cf = &s.curseforge;
    if !cf.configured() {
        return Err(ApiError::bad(
            "CURSEFORGE_API_KEY is not configured; the Store cannot install",
        ));
    }
    let project_id = b["project_id"]
        .as_u64()
        .ok_or_else(|| ApiError::bad("'project_id' must be a number"))?;
    let file_id = b["file_id"]
        .as_u64()
        .ok_or_else(|| ApiError::bad("'file_id' must be a number"))?;
    let forced = match optional_string(b, "target", "auto") {
        "auto" => None,
        "behavior_packs" => Some("behavior_packs"),
        "resource_packs" => Some("resource_packs"),
        "world" => None,
        other => return Err(ApiError::bad(format!("Invalid install target '{other}'"))),
    };
    let force_world = b["target"].as_str() == Some("world");
    let world = value_string(b, "world")?.to_owned();
    component(&world)?;
    let root = svc.config.bedrock_server_dir.join("worlds");
    if !force_world && !tokio::fs::try_exists(safe_path(&root, FsPath::new(&world)).await?).await? {
        return Err(ApiError::missing("World not found"));
    }
    let (project, file) = tokio::try_join!(cf.project(project_id), cf.file(project_id, file_id))
        .map_err(store_error)?;
    let website = project
        .links
        .as_ref()
        .and_then(|l| l.website_url.clone())
        .unwrap_or_else(|| format!("https://www.curseforge.com/projects/{}", project.slug));
    let download_url = file
        .download_url
        .clone()
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            ApiError::conflict(format!(
                "CurseForge blocks direct downloads for '{}' (file {}). Download it manually from {website} and upload it on the Addons page",
                file.display_name, file.id
            ))
        })?;
    let temp = std::env::temp_dir().join(format!("omnibedrock-store-{}.zip", uuid::Uuid::new_v4()));
    if let Err(e) = cf.download_to(&download_url, &temp).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(store_error(e));
    }
    let _lock = s.edits.lock().await;
    let install_root = root.clone();
    let archive = temp.clone();
    let install_world_name = world.clone();
    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<InstalledPack>, bool)> {
            match addons::detect_archive(&archive)? {
                ArchiveKind::Addon => Ok((
                    addons::install_archive(&archive, &install_root, &install_world_name, forced)?,
                    false,
                )),
                ArchiveKind::World => Ok((
                    vec![addons::install_world(&archive, &install_root, None)?],
                    true,
                )),
                ArchiveKind::Unknown => {
                    anyhow::bail!("archive holds neither a pack manifest nor a level.dat world")
                }
            }
        })
        .await
        .map_err(anyhow::Error::from)?;
    let _ = tokio::fs::remove_file(&temp).await;
    let (installed, is_world) = match result {
        Ok(value) => value,
        Err(e) => return Err(ApiError::conflict(e.to_string())),
    };
    // A world import: sync its pack orders to the versions the bundled
    // manifests actually declare and warn about packs the world references
    // but that are not on disk.
    if is_world && let Some(world_pack) = installed.first() {
        let world_dir = root.join(&world_pack.name);
        let report = tokio::task::spawn_blocking(move || addons::sync_world_packs(&world_dir))
            .await
            .unwrap_or_default();
        if !report.missing.is_empty() {
            svc.server
                .console
                .publish(
                    format!(
                        "Imported world '{}': {} pack-order version(s) synced to installed manifests; missing pack uuids: {}",
                        world_pack.name,
                        report.synced,
                        report.missing.join(", ")
                    ),
                    "panel",
                    false,
                )
                .await;
        }
    }
    // Provenance sidecar for every installed pack or the new world.
    let metadata = json!({
        "source": "curseforge",
        "project_id": project_id,
        "project_slug": project.slug,
        "project_name": project.name,
        "authors": project.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "file_id": file.id,
        "file_name": file.file_name,
        "display_name": file.display_name,
        "download_url": download_url,
        "game_versions": file.game_versions,
        "installed_at": chrono::Utc::now().to_rfc3339(),
    });
    for pack in &installed {
        let dir = if is_world {
            root.join(&pack.name)
        } else {
            root.join(&world).join(pack.pack_type).join(&pack.name)
        };
        if let Err(e) =
            omnibedrock_server::addons::write_pack_metadata(&dir, metadata.clone()).await
        {
            tracing::warn!(error = %e, pack = %pack.name, "failed to write store metadata");
        }
    }
    let names = installed
        .iter()
        .map(|p| format!("{} ({})", p.name, p.pack_type))
        .collect::<Vec<_>>()
        .join(", ");
    // The generic dispatcher audit already records store.install:<slug>.
    Ok(json!({
        "success": true,
        "installed": installed,
        "as_world": is_world,
        "message": format!("Installed {} pack(s): {}", installed.len(), names),
    }))
}
