//! Addon archive upload handler: streams the archive to a temp file,
//! classifies it, and installs the packs or world it holds.
use axum::response::Response;
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{component, safe_path},
};
use omnibedrock_server::{
    addons::{self, ArchiveKind, InstalledPack},
    instances::Instance,
};
use serde_json::json;
use std::{collections::HashMap, path::Path as FsPath, sync::Arc};

use super::helpers::data_response;
use super::state::AppState;
use crate::auth::User;

/// Stream an addon archive to a temp file, then install every pack it holds.
/// Handles `.mcpack` (single), `.mcaddon` (bundle), and `.zip` uploads.
pub(super) async fn upload_addons(
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
