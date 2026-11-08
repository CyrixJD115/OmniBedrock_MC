//! Endstone plugin management: list, upload, and delete the Python plugins
//! and wheels inside one instance's `server/plugins` directory. Endstone
//! loads that directory at boot, so every mutation carries a restart hint.
//!
//! Path confinement goes through the ONE shared helper
//! (`omnibedrock_core::files::safe_path`) and upload names through
//! `sanitize_filename` — the same pair the file explorer uses.
use axum::{
    extract::Multipart,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{atomic_write, safe_path, sanitize_filename},
};
use omnibedrock_server::instances::Instance;
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path as FsPath, sync::Arc};

use super::helpers::{data_response, file_open_by_server, read_upload_field, success};
use super::state::AppState;
use crate::auth::User;

/// Hard upload cap for one plugin file (a wheel can be tens of MiB; a
/// plugin .py never is).
pub(super) const MAX_PLUGIN_BYTES: u64 = 25 * 1024 * 1024;
/// Endstone loads `.py` plugins and `.whl` distributions from its plugins dir.
const PLUGIN_EXTENSIONS: [&str; 2] = ["py", "whl"];

pub(super) const RESTART_HINT: &str =
    "A server restart is required for plugin changes to take effect";

fn plugins_dir(svc: &Instance) -> std::path::PathBuf {
    svc.config.bedrock_server_dir.join("plugins")
}

async fn plugin_target(svc: &Instance, filename: &str) -> Result<std::path::PathBuf> {
    let name = sanitize_filename(filename)?;
    if FsPath::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|ext| !PLUGIN_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
    {
        return Err(ApiError::bad("Plugins must be a .py or .whl file"));
    }
    safe_path(&plugins_dir(svc), FsPath::new(name)).await
}

/// GET /instances/{id}/plugins — every plugin file with size and mtime. The
/// directory is created on demand so a fresh Endstone instance lists an
/// empty table instead of erroring.
pub(super) async fn list(svc: &Instance) -> Result<Value> {
    let dir = plugins_dir(svc);
    tokio::fs::create_dir_all(&dir).await?;
    let mut entries = Vec::new();
    let mut read = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = read.next_entry().await? {
        // Symlinked "plugins" are forbidden for every other operation; do
        // not present them as manageable files either.
        if entry.file_type().await?.is_symlink() {
            continue;
        }
        let meta = entry.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        entries.push(json!({
            "filename": entry.file_name().to_string_lossy(),
            "size": meta.len(),
            "modified": modified,
        }));
    }
    entries.sort_by(|a, b| a["filename"].as_str().cmp(&b["filename"].as_str()));
    Ok(json!({"plugins": entries}))
}

/// POST /instances/{id}/plugins — multipart upload of one plugin file.
/// `?overwrite=true` (or `=1`) replaces an existing plugin of the same name;
/// otherwise a duplicate is a conflict. Permission (PLUGIN_MANAGE) is gated
/// by the dispatcher before this runs.
pub(super) async fn upload(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    q: &HashMap<String, String>,
    multipart: Multipart,
) -> Result<Response> {
    let (name, bytes) = read_upload_field(multipart, MAX_PLUGIN_BYTES, "Plugin").await?;
    let target = plugin_target(svc, &name).await?;
    let overwrite = matches!(q.get("overwrite").map(String::as_str), Some("1" | "true"));
    if !overwrite
        && tokio::fs::try_exists(&target)
            .await
            .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::conflict(format!(
            "'{}' already exists — pass ?overwrite=true to replace it",
            target.file_name().and_then(|n| n.to_str()).unwrap_or(&name)
        )));
    }
    if file_open_by_server(svc, &target) {
        return Err(ApiError::conflict(
            "The running server has this file open; stop the server first",
        ));
    }
    atomic_write(&target, &bytes).await?;
    s.audit(u, "plugins.upload", &name, "plugins").await.ok();
    Ok(data_response(json!({
        "success": true,
        "message": format!("Uploaded '{name}'. {RESTART_HINT}"),
        "filename": name,
        "restart_required": true,
    })))
}

/// DELETE /instances/{id}/plugins/{filename} — remove one plugin. Directories
/// are refused (plugins are files; use the file explorer for directory work).
/// Auditing happens through the generic dispatch hook.
pub(super) async fn delete_plugin(svc: &Instance, filename: &str) -> Result<Response> {
    let target = plugin_target(svc, filename).await?;
    if !tokio::fs::try_exists(&target)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::missing("Plugin not found"));
    }
    if tokio::fs::metadata(&target).await?.is_dir() {
        return Err(ApiError::bad(
            "That path is a directory; plugins are single files",
        ));
    }
    if file_open_by_server(svc, &target) {
        return Err(ApiError::conflict(
            "The running server has this file open; stop the server first",
        ));
    }
    tokio::fs::remove_file(&target).await?;
    Ok(data_response(success(format!(
        "Deleted '{filename}'. {RESTART_HINT}"
    ))))
}

/// GET /instances/{id}/plugins/{filename} — download a plugin (e.g. to pull
/// one back off an instance before editing it locally).
pub(super) async fn download(svc: &Instance, filename: &str) -> Result<Response> {
    let target = plugin_target(svc, filename).await?;
    let bytes = tokio::fs::read(&target).await?;
    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )],
        bytes,
    )
        .into_response())
}
