//! Public pack-CDN downloads: `GET /packs/{bucket}/{uuid}_{version}.zip`.
//!
//! Unauthenticated on purpose — BDS hands these URLs to every joining client
//! (see `omnibedrock_server::addons::cdn`). Only zips the panel built for an
//! instance whose pack CDN is enabled exist under `<cache>/pack-cdn/`, names
//! are validated against a strict pattern, and nothing else is reachable.
//! The URL embeds the pack version, so responses are immutable and cacheable.
use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use omnibedrock_server::addons::cdn;

use super::state::AppState;

pub(super) async fn serve(
    State(state): State<AppState>,
    Path((bucket, file)): Path<(String, String)>,
) -> Response {
    if !cdn::is_bucket_name(&bucket) || !cdn::is_servable_name(&file) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = cdn::bucket_dir(&state.config.cache_dir, &bucket).join(&file);
    let Ok(handle) = tokio::fs::File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(meta) = handle.metadata().await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_LENGTH, meta.len().to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(handle)),
    )
        .into_response()
}
