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
            // Manifest names often carry § format codes ("§7[v5.1]§r WAILA");
            // show clean text and keep the original as raw_name.
            for pack_type in ["behavior_packs", "resource_packs"] {
                for item in result[pack_type].as_array_mut().into_iter().flatten() {
                    if let Some(name) = item["name"].as_str().filter(|n| n.contains('§')) {
                        let clean = omnibedrock_server::compat::strip_codes(name)
                            .trim()
                            .to_owned();
                        item["raw_name"] = json!(name);
                        item["name"] = json!(clean);
                    }
                }
            }
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
        // What a joining client downloads for the active (or ?world=) world:
        // active resource packs with sizes, unpaired leftovers, and version
        // pins that disagree with the manifests.
        ["addons", "delivery"] if read => {
            let props =
                tokio::fs::read_to_string(svc.config.bedrock_server_dir.join("server.properties"))
                    .await
                    .unwrap_or_default();
            let prop = |key: &str| {
                props.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix(key)
                        .and_then(|v| v.strip_prefix('='))
                        .map(|v| v.trim().to_owned())
                })
            };
            let world = match q.get("world") {
                Some(w) => w.clone(),
                None => prop("level-name").unwrap_or_else(|| "Bedrock level".into()),
            };
            component(&world)?;
            let world_dir = safe_path(&root, &PathBuf::from(&world)).await?;
            let required = prop("texturepack-required").map(|v| v.eq_ignore_ascii_case("true"));
            let report =
                tokio::task::spawn_blocking(move || addons::delivery_report(&world_dir, required))
                    .await
                    .map_err(anyhow::Error::from)?;
            serde_json::to_value(report).map_err(anyhow::Error::from)?
        }
        // Compatibility checker: log + manifest + script analysis, its
        // Markdown export, and ContentLog cleanup.
        ["addons", "compat"] if read => super::compat::report(s, svc, q).await?,
        ["addons", "compat", "export"] if read => {
            return super::compat::export(s, svc, q).await;
        }
        ["addons", "compat", "content-logs"] if create => {
            let result = super::compat::prune_content_logs(svc, b).await?;
            s.audit(u, "addons.compat.prune", "ContentLog files", "addons")
                .await
                .ok();
            result
        }
        // Optional pack CDN (cdn_config.json): status / configure / self-test.
        ["addons", "cdn"] if read => cdn_status(s, svc).await?,
        ["addons", "cdn"] if *m == Method::PUT => {
            use omnibedrock_server::addons::cdn;
            let enabled = b["enabled"].as_bool().unwrap_or(false);
            let raw_url = b["base_url"].as_str().unwrap_or_default();
            let base_url = if raw_url.trim().is_empty() {
                if enabled {
                    return Err(ApiError::bad(
                        "A public base URL is required to enable the pack CDN",
                    ));
                }
                String::new()
            } else {
                cdn::normalize_base_url(raw_url).map_err(|e| ApiError::bad(e.to_string()))?
            };
            let settings = cdn::CdnSettings { enabled, base_url };
            let server_dir = svc.config.bedrock_server_dir.clone();
            let cache_dir = svc.config.cache_dir.clone();
            let saved = settings.clone();
            let build =
                tokio::task::spawn_blocking(move || -> anyhow::Result<Option<cdn::CdnBuild>> {
                    cdn::save_settings(&server_dir, &saved)?;
                    if saved.enabled {
                        cdn::build(&server_dir, &cache_dir, &saved).map(Some)
                    } else {
                        cdn::disable(&server_dir, &cache_dir);
                        Ok(None)
                    }
                })
                .await
                .map_err(anyhow::Error::from)?
                .map_err(|e| ApiError::bad(format!("{e:#}")))?;
            let mut status = cdn_status(s, svc).await?;
            status["build"] = json!(build);
            status["message"] = json!(match &build {
                Some(b) if b.written => format!(
                    "Pack CDN ready: {} packs. Restart the server so BDS loads cdn_config.json.",
                    b.files.len()
                ),
                Some(b) => format!(
                    "Saved, but {} pack(s) are missing on disk, so no cdn_config.json was written.",
                    b.missing.len()
                ),
                None => "Pack CDN disabled; restart the server to apply.".into(),
            });
            status
        }
        ["addons", "cdn", "test"] if create => {
            use omnibedrock_server::addons::cdn;
            let server_dir = svc.config.bedrock_server_dir.clone();
            let settings = cdn::load_settings(&server_dir);
            if !settings.enabled {
                return Err(ApiError::bad("Enable the pack CDN first"));
            }
            let base = cdn::normalize_base_url(&settings.base_url)
                .map_err(|e| ApiError::bad(e.to_string()))?;
            let dir = cdn::bucket_dir(&svc.config.cache_dir, &cdn::bucket(&server_dir));
            let mut zips: Vec<(String, u64)> = std::fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            name.ends_with(".zip")
                                .then(|| (name, e.metadata().map_or(0, |m| m.len())))
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Smallest file: the check is about reachability and headers.
            zips.sort_by_key(|(_, size)| *size);
            let Some((name, size)) = zips.into_iter().next() else {
                return Err(ApiError::bad(
                    "No pack zips built yet; save the settings first",
                ));
            };
            let url = format!("{base}/packs/{}/{name}", cdn::bucket(&server_dir));
            json!(cdn::self_test(&url, size).await)
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

/// Pack CDN status for the Mods page: settings, coverage, what BDS said
/// about cdn_config.json at the last boot, and address suggestions.
async fn cdn_status(s: &AppState, svc: &Arc<Instance>) -> Result<Value> {
    use omnibedrock_server::addons::cdn;
    let server_dir = svc.config.bedrock_server_dir.clone();
    let cache_dir = svc.config.cache_dir.clone();
    let state = tokio::task::spawn_blocking(move || cdn::state(&server_dir, &cache_dir))
        .await
        .map_err(anyhow::Error::from)?;
    // The newest BDS verdict about the CDN config in this run's console.
    // BDS only speaks up on problems ("No CDN config file found", a
    // mismatch, a parse error). A boot that reached "Server started." after
    // the panel wrote the file, with none of those, means the file was found
    // and accepted.
    let tail = svc.server.console.tail(4000);
    let mut bds: Option<(&str, String)> = None;
    let mut wrote_config = false;
    for e in tail.iter() {
        let line = e.line.as_str();
        if line.contains("Pack CDN:") {
            wrote_config = line.contains("offered for HTTP download");
            bds = None;
        } else if line.contains("CDN config packIdVersions") {
            bds = Some(("loaded", line.to_owned()));
        } else if line.contains("Mismatch detected between CDN config") {
            bds = Some(("mismatch", line.to_owned()));
        } else if line.contains("Failed to parse CDN config") {
            bds = Some(("parse_error", line.to_owned()));
        } else if line.contains("No CDN config file found") {
            bds = Some(("absent", line.to_owned()));
        } else if line.contains("Server started.") && wrote_config && bds.is_none() {
            bds = Some((
                "accepted",
                "Bedrock started with cdn_config.json and reported no problem with it".to_owned(),
            ));
        }
    }
    // A public address the operator already configured for joins.
    let props = tokio::fs::read_to_string(svc.config.bedrock_server_dir.join("server.properties"))
        .await
        .unwrap_or_default();
    let public_ip = props.lines().find_map(|l| {
        let v = l.trim().strip_prefix("server-udp-ports=")?;
        let entry = v.split(',').next()?.trim();
        let parts: Vec<&str> = entry.rsplitn(3, ':').collect();
        (parts.len() == 3 && !parts[2].is_empty()).then(|| parts[2].to_owned())
    });
    let port = s.config.port;
    let suggestions: Vec<String> = public_ip
        .iter()
        .map(|ip| format!("http://{ip}:{port}"))
        .collect();
    Ok(json!({
        "state": state,
        "bds": bds.map(|(kind, line)| json!({"status": kind, "line": line})),
        "suggestions": suggestions,
        "panel_port": port,
    }))
}
