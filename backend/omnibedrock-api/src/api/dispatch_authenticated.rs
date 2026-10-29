//! The authenticated route table: one match over HTTP method and path
//! segments dispatching every endpoint to its per-area handler.
use axum::{
    http::{Method, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{self, atomic_write, component, safe_path},
};
use omnibedrock_server::{
    instances::{DEFAULT_INSTANCE, Deployment, Instance},
    server::{Backend, ServerSettings},
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path as FsPath, sync::Arc};

use super::addons::addons;
use super::backups::backups;
use super::broadcaster::broadcaster;
use super::files::{config_file_path, list_files};
use super::helpers::{data_response, optional_string, success, value_string};
use super::instances::{create_instance, update_instance, usage_json};
use super::players::list_players;
use super::properties::properties_text;
use super::server::{
    backend_status, ensure_endstone_toolchain, ensure_vanilla_installed, install_bds,
    install_endstone, switch_backend,
};
use super::state::AppState;
use super::store::store;
use crate::auth::User;

#[allow(clippy::too_many_lines)]
pub(super) async fn dispatch_authenticated(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    m: &Method,
    p: &[&str],
    q: &HashMap<String, String>,
    b: &Value,
) -> Result<Response> {
    let read = m == Method::GET;
    let write = m == Method::PUT;
    let create = m == Method::POST;
    let delete = m == Method::DELETE;
    let data = match p {
        ["auth", "me"] if read => s.auth.me(u).await,
        ["auth", "users"] if read => {
            s.auth.require(u, "USERS_VIEW").await?;
            s.auth.users().await
        }
        ["auth", "users"] if create => {
            s.auth.require(u, "USERS_CREATE").await?;
            s.auth
                .mutate_user(u, value_string(b, "username")?, Some(b.clone()), true)
                .await?
        }
        ["auth", "users", name] if write || delete => {
            s.auth
                .require(u, if delete { "USERS_DELETE" } else { "USERS_EDIT" })
                .await?;
            s.auth
                .mutate_user(u, name, if delete { None } else { Some(b.clone()) }, false)
                .await?
        }
        // Role metadata is readable by every signed-in user: the UI renders
        // role chips with names and colors across pages. Mutations are gated.
        ["roles"] if read => s.auth.roles().await,
        ["roles", name] if read => s.auth.role(name).await?,
        ["roles"] if create => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (role, renamed) = s
                .auth
                .mutate_role(u, value_string(b, "name")?, Some(b.clone()), true)
                .await?;
            if let Some(old) = renamed {
                s.instances
                    .rename_access_role(&old, role["name"].as_str().unwrap_or_default())
                    .await?;
            }
            s.audit(
                u,
                "roles.create",
                role["name"].as_str().unwrap_or("role"),
                "settings",
            )
            .await?;
            role
        }
        ["roles", name] if write => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (role, renamed) = s.auth.mutate_role(u, name, Some(b.clone()), false).await?;
            if let Some(old) = renamed {
                s.instances
                    .rename_access_role(&old, role["name"].as_str().unwrap_or_default())
                    .await?;
            }
            s.audit(u, "roles.update", name, "settings").await?;
            role
        }
        ["roles", name] if delete => {
            s.auth.require(u, "ROLES_MANAGE").await?;
            let (result, _) = s.auth.mutate_role(u, name, None, false).await?;
            // Memberships in instance access entries are dropped with the role.
            s.instances.strip_access_role(name).await?;
            s.audit(u, "roles.delete", name, "settings").await?;
            result
        }
        ["server", "status"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            json!(svc.server.snapshot())
        }
        ["server", "backend"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            backend_status(svc).await
        }
        ["server", "backend", "versions"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            serde_json::to_value(s.bds.registry().await?).map_err(anyhow::Error::from)?
        }
        ["server", "backend", "endstone", "versions"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            serde_json::to_value(s.endstone.registry().await?).map_err(anyhow::Error::from)?
        }
        ["server", "backend", "install"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            install_bds(svc, optional_string(b, "version", "latest")).await?
        }
        ["server", "backend", "endstone", "install"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            install_endstone(svc, b["version"].as_str().map(str::to_string)).await?
        }
        ["server", "backend", "switch"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            switch_backend(svc, b).await?
        }
        ["server", "action"] if create => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            let action = value_string(b, "action")?.to_owned();
            // Starting on Endstone with no launcher available bootstraps the
            // panel toolchain inline instead of failing or crash-looping.
            // Copy the backend out: the settings read guard is not Send and
            // must not live across the bootstrap awaits.
            let backend = svc.server.settings.borrow().backend;
            let deployment = svc.record().deployment;
            let bootstrap = if action == "start" {
                match backend {
                    Backend::Vanilla => ensure_vanilla_installed(svc).await,
                    // Docker containers ship their own Endstone runtime; only
                    // the vanilla binary is still fetched into the bind mount.
                    Backend::Endstone if deployment == Deployment::Docker => Ok(()),
                    Backend::Endstone => ensure_endstone_toolchain(svc).await,
                }
            } else {
                Ok(())
            };
            if let Err(e) = bootstrap {
                json!({"success": false, "message": e.to_string()})
            } else {
                let server = svc.server.clone();
                let lock = svc.operations.clone().lock_owned().await;
                let result = tokio::spawn(async move {
                    let _lock = lock;
                    server.action(&action).await
                })
                .await
                .map_err(anyhow::Error::from)?;
                match result {
                    Ok(message) => success(message),
                    Err(e) => json!({"success":false,"message":e.to_string()}),
                }
            }
        }
        ["console", "command"] if create => {
            s.require_for(u, "CONSOLE_SEND", svc).await?;
            svc.server
                .send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("")
        }
        ["console", "tail"] if read => {
            s.require_for(u, "CONSOLE_VIEW", svc).await?;
            let limit = q
                .get("lines")
                .map_or(Ok(200), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid 'lines'"))?;
            if !(1..=2000).contains(&limit) {
                return Err(ApiError::bad("'lines' must be 1..2000"));
            }
            let events = svc.server.console.tail(limit);
            if q.get("format").map(String::as_str) == Some("text") {
                let body = events
                    .iter()
                    .map(|e| e.line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok((
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    body + "\n",
                )
                    .into_response());
            }
            let owned = events
                .iter()
                .map(|e| (**e).clone())
                .collect::<Vec<omnibedrock_server::server::ConsoleEvent>>();
            json!({
                "instance": svc.record().id,
                "latest": owned.last().map(|e| e.sequence).unwrap_or(0),
                "events": owned,
            })
        }
        ["performance", "metrics"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            s.instance_metrics(&svc.record().id)
        }
        ["players"] if read => {
            s.require_for(u, "PLAYERS_VIEW", svc).await?;
            list_players(svc).await?
        }
        ["players", "action"] if create => {
            let action = value_string(b, "action")?;
            s.require_for(
                u,
                match action {
                    "kick" => "PLAYERS_KICK",
                    "ban" | "pardon" => "PLAYERS_BAN",
                    "op" | "deop" => "PLAYERS_OP",
                    _ => return Err(ApiError::bad("Invalid player action")),
                },
                svc,
            )
            .await?;
            let target = value_string(b, "target")?;
            if target.contains(['\n', '\r', '\0', '"']) || target.is_empty() {
                return Err(ApiError::bad("Invalid player name"));
            }
            let reason = optional_string(b, "reason", "");
            let command = format!(
                "{action} \"{target}\"{}",
                if action == "kick" && !reason.is_empty() {
                    format!(" {reason}")
                } else {
                    String::new()
                }
            );
            svc.server
                .send_command(&command)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!("Sent '{command}' to server"))
        }
        ["settings"] if read => {
            s.auth.require(u, "SETTINGS_VIEW").await?;
            let mut v = serde_json::to_value(s.config.as_ref()).map_err(anyhow::Error::from)?;
            if let Some(v) = v.as_object_mut() {
                v.remove("endstone_python");
                v.remove("endstone_bin");
            }
            v
        }
        ["settings", "server"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            let mut v = json!(svc.server.settings.borrow().clone());
            v["crash_count"] = json!(svc.server.snapshot().crash_count);
            v
        }
        ["settings", "panel"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            let enabled = b["login_enabled"]
                .as_bool()
                .ok_or_else(|| ApiError::bad("'login_enabled' must be a boolean"))?;
            s.auth.set_login_enabled(enabled).await?;
            success(if enabled {
                "Login required"
            } else {
                "Login disabled; the panel now serves every visitor as Admin"
            })
        }
        ["settings", "curseforge"] if read => {
            s.auth.require(u, "SETTINGS_VIEW").await?;
            let hint = s.auth.curseforge_key_hint().await;
            json!({
                "configured": s.curseforge.configured(),
                "key_hint": hint,
                "source": if hint.is_some() { "settings" } else { "env" },
            })
        }
        ["settings", "curseforge"] if write => {
            s.auth.require(u, "SETTINGS_EDIT").await?;
            let key = b
                .get("api_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            s.auth.set_curseforge_key(key).await?;
            // An empty key clears the override and falls back to the .env value.
            if key.is_empty() {
                s.curseforge
                    .set_api_key(s.config.curseforge_api_key.expose_secret());
            } else {
                s.curseforge.set_api_key(key);
            }
            json!({
                "success": true,
                "message": if key.is_empty() {
                    "CurseForge API key cleared; falling back to CURSEFORGE_API_KEY from .env"
                } else {
                    "CurseForge API key saved — the Store picks it up immediately"
                },
                "configured": s.curseforge.configured(),
            })
        }
        ["settings", "server"] if write => {
            s.require_for(u, "SERVER_MANAGE", svc).await?;
            let mut v = json!(svc.server.settings.borrow().clone());
            let obj = b
                .as_object()
                .ok_or_else(|| ApiError::bad("Settings must be an object"))?;
            for (key, val) in obj {
                if !val.is_null() && v.get(key).is_some() {
                    v[key] = val.clone();
                }
            }
            let options: ServerSettings =
                serde_json::from_value(v).map_err(|e| ApiError::bad(e.to_string()))?;
            options
                .validate()
                .map_err(|e| ApiError::bad(e.to_string()))?;
            success(svc.server.configure(options).await?)
        }
        ["audit"] if read => {
            s.auth.require(u, "AUDIT_VIEW").await?;
            let limit = q
                .get("limit")
                .map_or(Ok(100), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid limit"))?;
            if !(1..=500).contains(&limit) {
                return Err(ApiError::bad("Limit must be 1..500"));
            }
            let offset = q
                .get("offset")
                .map_or(Ok(0), |v| v.parse::<usize>())
                .map_err(|_| ApiError::bad("Invalid offset"))?;
            json!(
                s.audit_entries
                    .lock()
                    .await
                    .iter()
                    .rev()
                    .filter(|e| q.get("username").is_none_or(|v| e["username"] == *v)
                        && q.get("category").is_none_or(|v| e["category"] == *v)
                        && q.get("action")
                            .is_none_or(|v| e["action"].as_str().is_some_and(|a| a.contains(v))))
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>()
            )
        }
        ["properties"] if read => {
            s.require_for(u, "PROPERTIES_VIEW", svc).await?;
            json!(files::properties(&properties_text(svc).await?))
        }
        ["properties", "raw"] if read => {
            s.require_for(u, "PROPERTIES_VIEW", svc).await?;
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                properties_text(svc).await?,
            )
                .into_response());
        }
        ["properties", "raw"] if write => {
            s.require_for(u, "PROPERTIES_EDIT", svc).await?;
            let text = value_string(b, "text")?;
            // The raw editor must not become a side door around the
            // per-key server-udp-ports checks: validate the embedded value
            // against the same backend matrix before anything is written.
            {
                let settings = svc.server.settings.borrow().clone();
                let mut transport = "nethernet";
                let mut port: u16 = 19132;
                let mut udp_ports: Option<String> = None;
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with('#') {
                        continue;
                    }
                    let Some((key, raw_value)) = line.split_once('=') else {
                        continue;
                    };
                    // Cut inline comments, which the raw editor preserves.
                    let value = raw_value.split('#').next().unwrap_or("").trim();
                    match key.trim() {
                        "transport" => transport = value,
                        "server-port" => {
                            if let Ok(p) = value.parse() {
                                port = p;
                            }
                        }
                        "server-udp-ports" => udp_ports = Some(value.to_string()),
                        _ => {}
                    }
                }
                if let Some(udp) = udp_ports {
                    omnibedrock_server::instances::validate_udp_ports_value(
                        settings.backend,
                        transport,
                        port,
                        &udp,
                    )
                    .map_err(ApiError::bad)?;
                }
            }
            let _lock = s.edits.lock().await;
            // Writes must land in the SAME file the reads come from: the
            // X-Instance-scoped server dir, not the panel-level default.
            atomic_write(
                &safe_path(
                    &svc.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                text.as_bytes(),
            )
            .await?;
            success("Properties saved")
        }
        ["properties", key] if write => {
            s.require_for(u, "PROPERTIES_EDIT", svc).await?;
            let value = value_string(b, "value")?;
            files::validate_property(key, value)?;
            let _lock = s.edits.lock().await;
            let text = properties_text(svc).await?;
            // server-udp-ports means different things per backend/transport;
            // reject values that would silently bind nothing instead of
            // letting a bad forward rule surface as "clients can't join".
            if *key == "server-udp-ports" {
                let settings = svc.server.settings.borrow().clone();
                let transport = text
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("transport="))
                    .map(str::trim)
                    .unwrap_or("nethernet");
                let port = text
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("server-port="))
                    .and_then(|v| v.trim().parse::<u16>().ok())
                    .unwrap_or(19132);
                omnibedrock_server::instances::validate_udp_ports_value(
                    settings.backend,
                    transport,
                    port,
                    value,
                )
                .map_err(ApiError::bad)?;
            }
            let mut entries = files::properties(&text);
            let mut found = false;
            for e in &mut entries {
                if e["key"] == *key {
                    e["value"] = json!(value);
                    found = true;
                }
            }
            if !found {
                // A key missing from the file is appended, not rejected, so
                // every property stays configurable even before the server
                // has ever written it.
                entries.push(json!({
                    "key": key,
                    "value": value,
                    "comment": "",
                    "inline_comment": ""
                }));
            }
            let text = entries
                .iter()
                .map(|e| {
                    if e["comment"].as_str().is_some_and(|s| !s.is_empty()) {
                        e["comment"].as_str().unwrap_or_default().to_string()
                    } else if e["key"] == "" {
                        String::new()
                    } else {
                        let comment = e["inline_comment"].as_str().unwrap_or_default();
                        format!(
                            "{}={}{}",
                            e["key"].as_str().unwrap_or_default(),
                            e["value"].as_str().unwrap_or_default(),
                            if comment.is_empty() {
                                String::new()
                            } else {
                                format!(" {comment}")
                            }
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            atomic_write(
                &safe_path(
                    &svc.config.bedrock_server_dir,
                    FsPath::new("server.properties"),
                )
                .await?,
                text.as_bytes(),
            )
            .await?;
            success(format!("'{key}' updated to '{value}'"))
        }
        ["worlds"] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            json!(files::dirs(&svc.config.bedrock_server_dir.join("worlds")).await?)
        }
        ["worlds", world] if read => {
            s.require_for(u, "SERVER_VIEW", svc).await?;
            component(world)?;
            let path = safe_path(
                &svc.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            if !tokio::fs::try_exists(&path).await? {
                return Err(ApiError::missing("World not found"));
            }
            let name = world.to_string();
            tokio::task::spawn_blocking(move || {
                let mut size = 0;
                let mut count = 0;
                for entry in walkdir::WalkDir::new(&path).follow_links(false) {
                    let entry = entry.map_err(anyhow::Error::from)?;
                    if entry.file_type().is_file() {
                        size += entry.metadata().map_err(anyhow::Error::from)?.len();
                        count += 1;
                    }
                }
                Ok::<_, anyhow::Error>(
                    json!({"name":name,"path":path,"size_bytes":size,"files":count}),
                )
            })
            .await
            .map_err(anyhow::Error::from)??
        }
        ["worlds", world, "contents"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            component(world)?;
            let path = safe_path(
                &svc.config.bedrock_server_dir.join("worlds"),
                FsPath::new(world),
            )
            .await?;
            tokio::task::spawn_blocking(move || files::world_tree(&path))
                .await
                .map_err(anyhow::Error::from)??
        }
        ["files"] if read => {
            s.require_for(u, "FILES_VIEW", svc).await?;
            list_files(s).await?
        }
        ["files", filename] if read || write || delete => {
            s.auth
                .require(u, if read { "FILES_VIEW" } else { "FILES_EDIT" })
                .await?;
            let path = config_file_path(s, filename).await?;
            if read {
                json!({"name":filename,"content":tokio::fs::read_to_string(path).await?})
            } else {
                let _lock = s.edits.lock().await;
                if write {
                    atomic_write(&path, value_string(b, "content")?.as_bytes()).await?;
                } else {
                    tokio::fs::remove_file(path).await?;
                }
                success("File updated")
            }
        }
        ["addons", ..] => {
            return addons(s, svc, u, m, p, q, b).await;
        }
        ["store", ..] => {
            return store(s, svc, u, m, p, q, b).await;
        }
        ["backups", ..] => {
            return backups(s, svc, u, m, p, q, b).await;
        }
        ["broadcaster", ..] => {
            return broadcaster(s, svc, u, m, p, q, b).await;
        }
        // ---- Multi-instance management ----
        ["instances"] if read => {
            // Every user sees the instances they can access.
            let mut list = Vec::new();
            for instance in s.instances.list().await {
                let record = instance.record();
                if record.id != DEFAULT_INSTANCE && !s.auth.can_access_instance(u, &record).await {
                    continue;
                }
                let mut item = instance.summary();
                item["usage"] = usage_json(&s.instance_metrics(&record.id));
                list.push(item);
            }
            json!(list)
        }
        ["instances"] if create => {
            s.auth.require(u, "INSTANCES_CREATE").await?;
            create_instance(s, u, b).await?
        }
        ["instances", id] if read => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let mut item = instance.summary();
            item["usage"] = usage_json(&s.instance_metrics(&instance.record().id));
            // Live scan: pack uuids referenced by world order files or pack
            // dependencies with nothing on disk, so the UI can warn before a
            // boot logs per-pack "was not found" noise.
            let warnings = tokio::task::spawn_blocking({
                let worlds = instance.config.bedrock_server_dir.join("worlds");
                move || omnibedrock_server::addons::missing_dependencies(&worlds)
            })
            .await
            .unwrap_or_default();
            if !warnings.is_empty() {
                item["pack_warnings"] = json!(warnings);
            }
            item
        }
        ["instances", id] if write => {
            let instance = s.resolve_instance(Some(id), u).await?;
            update_instance(s, u, &instance, b).await?
        }
        ["instances", id] if delete => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let record = instance.record();
            let may_delete = s.auth.is_owner(u).await
                || record.owner == u.username
                || s.auth.require(u, "INSTANCES_DELETE").await.is_ok();
            if !may_delete {
                return Err(ApiError::forbidden(
                    "Only the instance owner, a panel owner, or a holder of INSTANCES_DELETE may delete it",
                ));
            }
            if q.get("confirm").map(String::as_str) != Some("yes") {
                return Err(ApiError::bad(
                    "Pass ?confirm=yes to delete; this cannot be undone",
                ));
            }
            let keep_data = q.get("keep_data").is_some_and(|v| v == "1" || v == "true");
            s.instances
                .remove(id, !keep_data)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!(
                "Instance '{id}' removed{}",
                if keep_data {
                    " (data kept on disk)"
                } else {
                    ""
                }
            ))
        }
        ["instances", id, "action"] if create => {
            let instance = s.resolve_instance(Some(id), u).await?;
            let record = instance.record();
            if !s.auth.can_access_instance(u, &record).await {
                return Err(ApiError::forbidden(
                    "You can view this instance but cannot control its runtime",
                ));
            }
            s.require_for(u, "SERVER_MANAGE", &instance).await?;
            let action = value_string(b, "action")?.to_owned();
            let backend = instance.server.settings.borrow().backend;
            let deployment = instance.record().deployment;
            let bootstrap = if action == "start" {
                match backend {
                    Backend::Vanilla => ensure_vanilla_installed(&instance).await,
                    Backend::Endstone if deployment == Deployment::Docker => Ok(()),
                    Backend::Endstone => ensure_endstone_toolchain(&instance).await,
                }
            } else {
                Ok(())
            };
            if let Err(e) = bootstrap {
                json!({"success": false, "message": e.to_string()})
            } else {
                let server = instance.server.clone();
                let lock = instance.operations.clone().lock_owned().await;
                let result = tokio::spawn(async move {
                    let _lock = lock;
                    server.action(&action).await
                })
                .await
                .map_err(anyhow::Error::from)?;
                match result {
                    Ok(message) => success(message),
                    Err(e) => json!({"success":false,"message":e.to_string()}),
                }
            }
        }
        ["host"] if read => {
            s.auth.require(u, "SERVER_VIEW").await?;
            let mut host = s.metrics.all.borrow()["host"].clone();
            host["capabilities"] = json!(s.instances.capabilities());
            let records = s.instances.records().await;
            let allocated_memory: u64 = records
                .iter()
                .map(|r| r.resources.memory_mb.unwrap_or(0))
                .sum();
            let allocated_cpus: f64 = records
                .iter()
                .map(|r| r.resources.cpus.unwrap_or(0.0))
                .sum();
            host["allocated"] = json!({
                "memory_mb": allocated_memory,
                "cpus": allocated_cpus,
                "instances": records.len(),
            });
            // Overcommitment: aggregate allocations vs capacity minus the
            // configurable reserve. Warnings are advisory — per-instance
            // limits stay enforced regardless.
            let reserve = s.config.host_reserve_mb;
            let (memory, cpu) = omnibedrock_server::instances::overcommit_status(
                host["memory_total_mb"].as_f64().unwrap_or(0.0),
                reserve,
                allocated_memory,
                host["cpu_count"].as_u64().unwrap_or(0) as usize,
                allocated_cpus,
            );
            host["reserve_mb"] = json!(reserve);
            host["overcommit"] = json!({
                "memory_exceeded": memory.is_some(),
                "memory_warning": memory,
                "cpu_exceeded": cpu.is_some(),
                "cpu_warning": cpu,
            });
            host
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}
