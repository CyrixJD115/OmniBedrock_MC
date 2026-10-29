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
                // A broadcaster java for this instance the panel did not spawn.
                "external": bc.external().await,
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
            return Ok((
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                bc.read_config().await?,
            )
                .into_response());
        }
        ["broadcaster", "config"] if *m == Method::PUT => {
            s.require_for(u, "BROADCASTER_MANAGE", svc).await?;
            let _lock = s.edits.lock().await;
            bc.write_config(value_string(b, "text")?)
                .await
                .map_err(|e| ApiError::bad(e.to_string()))?;
            success("Config saved; restart the broadcaster to apply")
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
