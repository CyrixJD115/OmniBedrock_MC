//! Addon pack endpoints: listing, icons, manifests, renames, UUID edits, and
//! world pack ordering.
use axum::{
    body::Body,
    http::{Method, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{self, atomic_write, component, safe_path},
};
use omnibedrock_server::{addons, instances::Instance};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tokio_util::io::ReaderStream;

use super::helpers::{data_response, success, value_string};
use super::state::AppState;
use crate::auth::User;

pub(super) async fn addons(
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
    // Packs live inside a specific instance's world dirs, so the gate is
    // instance-scoped: on shared instances an access entry can carry it,
    // mirroring console/backups/properties. The default instance keeps the
    // legacy global-permission model.
    s.require_for(u, if read { "ADDONS_VIEW" } else { "ADDONS_MANAGE" }, svc)
        .await?;
    let root = svc.config.bedrock_server_dir.join("worlds");
    let _lock = if read {
        None
    } else {
        Some(s.edits.lock().await)
    };
    let pack_path = |value: String| {
        let root = root.clone();
        async move {
            let path = FsPath::new(&value);
            let relative = if path.is_absolute() {
                let canonical = tokio::fs::canonicalize(&root).await?;
                path.strip_prefix(canonical)
                    .map_err(|_| ApiError::bad("Addon path outside worlds"))?
                    .to_path_buf()
            } else {
                let lexical_root = &root;
                if let Ok(p) = path.strip_prefix(lexical_root) {
                    p.to_path_buf()
                } else {
                    path.to_path_buf()
                }
            };
            let path = safe_path(&root, &relative).await?;
            let parts = relative.components().collect::<Vec<_>>();
            if parts.len() != 3
                || !["behavior_packs", "resource_packs"]
                    .iter()
                    .any(|name| parts[1].as_os_str() == *name)
            {
                return Err(ApiError::bad("Invalid addon path"));
            }
            Ok::<_, ApiError>(path)
        }
    };
    let data = match p {
        ["addons"] if read => {
            // Force a full re-walk with ?refresh=1; otherwise packs whose
            // fingerprint is unchanged come straight from the parse cache.
            let refresh = q.contains_key("refresh");
            let mut result = json!({"behavior_packs":[],"resource_packs":[]});
            let mut live: HashSet<String> = HashSet::new();
            for world in files::dirs(&root).await? {
                let world_path = safe_path(&root, FsPath::new(&world)).await?;
                for pack_type in ["behavior_packs", "resource_packs"] {
                    let path = safe_path(&root, &PathBuf::from(&world).join(pack_type)).await?;
                    if !tokio::fs::try_exists(&path).await? {
                        continue;
                    }
                    for name in files::dirs(&path).await? {
                        let pack =
                            safe_path(&root, &PathBuf::from(&world).join(pack_type).join(&name))
                                .await?;
                        let pack_string = pack.to_string_lossy().into_owned();
                        live.insert(pack_string.clone());
                        let fingerprint =
                            omnibedrock_server::addon_cache::pack_fingerprint(&pack).await;
                        if !refresh
                            && let Some(item) =
                                s.addon_meta.lookup(&pack_string, &fingerprint).await
                        {
                            result[pack_type]
                                .as_array_mut()
                                .expect("pack array")
                                .push(item);
                            continue;
                        }
                        let manifest = read_manifest(&pack).await.ok();
                        let store_meta = omnibedrock_server::addons::read_pack_metadata(&pack);
                        let source = store_meta
                            .as_ref()
                            .and_then(|m| m["source"].as_str())
                            .unwrap_or("manual")
                            .to_owned();
                        let stats = tokio::task::spawn_blocking({
                            let pack = pack.clone();
                            move || omnibedrock_server::addons::pack_stats(&pack)
                        })
                        .await
                        .map_err(anyhow::Error::from)?;
                        // Marketplace packs put lang keys like `pack.name` in
                        // the manifest; resolve them from texts/*.lang.
                        let texts = omnibedrock_server::addons::read_texts(&pack).await;
                        let header = manifest.as_ref().and_then(|m| m.get("header"));
                        let display_name = omnibedrock_server::addons::resolve_lang(
                            &texts,
                            header.and_then(|h| h["name"].as_str()),
                        )
                        .unwrap_or_else(|| name.clone());
                        let description = omnibedrock_server::addons::resolve_lang(
                            &texts,
                            header.and_then(|h| h["description"].as_str()),
                        );
                        let item = json!({"name":display_name,"description":description,"path":pack,"world":world,"pack_type":pack_type,"uuid":manifest.as_ref().map(|v|v["header"]["uuid"].clone()).unwrap_or(json!("")),"version":manifest.as_ref().map(|v|v["header"]["version"].clone()).unwrap_or(json!([])),"valid":manifest.is_some(),"manifest":manifest,"size_bytes":stats.0,"modified":stats.1,"icon":stats.2,"icon_version":fingerprint,"source":source,"store":store_meta});
                        s.addon_meta
                            .store(pack_string, fingerprint, item.clone())
                            .await;
                        result[pack_type]
                            .as_array_mut()
                            .expect("pack array")
                            .push(item);
                    }
                }
                let _ = world_path;
            }
            s.addon_meta.retain_paths(&live).await;
            // No-op unless something changed (miss, prune, or forced refresh).
            s.addon_meta.flush().await;
            result
        }
        ["addons", "icon"] if read => {
            let raw = q
                .get("path")
                .cloned()
                .ok_or_else(|| ApiError::bad("Path required"))?;
            let pack = pack_path(raw).await?;
            let mut found = None;
            for candidate in ["pack_icon.png", "pack_icon.jpg", "Pack_Icon.png"] {
                let icon = pack.join(candidate);
                if tokio::fs::try_exists(&icon).await? {
                    found = Some((
                        icon,
                        if candidate.ends_with(".jpg") {
                            "image/jpeg"
                        } else {
                            "image/png"
                        },
                    ));
                    break;
                }
            }
            let Some((icon, content_type)) = found else {
                return Err(ApiError::missing("Pack has no icon"));
            };
            let file = tokio::fs::File::open(&icon).await?;
            let size = file.metadata().await?.len();
            // Icons are addressed with a fingerprint version param, so a
            // changed pack gets a new URL and a long max-age stays safe.
            return Ok((
                [
                    (header::CONTENT_TYPE, content_type.to_string()),
                    (header::CONTENT_LENGTH, size.to_string()),
                    (header::CACHE_CONTROL, "public, max-age=604800".to_string()),
                ],
                Body::from_stream(ReaderStream::new(file)),
            )
                .into_response());
        }
        ["addons", "delete"] if create => {
            let raw = value_string(b, "path")?.to_owned();
            let pack = pack_path(raw).await?;
            let canonical_root = tokio::fs::canonicalize(&root).await?;
            let world = pack
                .strip_prefix(&canonical_root)
                .map_err(|_| ApiError::bad("Addon path outside worlds"))?
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_default();
            let uuid = read_manifest(&pack)
                .await
                .ok()
                .and_then(|m| m["header"]["uuid"].as_str().map(str::to_owned))
                .unwrap_or_default();
            let name = pack
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            omnibedrock_server::addons::delete_pack(&pack, &root.join(&world), &uuid)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            s.audit(u, "addons.delete", &format!("{world}/{name}"), "addons")
                .await
                .ok();
            json!({"success":true,"message":format!("Deleted addon '{name}'")})
        }
        ["addons", "manifest"] if read => {
            let path = pack_path(
                q.get("path")
                    .cloned()
                    .ok_or_else(|| ApiError::bad("Path required"))?,
            )
            .await?;
            read_manifest(&path).await?
        }
        ["addons", "manifest"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            if !b["manifest"].is_object() {
                return Err(ApiError::bad("Manifest must be an object"));
            }
            write_manifest(&path, &b["manifest"]).await?;
            success("Manifest updated")
        }
        ["addons", "rename"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let name = component(value_string(b, "new_name")?)?;
            let destination = path
                .parent()
                .ok_or_else(|| ApiError::bad("Invalid addon"))?
                .join(name);
            if tokio::fs::try_exists(&destination).await? {
                return Err(ApiError::conflict("An addon with that name already exists"));
            }
            tokio::fs::rename(path, &destination).await?;
            json!({"success":true,"new_path":destination,"message":"Addon renamed"})
        }
        ["addons", "randomize-uuid"] if *m == Method::POST => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let mut manifest = read_manifest(&path).await?;
            let uuid = uuid::Uuid::new_v4().to_string();
            if !manifest["header"].is_object() {
                manifest["header"] = json!({});
            }
            manifest["header"]["uuid"] = json!(uuid);
            write_manifest(&path, &manifest).await?;
            json!({"success":true,"uuid":uuid,"message":"UUID randomized"})
        }
        ["addons", "change-uuid"] if *m == Method::PUT => {
            let path = pack_path(value_string(b, "path")?.into()).await?;
            let uuid = value_string(b, "uuid")?;
            uuid::Uuid::parse_str(uuid).map_err(|_| ApiError::bad("Invalid UUID"))?;
            let mut manifest = read_manifest(&path).await?;
            if !manifest["header"].is_object() {
                manifest["header"] = json!({});
            }
            manifest["header"]["uuid"] = json!(uuid);
            write_manifest(&path, &manifest).await?;
            success("UUID updated")
        }
        ["addons", "order", world, pack_type] if read || *m == Method::PUT => {
            component(world)?;
            if !["behavior_packs", "resource_packs"].contains(pack_type) {
                return Err(ApiError::bad("Invalid pack type"));
            }
            let path = safe_path(
                &root,
                &PathBuf::from(world).join(format!("world_{pack_type}.json")),
            )
            .await?;
            if read {
                match tokio::fs::read_to_string(path).await {
                    Ok(text) => serde_json::from_str(&text).map_err(|e| {
                        tracing::debug!(%e, "malformed world pack-order file");
                        ApiError::bad("World pack order file is not valid JSON")
                    })?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!([]),
                    Err(e) => return Err(e.into()),
                }
            } else {
                let uuids = b["uuids"]
                    .as_array()
                    .ok_or_else(|| ApiError::bad("uuids must be an array"))?;
                let world_dir = safe_path(&root, &PathBuf::from(world)).await?;
                let installed =
                    tokio::task::spawn_blocking(move || addons::manifest_versions(&world_dir))
                        .await
                        .unwrap_or_default();
                let mut order = Vec::new();
                let previous = tokio::fs::read_to_string(&path)
                    .await
                    .ok()
                    .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
                    .unwrap_or_default();
                for uid in uuids {
                    let uid = uid.as_str().ok_or_else(|| ApiError::bad("Invalid UUID"))?;
                    uuid::Uuid::parse_str(uid).map_err(|_| ApiError::bad("Invalid UUID"))?;
                    // BDS resolves packs by (uuid, exact version), so entries
                    // must carry the installed manifest's version — a fresh
                    // or stale entry would be silently dropped at boot.
                    let mut entry = previous
                        .iter()
                        .find(|v| v["pack_id"] == uid)
                        .cloned()
                        .unwrap_or(json!({"pack_id": uid}));
                    if let Some(version) = installed.get(uid) {
                        entry["version"] = json!(version);
                    } else if entry.get("version").is_none() {
                        entry["version"] = json!([1, 0, 0]);
                    }
                    order.push(entry);
                }
                atomic_write(
                    &path,
                    serde_json::to_string_pretty(&order)
                        .map_err(anyhow::Error::from)?
                        .as_bytes(),
                )
                .await?;
                success("Pack order updated")
            }
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}
async fn read_manifest(path: &FsPath) -> Result<Value> {
    let path = safe_path(path, FsPath::new("manifest.json")).await?;
    let text = tokio::fs::read_to_string(path).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        tracing::debug!(%e, "malformed manifest.json");
        ApiError::bad("Manifest is not valid JSON")
    })?;
    if !v.is_object() {
        return Err(ApiError::bad("Manifest must be an object"));
    }
    Ok(v)
}
async fn write_manifest(path: &FsPath, value: &Value) -> Result<()> {
    let path = safe_path(path, FsPath::new("manifest.json")).await?;
    atomic_write(
        &path,
        serde_json::to_string_pretty(value)
            .map_err(anyhow::Error::from)?
            .as_bytes(),
    )
    .await?;
    Ok(())
}
