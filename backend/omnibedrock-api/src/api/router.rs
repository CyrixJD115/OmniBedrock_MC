//! Top-level HTTP router: health, the versioned API prefix, the CORS,
//! compression, and tracing layers, and the embedded UI fallback.
use crate::websocket;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde_json::json;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

use super::dispatch::dispatch;
use super::state::AppState;

/// Baseline security headers on every response, API and embedded UI alike.
/// The SPA is same-origin (external scripts, Svelte inline styles,
/// data:/blob: images). SvelteKit ships a few INLINE scripts (theme
/// pre-paint + kit bootstrap), so script-src allows those by content hash —
/// build.rs computes the hashes from the embedded index.html and a changed
/// UI forces a recompile — keeping 'unsafe-inline' out of the policy; the
/// sandboxed srcdoc iframe on the Store page inherits it and blocks scripts
/// anyway.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, csp_header());
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}

/// The embedded UI's inline-script content hashes, space-separated
/// `'sha256-…'` sources; empty when the tree carries no inline scripts (the
/// placeholder UI) or was built with `OMNI_SKIP_FRONTEND_BUILD=1`.
pub(crate) fn content_security_policy() -> String {
    let hashes = option_env!("OMNI_INLINE_SCRIPT_HASHES")
        .unwrap_or("")
        .trim();
    let script_src = if hashes.is_empty() {
        "script-src 'self'".to_owned()
    } else {
        format!("script-src 'self' {hashes}")
    };
    format!(
        "default-src 'self'; {script_src}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; \
         frame-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
         frame-ancestors 'self'"
    )
}

static CSP_HEADER: std::sync::LazyLock<header::HeaderValue> = std::sync::LazyLock::new(|| {
    header::HeaderValue::from_str(&content_security_policy()).unwrap_or_else(|_| {
        header::HeaderValue::from_static("default-src 'self'; script-src 'self'")
    })
});
fn csp_header() -> header::HeaderValue {
    CSP_HEADER.clone()
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
        // Embedded-UI fingerprint (matches the boot log's "UI build:" line),
        // public so a stale open page can detect a new backend and reload.
        .route(
            "/build-id",
            get(|| async { Json(json!({"build_id": crate::bundle::UI_BUILD_HASH})) }),
        )
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
        .layer(middleware::from_fn(security_headers))
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

/// The static endpoint inventory served at `{prefix}/openapi.json`. Kept as a
/// checked-in document (not generated) so each entry can carry the real
/// permission gate and body keys; update it alongside `dispatch_authenticated`.
fn openapi_doc() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        include_str!("../../openapi.json"),
    )
        .into_response()
}
