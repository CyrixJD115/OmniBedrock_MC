//! API v1 compatibility layer. Services own process, auth, backup and filesystem logic.
use crate::{
    auth::{Auth, User},
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
    backups::{BackupRequest, Backups, Hook, Hooks},
    bds::{self, Bds},
    broadcaster::Broadcaster,
    curseforge::{self, CurseForge, CLASSES},
    endstone::Endstone,
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
    pub server: ServerManager,
    pub auth: Auth,
    pub backups: Backups,
    pub metrics: Metrics,
    pub bds: Bds,
    pub endstone: Endstone,
    pub broadcaster: Broadcaster,
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
        let server_job = ServerManager::new(config.clone(), CancellationToken::new());
        let (auth_result, server_result) = tokio::join!(Auth::new(config.clone()), server_job);
        let auth = auth_result?;
        let server = server_result?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            accounts = auth.users().await.as_array().map_or(0, Vec::len),
            login_enabled = auth.login_enabled(),
            "authentication ready"
        );
        let bds = Bds::new(config.clone(), server.console.clone(), shutdown.clone());
        let endstone = Endstone::new(config.clone(), server.console.clone(), shutdown.clone());
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
        let broadcaster = Broadcaster::new(config.clone(), shutdown.clone())?;
        let curseforge = CurseForge::new(
            config.curseforge_api_key.expose_secret(),
            Some(image_cache_dir),
        );
        let addon_meta =
            omnibedrock_server::addon_cache::AddonMetaCache::new(&config.cache_dir);
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
            "broadcaster manager ready (Java probe runs in the background)"
        );
        let backups = Backups::new(config.clone(), server.clone(), shutdown.clone()).await?;
        let metrics = Metrics::start(server.clone(), config.clone(), shutdown.clone());
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
            server,
            auth,
            backups,
            metrics,
            bds,
            endstone,
            broadcaster,
            curseforge,
            addon_meta,
            shutdown,
            operations: Arc::new(Mutex::new(())),
            edits: Arc::new(Mutex::new(())),
            audit_entries: Arc::new(Mutex::new(audit)),
        };
        state.backups.start_scheduler(state.operations.clone());
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "all services initialized"
        );
        Ok(state)
    }
    pub async fn audit(
        &self,
        user: &User,
        action: &str,
        detail: &str,
        category: &str,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let entry = json!({"timestamp":chrono::Utc::now().to_rfc3339(),"username":user.username,"action":action,"detail":detail,"category":category,"ip":""});
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
        return upload_addons(state, user, query, request.into_body()).await;
    }
    let bytes = to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| ApiError::bad("Request body too large"))?;
    // Screenshot uploads are raw image bytes, not the JSON envelope every other
    // endpoint uses, so they are handled before the generic body parse.
    if path == "broadcaster/screenshot" && method == Method::PUT {
        let user = state.auth.resolve_user(header_token.as_deref()).await?;
        state.auth.require(&user, "BROADCASTER_MANAGE").await?;
        let account = query.get("account").map_or(Ok("main"), |a| component(a))?;
        if bytes.is_empty() {
            return Err(ApiError::bad("Screenshot body is required"));
        }
        let jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
        let png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        if !jpeg && !png {
            return Err(ApiError::bad("Screenshot must be a JPEG or PNG image"));
        }
        let destination = state.broadcaster.screenshot_path(account);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        atomic_write(&destination, &bytes).await?;
        state
            .audit(
                &user,
                "broadcaster.screenshot_update",
                account,
                "broadcaster",
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
    let result = dispatch_authenticated(&state, &user, &method, &segments, &query, &body).await?;
    // Never include passwords, JWTs, or full edited documents in the audit trail.
    if method != Method::GET && result.status().is_success() {
        let (action, detail, category) = audit_action(&segments, &method, &body);
        if let Err(e) = state.audit(&user, &action, &detail, category).await {
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
async fn dispatch_authenticated(
    s: &AppState,
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
        ["roles"] if read => {
            s.auth.require(u, "USERS_VIEW").await?;
            s.auth.roles().await
        }
        ["roles", name] if read => {
            s.auth.require(u, "USERS_VIEW").await?;
            s.auth.role(name).await?
        }
        ["roles"] if create => {
            s.auth.require(u, "USERS_CREATE").await?;
            s.auth
                .mutate_role(value_string(b, "name")?, Some(b.clone()), true)
                .await?
        }
        ["roles", name] if write || delete => {
            s.auth
                .require(u, if delete { "USERS_DELETE" } else { "USERS_EDIT" })
                .await?;
            s.auth
                .mutate_role(name, if delete { None } else { Some(b.clone()) }, false)
                .await?
        }
        ["server", "status"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            json!(s.server.snapshot())
        }
        ["server", "backend"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            backend_status(s).await
        }
        ["server", "backend", "versions"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            serde_json::to_value(s.bds.registry().await?).map_err(anyhow::Error::from)?
        }
        ["server", "backend", "install"] if create => {
            s.auth.require(u, "SERVER_MANAGE").await?;
            install_bds(s, optional_string(b, "version", "latest")).await?
        }
        ["server", "backend", "endstone", "install"] if create => {
            s.auth.require(u, "SERVER_MANAGE").await?;
            install_endstone(s, b["version"].as_str().map(str::to_string)).await?
        }
        ["server", "backend", "switch"] if create => {
            s.auth.require(u, "SERVER_MANAGE").await?;
            switch_backend(s, b).await?
        }
        ["server", "action"] if create => {
            s.auth.require(u, "SERVER_MANAGE").await?;
            let action = value_string(b, "action")?.to_owned();
            // Starting on Endstone with no launcher available bootstraps the
            // panel toolchain inline instead of failing or crash-looping.
            if action == "start"
                && s.server.settings.borrow().backend == Backend::Endstone
                && let Err(e) = ensure_endstone_toolchain(s).await
            {
                json!({"success": false, "message": e.to_string()})
            } else {
                let server = s.server.clone();
                let lock = s.operations.clone().lock_owned().await;
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
            s.auth.require(u, "CONSOLE_SEND").await?;
            s.server
                .send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("")
        }
        ["performance", "metrics"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            s.metrics.latest.borrow().clone()
        }
        ["players"] if read => {
            s.auth.require(u, "PLAYERS_VIEW").await?;
            list_players(s).await?
        }
        ["players", "action"] if create => {
            let action = value_string(b, "action")?;
            s.auth
                .require(
                    u,
                    match action {
                        "kick" => "PLAYERS_KICK",
                        "ban" | "pardon" => "PLAYERS_BAN",
                        "op" | "deop" => "PLAYERS_OP",
                        _ => return Err(ApiError::bad("Invalid player action")),
                    },
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
            s.server
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
            s.auth.require(u, "SETTINGS_VIEW").await?;
            let mut v = json!(s.server.settings.borrow().clone());
            v["crash_count"] = json!(s.server.snapshot().crash_count);
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
            s.auth.require(u, "SETTINGS_EDIT").await?;
            let mut v = json!(s.server.settings.borrow().clone());
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
            success(s.server.configure(options).await?)
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
            s.auth.require(u, "PROPERTIES_VIEW").await?;
            json!(files::properties(&properties_text(s).await?))
        }
        ["properties", "raw"] if read => {
            s.auth.require(u, "PROPERTIES_VIEW").await?;
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                properties_text(s).await?,
            )
                .into_response());
        }
        ["properties", "raw"] if write => {
            s.auth.require(u, "PROPERTIES_EDIT").await?;
            let _lock = s.edits.lock().await;
            atomic_write(
                &safe_path(
                    &s.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                value_string(b, "text")?.as_bytes(),
            )
            .await?;
            success("Properties saved")
        }
        ["properties", key] if write => {
            s.auth.require(u, "PROPERTIES_EDIT").await?;
            let value = value_string(b, "value")?;
            files::validate_property(key, value)?;
            let _lock = s.edits.lock().await;
            let text = properties_text(s).await?;
            let mut entries = files::properties(&text);
            let mut found = false;
            for e in &mut entries {
                if e["key"] == *key {
                    e["value"] = json!(value);
                    found = true;
                }
            }
            if !found {
                return Err(ApiError::missing(format!("Property '{key}' not found")));
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
                    &s.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                text.as_bytes(),
            )
            .await?;
            success(format!("'{key}' updated to '{value}'"))
        }
        ["worlds"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            json!(files::dirs(&s.config.bedrock_server_dir.join("worlds")).await?)
        }
        ["worlds", world] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            component(world)?;
            let path = safe_path(
                &s.config.bedrock_server_dir.join("worlds"),
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
                &s.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            tokio::task::spawn_blocking(move || files::world_tree(&path))
                .await
                .map_err(anyhow::Error::from)??
        }
        ["files"] if read => {
            s.auth.require(u, "FILES_VIEW").await?;
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
            return addons(s, u, m, p, q, b).await;
        }
        ["store", ..] => {
            return store(s, u, m, p, q, b).await;
        }
        ["backups", ..] => {
            return backups(s, u, m, p, q, b).await;
        }
        ["broadcaster", ..] => {
            return broadcaster(s, u, m, p, q, b).await;
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}
async fn properties_text(s: &AppState) -> Result<String> {
    match tokio::fs::read_to_string(
        safe_path(
            &s.config.bedrock_server_dir,
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
async fn list_players(s: &AppState) -> Result<Value> {
    if s.server.snapshot().status != Status::Running {
        return Ok(json!({"players":[],"count":0}));
    }
    let console = &s.server.console;
    let mut changes = console.changed.subscribe();
    let mut cursor = Some(console.replay(None, 0).latest);
    s.server.send_command("list").await?;
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
    let root = s.config.bedrock_server_dir.join("worlds");
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
                        let store_meta =
                            omnibedrock_server::addons::read_pack_metadata(&pack);
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
                let mut order = Vec::new();
                let previous = tokio::fs::read_to_string(&path)
                    .await
                    .ok()
                    .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
                    .unwrap_or_default();
                for uid in uuids {
                    let uid = uid.as_str().ok_or_else(|| ApiError::bad("Invalid UUID"))?;
                    uuid::Uuid::parse_str(uid).map_err(|_| ApiError::bad("Invalid UUID"))?;
                    order.push(
                        previous
                            .iter()
                            .find(|v| v["pack_id"] == uid)
                            .cloned()
                            .unwrap_or(json!({"pack_id":uid,"version":[1,0,0]})),
                    );
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
            s.auth.require(u, "BACKUPS_VIEW").await?;
            json!(s.backups.worlds().await?)
        }
        ["backups"] if read => {
            s.auth.require(u, "BACKUPS_VIEW").await?;
            let backups = s.backups.list(q.get("world").cloned(), false).await?;
            json!({"total":backups.len(),"backups":backups})
        }
        ["backups", "trash"] if read => {
            s.auth.require(u, "BACKUPS_VIEW").await?;
            json!(s.backups.list(q.get("world").cloned(), true).await?)
        }
        ["backups", "create"] if create => {
            s.auth.require(u, "BACKUPS_CREATE").await?;
            let r: BackupRequest =
                serde_json::from_value(b.clone()).map_err(|e| ApiError::bad(e.to_string()))?;
            let lock = s.operations.clone().try_lock_owned().map_err(|_| {
                ApiError::conflict("A server operation or backup is already running")
            })?;
            let service = s.backups.clone();
            tokio::spawn(async move {
                let _lock = lock;
                service.run(r).await
            })
            .await
            .map_err(anyhow::Error::from)??
        }
        ["backups", "restore", world, filename] if create => {
            s.auth.require(u, "BACKUPS_RESTORE").await?;
            s.backups.move_trash(world, filename, true).await?;
            success(format!("Restored {filename}"))
        }
        ["backups", world, filename] if *m == Method::DELETE => {
            s.auth.require(u, "BACKUPS_DELETE").await?;
            s.backups.move_trash(world, filename, false).await?;
            success(format!("Moved {filename} to trash"))
        }
        ["backups", world, filename, "restore-to-world"] if create => {
            s.auth.require(u, "BACKUPS_RESTORE").await?;
            let lock = s.operations.clone().try_lock_owned().map_err(|_| {
                ApiError::conflict("A server operation or backup is already running")
            })?;
            let service = s.backups.clone();
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
            s.auth.require(u, "BACKUPS_VIEW").await?;
            let path = s.backups.path(world, filename, false).await?;
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
            s.auth.require(u, "BACKUPS_VIEW").await?;
            s.backups.settings()
        }
        ["backups", "settings"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            s.backups.update_settings(b.clone()).await?;
            success("Backup settings saved")
        }
        ["backups", "scheduler"] if read => {
            s.auth.require(u, "BACKUPS_VIEW").await?;
            s.backups.settings()["auto"].clone()
        }
        ["backups", "scheduler"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            s.backups.update_settings(json!({"auto":b})).await?;
            success("Backup scheduler updated")
        }
        ["backups", "test-command"] if create => {
            s.auth.require(u, "BACKUPS_CREATE").await?;
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
                    s.auth.require(u, "CONSOLE_SEND").await?;
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
                    s.auth.require(u, "SETTINGS_EDIT").await?;
                    match s.backups.shell(&text, "test", 10).await {
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
            s.auth.require(u, "BACKUPS_VIEW").await?;
            let world = q
                .get("world")
                .ok_or_else(|| ApiError::bad("World required"))?;
            component(world)?;
            let path = safe_path(
                &s.config.bedrock_server_dir.join("worlds"),
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
            s.auth.require(u, "BACKUPS_VIEW").await?;
            let base = q.get("base").map(String::as_str).unwrap_or("");
            let path = if base.is_empty() {
                tokio::fs::canonicalize(&s.config.backups_dir).await?
            } else {
                safe_path(&s.config.backups_dir, FsPath::new(base)).await?
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

async fn backend_status(s: &AppState) -> Value {
    let settings = s.server.settings.borrow().clone();
    let installed = s.bds.installed_version().await;
    // Only surface a cached registry here; fetching stays on the versions endpoint.
    let latest = s.bds.registry().await.ok().map(|r| r.release.latest);
    let binary_present = s
        .config
        .bedrock_server_dir
        .join(if cfg!(windows) {
            "bedrock_server.exe"
        } else {
            "bedrock_server"
        })
        .exists();
    let install = serde_json::to_value(&*s.bds.state.borrow()).unwrap_or(Value::Null);
    let endstone_installed = s.endstone.installed().await;
    let endstone_binary_present = omnibedrock_server::endstone::endstone_binary(&s.config)
        .exists();
    let endstone_install =
        serde_json::to_value(&*s.endstone.state.borrow()).unwrap_or(Value::Null);
    json!({
        "backend": settings.backend.as_str(),
        "vanilla_version": settings.vanilla_version,
        "running": s.server.snapshot().status == Status::Running,
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
async fn install_bds(s: &AppState, version: &str) -> Result<Value> {
    if s.bds.state.borrow().phase != "idle" {
        return Err(ApiError::conflict("A BDS install is already running"));
    }
    if version != "latest" {
        let registry = s.bds.registry().await?;
        if !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
    }
    let receiver = s.bds.install(version).await?;
    let console = s.server.console.clone();
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
/// Make sure an Endstone launch candidate exists before starting on the
/// Endstone backend. When nothing is installed, run the toolchain bootstrap
/// inline (uv + managed CPython + Endstone) so a plain Start just works.
async fn ensure_endstone_toolchain(s: &AppState) -> Result<()> {
    let config = s.config.clone();
    let ready = tokio::task::spawn_blocking(move || {
        omnibedrock_server::endstone::resolve_launch(&config).is_some()
    })
    .await
    .map_err(anyhow::Error::from)?;
    if ready {
        return Ok(());
    }
    if s.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running; start the server again once it finishes",
        ));
    }
    s.server
        .console
        .publish(
            "[LOCAL] Endstone toolchain not found — installing uv + Python + Endstone first…"
                .to_string(),
            "panel",
            false,
        )
        .await;
    let receiver = s
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
async fn install_endstone(s: &AppState, version: Option<String>) -> Result<Value> {
    if let Some(v) = &version {
        component(v)?;
    }
    if s.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running",
        ));
    }
    let receiver = s.endstone.install(version.clone()).await?;
    let console = s.server.console.clone();
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
async fn switch_backend(s: &AppState, b: &Value) -> Result<Value> {
    let backend = Backend::parse(value_string(b, "backend")?)?;
    let mut settings = s.server.settings.borrow().clone();
    if let Some(version) = b["version"].as_str() {
        component(version)?;
        let registry = s.bds.registry().await?;
        if version != "latest" && !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
        settings.vanilla_version = version.into();
    }
    if backend == Backend::Vanilla {
        let binary = s.config.bedrock_server_dir.join(if cfg!(windows) {
            "bedrock_server.exe"
        } else {
            "bedrock_server"
        });
        if !binary.exists() {
            return Err(ApiError::conflict(
                "Vanilla BDS is not installed; install a version first",
            ));
        }
    }
    if backend == Backend::Endstone {
        ensure_endstone_toolchain(s).await?;
    }
    let was_running = s.server.snapshot().status == Status::Running;
    let lock = s.operations.clone().lock_owned().await;
    if was_running && let Err(e) = s.server.action("stop").await {
        drop(lock);
        return Err(ApiError::conflict(format!(
            "Stop before switch failed: {e}"
        )));
    }
    settings.backend = backend;
    let configured = s.server.configure(settings).await;
    let restarted = if was_running {
        s.server.action("start").await
    } else {
        Ok(String::new())
    };
    drop(lock);
    configured.map_err(|e| ApiError::conflict(e.to_string()))?;
    restarted.map_err(|e| ApiError::conflict(e.to_string()))?;
    Ok(success(format!(
        "Backend switched to {}; {}",
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
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let create = m == Method::POST;
    let bc = &s.broadcaster;
    let data = match p {
        ["broadcaster", "status"] if read => {
            s.auth.require(u, "BROADCASTER_VIEW").await?;
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
            s.auth.require(u, "BROADCASTER_MANAGE").await?;
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
            s.auth.require(u, "BROADCASTER_MANAGE").await?;
            bc.send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("Command sent")
        }
        ["broadcaster", "config"] if read => {
            s.auth.require(u, "BROADCASTER_VIEW").await?;
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                bc.read_config().await?,
            )
                .into_response());
        }
        ["broadcaster", "config"] if *m == Method::PUT => {
            s.auth.require(u, "BROADCASTER_MANAGE").await?;
            let _lock = s.edits.lock().await;
            bc.write_config(value_string(b, "text")?)
                .await
                .map_err(|e| ApiError::bad(e.to_string()))?;
            success("Config saved; restart the broadcaster to apply")
        }
        ["broadcaster", "accounts"] if read => {
            s.auth.require(u, "BROADCASTER_VIEW").await?;
            bc.accounts().await
        }
        ["broadcaster", "accounts"] if create => {
            s.auth.require(u, "BROADCASTER_MANAGE").await?;
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
            s.auth.require(u, "BROADCASTER_VIEW").await?;
            json!(bc.device_codes())
        }
        ["broadcaster", "screenshot"] if read => {
            s.auth.require(u, "BROADCASTER_VIEW").await?;
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
    u: User,
    q: HashMap<String, String>,
    body: axum::body::Body,
) -> Result<Response> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let world = q
        .get("world")
        .cloned()
        .ok_or_else(|| ApiError::bad("World required"))?;
    component(&world)?;
    let forced = match q.get("type").map(String::as_str).unwrap_or("auto") {
        "auto" => None,
        "behavior_packs" => Some("behavior_packs"),
        "resource_packs" => Some("resource_packs"),
        other => return Err(ApiError::bad(format!("Invalid pack type '{other}'"))),
    };
    let root = s.config.bedrock_server_dir.join("worlds");
    if !tokio::fs::try_exists(safe_path(&root, FsPath::new(&world)).await?).await? {
        return Err(ApiError::missing("World not found"));
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
    let install_root = root.clone();
    let archive = temp.clone();
    let upload_world = world.clone();
    let result = tokio::task::spawn_blocking(move || {
        omnibedrock_server::addons::install_archive(&archive, &install_root, &upload_world, forced)
    })
    .await
    .map_err(anyhow::Error::from)?;
    let _ = tokio::fs::remove_file(&temp).await;
    match result {
        Ok(installed) => {
            // Tag every freshly uploaded pack as a manual install so the
            // organizer can always tell Store content from local uploads.
            for pack in &installed {
                if pack.pack_type == "world" {
                    continue;
                }
                let dir = root.join(&world).join(pack.pack_type).join(&pack.name);
                let metadata = json!({
                    "source": "manual",
                    "installed_at": chrono::Utc::now().to_rfc3339(),
                });
                if let Err(e) = omnibedrock_server::addons::write_pack_metadata(&dir, metadata).await
                {
                    tracing::warn!(error = %e, pack = %pack.name, "failed to tag manual upload");
                }
            }
            let names = installed
                .iter()
                .map(|p| format!("{} ({})", p.name, p.pack_type))
                .collect::<Vec<_>>()
                .join(", ");
            s.audit(&u, "addons.upload", &format!("{world}: {names}"), "addons")
                .await
                .ok();
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
            let classes: Value = json!(CLASSES
                .iter()
                .map(|(slug, id)| json!({"slug": slug, "id": id}))
                .collect::<Vec<_>>());
            let mut grouped: std::collections::BTreeMap<String, Vec<&curseforge::CfCategory>> =
                std::collections::BTreeMap::new();
            for category in &categories {
                // Sub-categories carry their class id; root classes do not.
                if let Some(class) = category.class_id {
                    grouped
                        .entry(class.to_string())
                        .or_default()
                        .push(category);
                }
            }
            let grouped: Value = Value::Object(
                grouped
                    .into_iter()
                    .map(|(class, list)| {
                        (
                            class,
                            json!(list
                                .iter()
                                .map(|c| json!({"id": c.id, "name": c.name}))
                                .collect::<Vec<_>>()),
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
        ["store", "install"] if create => {
            install_from_store(s, b).await?
        }
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
async fn install_from_store(s: &AppState, b: &Value) -> Result<Value> {
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
    let root = s.config.bedrock_server_dir.join("worlds");
    if !force_world
        && !tokio::fs::try_exists(safe_path(&root, FsPath::new(&world)).await?).await?
    {
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
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<InstalledPack>, bool)> {
        match addons::detect_archive(&archive)? {
            ArchiveKind::Addon => Ok((
                addons::install_archive(&archive, &install_root, &install_world_name, forced)?,
                false,
            )),
            ArchiveKind::World => {
                Ok((vec![addons::install_world(&archive, &install_root, None)?], true))
            }
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
        if let Err(e) = omnibedrock_server::addons::write_pack_metadata(&dir, metadata.clone()).await
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
