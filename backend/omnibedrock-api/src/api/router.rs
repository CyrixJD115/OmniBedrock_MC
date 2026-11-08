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
use super::docs;
use super::middleware as envelope;
use super::routes_list;
use super::state::AppState;

/// Baseline security headers on every response, API and embedded UI alike.
/// The SPA is same-origin (external scripts, Svelte inline styles,
/// data:/blob: images). script-src deliberately allows inline scripts:
/// SvelteKit's kit bootstrap and the theme pre-paint ship inline, and the
/// hash-pinned variant still blanked the dashboard even with byte-exact
/// hashes (2026-10-31). The panel sits behind login auth with audited,
/// throttled logins, so the inline allowance is an accepted, documented
/// trade-off — everything else stays locked to same-origin, and the
/// sandboxed srcdoc iframe on the Store page inherits the policy and blocks
/// scripts anyway.
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

static CSP_HEADER: std::sync::LazyLock<header::HeaderValue> = std::sync::LazyLock::new(|| {
    header::HeaderValue::from_static(
        "default-src 'self'; script-src 'self' 'unsafe-inline'; \
         style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         connect-src 'self'; font-src 'self'; frame-src 'self'; object-src 'none'; \
         base-uri 'self'; form-action 'self'; frame-ancestors 'self'",
    )
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
        // Machine-readable route inventory + endpoint document, public like
        // /health so tools and agents can discover the API without a source
        // dive. The inventory is derived from openapi.json at runtime.
        .route("/", get(routes_list::route_list))
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
        // The OpenAPI document and the human /api-docs page live at the host
        // root (outside the configurable prefix) at stable, linkable URLs.
        .route("/openapi.json", get(|| async { openapi_doc() }))
        .route("/api-docs", get(docs::api_docs))
        .nest(&prefix, api)
        // Everything that is not the API serves the embedded panel UI.
        .fallback(crate::bundle::serve)
        // JSON endpoints re-cap their own bodies at 2 MiB inside dispatch;
        // this router-level cap exists for the streaming multipart uploads
        // (explorer 100 MiB, plugins 25 MiB), which enforce per-file caps in
        // their handlers on top of the multipart overhead slack.
        .layer(DefaultBodyLimit::max(
            super::explorer::MAX_UPLOAD_BYTES as usize + 12 * 1024 * 1024,
        ))
        .layer(middleware::from_fn(security_headers))
        // Envelope + X-Request-Id run inside the trace span (so the id lands
        // in every request-scoped log line) and inside compression (so the
        // envelope rewrites the raw JSON before it is compressed).
        .layer(middleware::from_fn(envelope::request_context))
        .layer(CompressionLayer::new())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::info_span!(
                    "http",
                    method = %request.method(),
                    path = request.uri().path(),
                    request_id = tracing::field::Empty
                )
            }),
        )
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
