//! CurseForge Store endpoints: search, metadata, project details, images,
//! and installs.
use axum::{
    body::Body,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{component, safe_path},
};
use omnibedrock_server::{
    addons::{self, ArchiveKind, InstalledPack},
    curseforge::{self, CLASSES},
    instances::Instance,
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path as FsPath, sync::Arc};

use super::helpers::{data_response, optional_string, value_string};
use super::state::AppState;
use crate::auth::User;

/// CurseForge Store: browse, inspect, and install Bedrock content. Reads need
/// STORE_VIEW; installs are writes and need STORE_INSTALL.
pub(super) async fn store(
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
            let classes: Value = json!(
                CLASSES
                    .iter()
                    .map(|(slug, id)| json!({"slug": slug, "id": id}))
                    .collect::<Vec<_>>()
            );
            let mut grouped: std::collections::BTreeMap<String, Vec<&curseforge::CfCategory>> =
                std::collections::BTreeMap::new();
            for category in &categories {
                // Sub-categories carry their class id; root classes do not.
                if let Some(class) = category.class_id {
                    grouped.entry(class.to_string()).or_default().push(category);
                }
            }
            let grouped: Value = Value::Object(
                grouped
                    .into_iter()
                    .map(|(class, list)| {
                        (
                            class,
                            json!(
                                list.iter()
                                    .map(|c| json!({"id": c.id, "name": c.name}))
                                    .collect::<Vec<_>>()
                            ),
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
        ["store", "install"] if create => install_from_store(s, svc, b).await?,
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
async fn install_from_store(s: &AppState, svc: &Arc<Instance>, b: &Value) -> Result<Value> {
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
    let root = svc.config.bedrock_server_dir.join("worlds");
    if !force_world && !tokio::fs::try_exists(safe_path(&root, FsPath::new(&world)).await?).await? {
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
    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<InstalledPack>, bool)> {
            match addons::detect_archive(&archive)? {
                ArchiveKind::Addon => Ok((
                    addons::install_archive(&archive, &install_root, &install_world_name, forced)?,
                    false,
                )),
                ArchiveKind::World => Ok((
                    vec![addons::install_world(&archive, &install_root, None)?],
                    true,
                )),
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
    // A world import: sync its pack orders to the versions the bundled
    // manifests actually declare and warn about packs the world references
    // but that are not on disk.
    if is_world && let Some(world_pack) = installed.first() {
        let world_dir = root.join(&world_pack.name);
        let report = tokio::task::spawn_blocking(move || addons::sync_world_packs(&world_dir))
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
        if let Err(e) =
            omnibedrock_server::addons::write_pack_metadata(&dir, metadata.clone()).await
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
