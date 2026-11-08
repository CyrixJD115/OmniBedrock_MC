//! Instance-scoped file explorer: confined list/download/upload/edit/mkdir/
//! rename/delete over the instance's data tree (`instances/<id>/`, or the
//! legacy server root for the migrated default instance).
//!
//! SECURITY: every operation resolves through the ONE shared confinement
//! helper `omnibedrock_core::files::safe_path` — absolute paths, `..`
//! segments, and symlinks (which could bridge outside the root) are all
//! rejected there. Upload names go through `sanitize_filename`. Files the
//! panel manages through dedicated APIs (instance settings, broadcaster
//! credentials) are unreachable through the explorer in any direction,
//! including as part of a recursive delete of an ancestor directory.
use axum::{
    body::Body,
    extract::Multipart,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{atomic_write, safe_path, sanitize_filename},
};
use omnibedrock_server::instances::{DEFAULT_INSTANCE, Instance};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path as FsPath, path::PathBuf, sync::Arc};
use tokio_util::io::ReaderStream;

use super::helpers::{data_response, file_open_by_server, read_upload_field, success};
use super::state::AppState;
use crate::auth::User;

/// Per-listing entry cap so a directory with a million files cannot stall
/// the API; `truncated` tells the client the listing is partial.
pub(super) const MAX_LIST_ENTRIES: usize = 2000;
/// Hard upload cap for one explorer file.
pub(super) const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
/// Directories the running server caches at boot; writes under them need a
/// restart to take effect, so mutations there carry the hint.
const RESTART_DIRS: [&str; 2] = ["worlds", "plugins"];
/// Panel-managed files the explorer must never read, write, move, or delete
/// (directly or inside a recursive delete of an ancestor). Relative to the
/// instance root, `/`-separated.
const PROTECTED: [&str; 3] = [
    "server_settings.yaml",
    "backup_settings.yaml",
    "broadcaster/config.yml",
];

/// The directory every explorer path resolves against: `instances/<id>/` for
/// dedicated instances, the legacy server root for the migrated default
/// (whose data dir still holds panel secrets that must never be reachable).
fn instance_root(svc: &Instance) -> PathBuf {
    if svc.record().id == DEFAULT_INSTANCE {
        svc.config.bedrock_server_dir.clone()
    } else {
        svc.config.data_dir.join("instances").join(svc.record().id)
    }
}

async fn confine(svc: &Instance, rel: &str) -> Result<PathBuf> {
    safe_path(&instance_root(svc), FsPath::new(rel)).await
}

/// Refuse operations that would touch a panel-managed file directly or
/// inside a recursive delete/rename of an ancestor directory. An empty
/// relative path (the root itself) matches too.
fn protected_conflict(rel: &str) -> Result<()> {
    let norm = rel.replace('\\', "/");
    for p in PROTECTED {
        if norm.is_empty() || norm == p || p.starts_with(&format!("{norm}/")) {
            return Err(ApiError::forbidden(
                "That path is managed by a dedicated panel API and cannot be touched here",
            ));
        }
    }
    Ok(())
}

/// True when the target lands under a directory the running server caches
/// at boot (worlds/, plugins/) — the restart hint for the UI. Dedicated
/// instances keep the game server under `server/`; the migrated default's
/// root IS its server dir.
fn touches_game_data(svc: &Instance, target: &FsPath) -> bool {
    let Ok(rest) = target.strip_prefix(instance_root(svc)) else {
        return false;
    };
    let rel = if svc.record().id == DEFAULT_INSTANCE {
        rest
    } else {
        rest.strip_prefix("server").unwrap_or(rest)
    };
    match rel.components().next() {
        Some(c) => RESTART_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()),
        None => false,
    }
}

fn restart_hint(svc: &Instance, target: &FsPath) -> bool {
    touches_game_data(svc, target)
}

