//! Top-level HTTP router: health, the versioned API prefix, the CORS,
//! compression, and tracing layers, and the embedded UI fallback.
use crate::websocket;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{Method, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde_json::json;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};

use super::dispatch::dispatch;
use super::state::AppState;

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
