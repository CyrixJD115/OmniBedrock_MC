//! Backup endpoints: listing, create/restore/delete, downloads, settings,
//! scheduler, and hook testing.
use axum::{
    body::Body,
    http::{Method, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{self, component, safe_path},
};
use omnibedrock_server::{
    backups::{BackupRequest, Hook, Hooks},
    instances::Instance,
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path as FsPath, sync::Arc};
use tokio_util::io::ReaderStream;

use super::helpers::{data_response, success};
use super::state::AppState;
use crate::auth::User;

pub(super) async fn backups(
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