fn modified_secs(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// GET /instances/{id}/files?path=... — one directory listing, dirs first,
/// names ascending, capped at [`MAX_LIST_ENTRIES`] with a `truncated` flag.
pub(super) async fn list(svc: &Instance, q: &HashMap<String, String>) -> Result<Value> {
    let dir = confine(svc, q.get("path").map(String::as_str).unwrap_or("")).await?;
    if !tokio::fs::try_exists(&dir)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::missing("Directory not found"));
    }
    let meta = tokio::fs::metadata(&dir).await?;
    if !meta.is_dir() {
        return Err(ApiError::bad("Not a directory"));
    }
    let mut entries = Vec::new();
    let mut read = tokio::fs::read_dir(&dir).await?;
    let mut truncated = false;
    while let Some(entry) = read.next_entry().await? {
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        // Do not follow symlinks: they are forbidden for every other
        // operation, so present them inertly (never as a navigable dir).
        let m = entry.metadata().await?;
        let is_dir = m.is_dir();
        entries.push((
            is_dir,
            entry.file_name().to_string_lossy().into_owned(),
            m.len(),
            modified_secs(&m),
        ));
    }
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    Ok(json!({
        "path": q.get("path").map(String::as_str).unwrap_or(""),
        "entries": entries.iter().map(|(is_dir, name, size, modified)| json!({
            "name": name,
            "is_dir": is_dir,
            "size": size,
            "modified": modified,
        })).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

/// GET /instances/{id}/files/raw?path=... — stream one file out as a
/// download. Protected files are refused like every other operation.
pub(super) async fn raw_download(svc: &Instance, q: &HashMap<String, String>) -> Result<Response> {
    let Some(rel) = q.get("path").map(String::as_str).filter(|p| !p.is_empty()) else {
        return Err(ApiError::bad("'path' is required"));
    };
    protected_conflict(rel)?;
    let target = confine(svc, rel).await?;
    let meta = tokio::fs::metadata(&target).await?;
    if !meta.is_file() {
        return Err(ApiError::bad("Not a regular file"));
    }
    let file = tokio::fs::File::open(&target).await?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_DISPOSITION, content_disposition(&name)),
            (header::CONTENT_LENGTH, meta.len().to_string()),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

/// PUT /instances/{id}/files/raw?path=... — save text content to one file
/// (atomic replacement; refuses directories, protected files, and files the
/// running server holds open).
pub(super) async fn raw_save(
    s: &AppState,
    svc: &Arc<Instance>,
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Value> {
    let Some(rel) = q.get("path").map(String::as_str).filter(|p| !p.is_empty()) else {
        return Err(ApiError::bad("'path' is required"));
    };
    protected_conflict(rel)?;
    let content = b["content"]
        .as_str()
        .ok_or_else(|| ApiError::bad("'content' must be a string"))?;
    let target = confine(svc, rel).await?;
    if tokio::fs::try_exists(&target)
        .await
        .map_err(anyhow::Error::from)?
        && tokio::fs::metadata(&target).await?.is_dir()
    {
        return Err(ApiError::bad("That path is a directory"));
    }
    guard_write(svc, &target).await?;
    let _lock = s.edits.lock().await;
    atomic_write(&target, content.as_bytes()).await?;
    let mut v = success(format!("Saved '{rel}'"));
    v["restart_required"] = json!(restart_hint(svc, &target));
    Ok(v)
}

/// POST /instances/{id}/files/upload?path=<dir> — multipart upload of one
/// file into a directory. Same sanitization and duplicate rule as plugins.
pub(super) async fn upload(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    q: &HashMap<String, String>,
    multipart: Multipart,
) -> Result<Response> {
    let dir_rel = q.get("path").map(String::as_str).unwrap_or("");
    let (name, bytes) = read_upload_field(multipart, MAX_UPLOAD_BYTES, "File").await?;
    let clean = sanitize_filename(&name)?;
    let rel = if dir_rel.is_empty() {
        clean.to_owned()
    } else {
        format!(
            "{}/{}",
            dir_rel.replace('\\', "/").trim_end_matches('/'),
            clean
        )
    };
    protected_conflict(&rel)?;
    let target = confine(svc, &rel).await?;
    let overwrite = matches!(q.get("overwrite").map(String::as_str), Some("1" | "true"));
    if !overwrite
        && tokio::fs::try_exists(&target)
            .await
            .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::conflict(format!(
            "'{clean}' already exists in that directory — pass ?overwrite=true to replace it"
        )));
    }
    guard_write(svc, &target).await?;
    let _lock = s.edits.lock().await;
    atomic_write(&target, &bytes).await?;
    s.audit(u, "files.upload", &rel, "files").await.ok();
    let mut v = json!({
        "success": true,
        "message": format!("Uploaded '{clean}'"),
        "filename": clean,
    });
    v["restart_required"] = json!(restart_hint(svc, &target));
    Ok(data_response(v))
}

/// POST /instances/{id}/files/mkdir?path=... — create a directory (parents
/// included; every component was confined first).
pub(super) async fn mkdir(
    s: &AppState,
    svc: &Arc<Instance>,
    q: &HashMap<String, String>,
) -> Result<Value> {
    let Some(rel) = q.get("path").map(String::as_str).filter(|p| !p.is_empty()) else {
        return Err(ApiError::bad("'path' is required"));
    };
    protected_conflict(rel)?;
    let target = confine(svc, rel).await?;
    if tokio::fs::try_exists(&target)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::conflict(format!("'{rel}' already exists")));
    }
    let _lock = s.edits.lock().await;
    tokio::fs::create_dir_all(&target).await?;
    Ok(success(format!("Created '{rel}'")))
}

