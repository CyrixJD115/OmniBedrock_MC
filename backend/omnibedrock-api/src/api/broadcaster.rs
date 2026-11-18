//! Broadcaster endpoints: status, actions, commands, config, accounts,
//! device codes, and screenshots.
use axum::{
    body::Body,
    http::{Method, header},
    response::{IntoResponse, Response},
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::component,
};
use omnibedrock_server::instances::Instance;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::helpers::{data_response, success, value_string};
use super::state::AppState;
use crate::auth::User;

pub(super) async fn broadcaster(
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
    let bc = &svc.broadcaster;
    let data = match p {
        ["broadcaster", "status"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            json!({
                "status": bc.snapshot(),
                "jar": bc.jar_path(),
                "jar_exists": bc.jar_path().exists(),
                "java": bc.java.borrow().clone(),
                "config_exists": bc.config_path().exists(),
                "dir": bc.dir(),
                "accounts": bc.accounts().await,
                "device_codes": bc.device_codes(),
                // Validated config.yml relay targets (loopback-vs-public) for
                // the co-location banner.
                "relay": bc.relay_status().await,
                // Panel-side query-ping health against the relay target.
                "ping": bc.ping_health(),
                // Version tag of config.yml on disk: the page reloads its
                // editor when this moves under an unmodified view.
                "config_etag": bc.config_etag().await,
                // server.properties facts the session should mirror.
                "server": bc.server_facts(),
                // Whether the instance's game server is up (the ping verdict
                // only means something while it is).
                "server_status": svc.server.snapshot().status,
                // A broadcaster java for this instance the panel did not spawn.
                "external": bc.external().await,
            })
        }
        // Jar build info: what last booted, what the panel installed, and
        // the newest release on GitHub (cached; ?refresh=1 refetches).
        ["broadcaster", "jar"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            let jar = bc.jar_path();
            let running = bc.jar_build.borrow().clone();
            let latest = omnibedrock_server::broadcaster::update::latest_release(
                q.get("refresh").is_some_and(|v| v == "1"),
            )
            .await;
            let running_build = running.as_ref().map(|(b, _)| b.clone());
            let installed = omnibedrock_server::broadcaster::update::installed_build(&jar).await;
            let current = installed.clone().or(running_build.clone());
            // An unknown current build (fresh panel, jar never booted under
            // it) still offers the latest release instead of hiding it.
            let update_available = match (&latest, &current) {
                (Ok(l), Some(c)) => l.tag.parse::<u64>().ok() > c.parse::<u64>().ok(),
                (Ok(_), None) => jar.exists(),
                _ => false,
            };
            json!({
                "jar": jar,
                "jar_exists": jar.exists(),
                "running_build": running_build,
                "running_bedrock": running.and_then(|(_, v)| v),
                "installed_build": installed,
                "latest": latest.as_ref().ok(),
                "latest_error": latest.as_ref().err().map(|e| format!("{e:#}")),
                "update_available": update_available,
            })
        }
        ["broadcaster", "jar", "update"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let jar = bc.jar_path();
            // A jar outside this instance's own dir is shared by every
            // instance: replacing it is a panel-level change.
            if !jar.starts_with(bc.dir()) {
                s.auth.require(u, "SETTINGS_EDIT").await?;
            }
            let release = omnibedrock_server::broadcaster::update::latest_release(true)
                .await
                .map_err(|e| {
                    ApiError::new(axum::http::StatusCode::BAD_GATEWAY, format!("{e:#}"))
                })?;
            let _lock = s.edits.lock().await;
            omnibedrock_server::broadcaster::update::install_release(&release, &jar)
                .await
                .map_err(|e| {
                    ApiError::new(axum::http::StatusCode::BAD_GATEWAY, format!("{e:#}"))
                })?;
            bc.console
                .publish(
                    format!(
                        "[LOCAL] Installed MCXboxBroadcast build {} (verified) at {} — restart the broadcaster to run it.",
                        release.tag,
                        jar.display()
                    ),
                    "panel",
                    false,
                )
                .await;
            json!({
                "success": true,
                "message": format!("Installed build {}; restart the broadcaster to apply", release.tag),
                "build": release.tag,
            })
        }
        ["broadcaster", "action"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let action = value_string(b, "action")?;
            if !["start", "stop", "restart", "takeover"].contains(&action) {
                return Err(ApiError::bad("Invalid broadcaster action"));
            }
            success(
                bc.action(action)
                    .await
                    .map_err(|e| ApiError::conflict(e.to_string()))?,
            )
        }
        ["broadcaster", "command"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            bc.send_command(value_string(b, "command")?)
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success("Command sent")
        }
        ["broadcaster", "config"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            let text = bc.read_config().await?;
            let etag = omnibedrock_server::broadcaster::config_etag_of(text.as_bytes());
            return Ok((
                [
                    (
                        header::CONTENT_TYPE,
                        "text/plain; charset=utf-8".to_string(),
                    ),
                    (header::ETAG, format!("\"{etag}\"")),
                    (header::CACHE_CONTROL, "no-store".to_string()),
                ],
                text,
            )
                .into_response());
        }
        ["broadcaster", "config"] if *m == Method::PUT => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let _lock = s.edits.lock().await;
            // Optimistic concurrency: a save based on a copy that no longer
            // matches the disk (the jar rewrote it, another tab saved, a hand
            // edit) is refused instead of clobbering the newer file.
            if let Some(base) = b["base_etag"].as_str().map(|e| e.trim_matches('"')) {
                let current = bc.config_etag().await;
                if !base.is_empty() && !current.is_empty() && base != current {
                    return Err(ApiError::conflict(
                        "config.yml changed on disk since you opened it; reload to see the \
                         current file, then re-apply your edit",
                    ));
                }
            }
            bc.write_config(value_string(b, "text")?)
                .await
                .map_err(|e| ApiError::bad(e.to_string()))?;
            json!({
                "success": true,
                "message": "Config saved; restart the broadcaster to apply",
                "etag": bc.config_etag().await,
            })
        }
        ["broadcaster", "accounts"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            bc.accounts().await
        }
        ["broadcaster", "accounts"] if create => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let action = value_string(b, "action")?;
            if !["add", "remove"].contains(&action) {
                return Err(ApiError::bad("Invalid account action"));
            }
            let id = value_string(b, "id")?;
            component(id)?;
            bc.send_command(&format!("accounts {action} {id}"))
                .await
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            success(format!(
                "Sent 'accounts {action} {id}'; watch the console for the sign-in code"
            ))
        }
        ["broadcaster", "device-code"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            json!(bc.device_codes())
        }
        ["broadcaster", "screenshot"] if read => {
            s.require_for(u, "BROADCASTER_VIEW", svc).await?;
            let account = q.get("account").map_or(Ok("main"), |a| component(a))?;
            let path = bc.screenshot_path(account);
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| ApiError::missing("No screenshot uploaded"))?;
            let size = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file);
            return Ok((
                [
                    (header::CONTENT_TYPE, "image/jpeg".to_string()),
                    (header::CONTENT_LENGTH, size.to_string()),
                ],
                Body::from_stream(stream),
            )
                .into_response());
        }
        _ => return Err(ApiError::missing("Endpoint not found")),
    };
    Ok(data_response(data))
}
