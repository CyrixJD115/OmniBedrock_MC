//! Panel data-directory file endpoints: safe config-file paths and the file
//! listing.
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{component, safe_path},
};
use serde_json::{Value, json};
use std::path::{Path as FsPath, PathBuf};

use super::state::AppState;

pub(super) async fn config_file_path(s: &AppState, name: &str) -> Result<PathBuf> {
    component(name)?;
    // Files with dedicated APIs (auth, roles, instance registry, settings)
    // must never be reachable through the generic editor: several of them
    // hold secrets or authority the generic FILES_EDIT gate does not carry.
    if [
        "users.yaml",
        "roles.yaml",
        "jwt_secret",
        "console_lock_state.yaml",
        "backup_settings.yaml",
        "server_settings.yaml",
        "instances.yaml",
        "panel_settings.yaml",
        "bds_version.json",
        "endstone_toolchain.json",
        "omnibedrock.db",
        "omnibedrock.db-wal",
        "omnibedrock.db-shm",
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
pub(super) async fn list_files(s: &AppState) -> Result<Value> {
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