/// POST /instances/{id}/files/rename — move {from} to {to}; both confined,
/// the destination must not exist, and a directory cannot be moved into its
/// own subtree.
pub(super) async fn rename(s: &AppState, svc: &Arc<Instance>, b: &Value) -> Result<Value> {
    let from_rel = value_rel(b, "from")?;
    let to_rel = value_rel(b, "to")?;
    protected_conflict(&from_rel)?;
    protected_conflict(&to_rel)?;
    let from = confine(svc, &from_rel).await?;
    let to = confine(svc, &to_rel).await?;
    if !tokio::fs::try_exists(&from)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::missing("Source not found"));
    }
    if tokio::fs::try_exists(&to)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::conflict(format!("'{to_rel}' already exists")));
    }
    if to.starts_with(&from) {
        return Err(ApiError::bad("Cannot move a directory inside itself"));
    }
    guard_write(svc, &from).await?;
    let _lock = s.edits.lock().await;
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&from, &to).await?;
    let mut v = success(format!("Renamed '{from_rel}' to '{to_rel}'"));
    v["restart_required"] = json!(restart_hint(svc, &from) || restart_hint(svc, &to));
    Ok(v)
}

/// DELETE /instances/{id}/files?path=... — delete one file or an EMPTY
/// directory. Non-empty directories require `?recursive=true&confirm=true`;
/// anything that would take a panel-managed file with it is refused.
pub(super) async fn delete(
    s: &AppState,
    svc: &Arc<Instance>,
    q: &HashMap<String, String>,
) -> Result<Value> {
    let Some(rel) = q.get("path").map(String::as_str).filter(|p| !p.is_empty()) else {
        return Err(ApiError::bad(
            "'path' is required; the instance root itself cannot be deleted",
        ));
    };
    protected_conflict(rel)?;
    let target = confine(svc, rel).await?;
    if !tokio::fs::try_exists(&target)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::missing("Path not found"));
    }
    let meta = tokio::fs::symlink_metadata(&target).await?;
    if meta.is_symlink() {
        // safe_path already refuses symlinks; belt-and-braces for the window
        // between check and remove.
        return Err(ApiError::bad("Symlinks are not allowed"));
    }
    let _lock = s.edits.lock().await;
    if meta.is_dir() {
        let mut read = tokio::fs::read_dir(&target).await?;
        let empty = read.next_entry().await?.is_none();
        if !empty {
            let explicit = q.get("recursive").map(String::as_str) == Some("true")
                && q.get("confirm").map(String::as_str) == Some("true");
            if !explicit {
                return Err(ApiError::bad(
                    "Directory is not empty — pass ?recursive=true&confirm=true to delete it with all contents",
                ));
            }
            if file_open_by_server(svc, &target) {
                return Err(ApiError::conflict(
                    "The running server has this directory open; stop the server first",
                ));
            }
            tokio::fs::remove_dir_all(&target).await?;
        } else {
            tokio::fs::remove_dir(&target).await?;
        }
    } else {
        guard_write(svc, &target).await?;
        tokio::fs::remove_file(&target).await?;
    }
    let mut v = success(format!("Deleted '{rel}'"));
    v["restart_required"] = json!(touches_game_data(svc, &target));
    Ok(v)
}

/// The write-time guard shared by save/upload/rename/delete: refuse files
/// the running server currently holds open.
async fn guard_write(svc: &Instance, target: &FsPath) -> Result<()> {
    if target.exists() && file_open_by_server(svc, target) {
        return Err(ApiError::conflict(
            "The running server has this file open; stop the server before changing it",
        ));
    }
    Ok(())
}

fn value_rel(b: &Value, key: &str) -> Result<String> {
    // Trailing slashes are cosmetic; a LEADING slash stays so safe_path
    // rejects the absolute path outright.
    let rel = b[key]
        .as_str()
        .ok_or_else(|| ApiError::bad(format!("'{key}' must be a string")))?
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_owned();
    if rel.is_empty() {
        return Err(ApiError::bad(format!("'{key}' must name a path")));
    }
    Ok(rel)
}

/// `Content-Disposition` with an ASCII fallback plus an RFC 5987 encoded
/// form, so any filename survives any browser.
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}
