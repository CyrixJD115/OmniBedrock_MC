//! Request entry point: the shutdown gate, instance scoping, authentication,
//! and the pre-parse special cases (addon uploads, screenshots, login).
use axum::{
    body::to_bytes,
    extract::{FromRequest, Multipart, Path, Query, Request, State},
    http::{Method, StatusCode, header},
    response::Response,
};
use omnibedrock_core::{
    error::{ApiError, Result},
    files::{atomic_write, component},
};
use serde_json::{Value, json};
use std::collections::HashMap;

use super::audit::audit_action;
use super::dispatch_authenticated::dispatch_authenticated;
use super::explorer;
use super::helpers::{data_response, success, value_string};
use super::plugins;
use super::state::AppState;
use super::upload::upload_addons;

/// True for routes whose handlers operate on the `X-Instance`-scoped instance
/// (server runtime, console, players, properties, worlds, files, addons,
/// store, backups, broadcaster, and per-instance server settings). Panel-level
/// routes — auth, identity, roles, panel settings, the instance registry
/// itself, audit, host — never touch it, so an unknown id must not 404 them:
/// a browser holding a renamed/deleted instance used to lose its whole
/// session to this check (the identity call the login screen validates with
/// returned 404 and the client wiped its token, forever).
fn instance_scoped(segments: &[&str]) -> bool {
    match segments.first().copied() {
        Some("settings") => segments.get(1) == Some(&"server"),
        Some(first) => matches!(
            first,
            "server"
                | "console"
                | "performance"
                | "players"
                | "properties"
                | "worlds"
                | "files"
                | "addons"
                | "store"
                | "backups"
                | "broadcaster"
        ),
        None => false,
    }
}

pub(super) async fn dispatch(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    request: Request,
) -> Result<Response> {
    if state.shutdown.is_cancelled() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Panel is shutting down",
        ));
    }
    // The client address feeds login throttling and the audit trail; absent
    // in the in-process test harness (no connection info in `oneshot`).
    let ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_default();
    let method = request.method().clone();
    let path = path.trim_matches('/');
    let segments = path.split('/').collect::<Vec<_>>();
    // Optional instance scoping for every endpoint; absent means the default.
    // Downloads pass it as a query param because browsers cannot set headers.
    let instance_id = request
        .headers()
        .get("x-instance")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| query.get("instance").cloned());
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned);
    // Addon archives stream straight to a temp file so large .mcpack/.mcaddon
    // uploads never sit whole in memory.
    if path == "addons/upload" && method == Method::POST {
        let user = state.auth.resolve_user(header_token.as_deref()).await?;
        let svc = state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?;
        // Instance-scoped like every other addons route (see api/addons.rs).
        state.require_for(&user, "ADDONS_MANAGE", &svc).await?;
        return upload_addons(state, svc, user, query, request.into_body()).await;
    }
    // Multipart uploads (Endstone plugins, explorer files) bypass the JSON
    // envelope parse: they are form bodies with their own byte caps. Their
    // instance id lives in the path (the `/instances/{id}/...` form), so the
    // target is resolved and the permission gated here before the body is
    // consumed. Everything else in this branch returns before the generic
    // JSON path below.
    if method == Method::POST {
        let upload = match segments.as_slice() {
            ["instances", id, "plugins"] => Some(("PLUGIN_MANAGE", id)),
            ["instances", id, "files", "upload"] => Some(("FILES_WRITE", id)),
            _ => None,
        };
        if let Some((permission, id)) = upload {
            let user = state.auth.resolve_user(header_token.as_deref()).await?;
            let svc = state.resolve_instance(Some(id), &user).await?;
            state.require_for(&user, permission, &svc).await?;
            let multipart = Multipart::from_request(request, &())
                .await
                .map_err(|e| ApiError::bad(format!("Invalid multipart upload: {e}")))?;
            return if permission == "PLUGIN_MANAGE" {
                plugins::upload(&state, &svc, &user, &query, multipart).await
            } else {
                explorer::upload(&state, &svc, &user, &query, multipart).await
            };
        }
    }
    let bytes = to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| ApiError::bad("Request body too large"))?;
    // Screenshot uploads are raw image bytes, not the JSON envelope every other
    // endpoint uses, so they are handled before the generic body parse.
    if path == "broadcaster/screenshot" && method == Method::PUT {
        let user = state.auth.resolve_user(header_token.as_deref()).await?;
        let svc = state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?;
        state.require_for(&user, "BROADCASTER_MANAGE", &svc).await?;
        let account = query.get("account").map_or(Ok("main"), |a| component(a))?;
        if bytes.is_empty() {
            return Err(ApiError::bad("Screenshot body is required"));
        }
        let jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
        let png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        if !jpeg && !png {
            return Err(ApiError::bad("Screenshot must be a JPEG or PNG image"));
        }
        let destination = svc.broadcaster.screenshot_path(account);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        atomic_write(&destination, &bytes).await?;
        state
            .audit_raw(
                &user.username,
                "broadcaster.screenshot_update",
                account,
                "broadcaster",
                &svc.record().id,
                &ip,
            )
            .await
            .ok();
        return Ok(data_response(success(
            "Screenshot uploaded; Xbox may take a few minutes to refresh it",
        )));
    }
    let body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).map_err(|e| ApiError::bad(format!("Invalid JSON: {e}")))?
    };
    if path == "auth/login" && method == Method::POST {
        let username = value_string(&body, "username")?;
        let password = value_string(&body, "password")?;
        let result = state
            .auth
            .authenticate(username.to_owned(), password.to_owned(), &ip)
            .await;
        // Login attempts are audited with the source address: success marks
        // the session, failures are the forensic trail for brute force.
        match &result {
            Ok(payload) => {
                let _ = state
                    .audit_raw(
                        payload["user"]["username"].as_str().unwrap_or(username),
                        "auth.login",
                        "login succeeded",
                        "auth",
                        "",
                        &ip,
                    )
                    .await;
            }
            Err(_) => {
                let _ = state
                    .audit_raw(
                        username,
                        "auth.login_failed",
                        "login failed",
                        "auth",
                        "",
                        &ip,
                    )
                    .await;
            }
        }
        return Ok(data_response(result?));
    }
    // Public, unauthenticated: the login screen needs this before it can render.
    if path == "settings/panel" && method == Method::GET {
        return Ok(data_response(state.auth.panel_info()));
    }
    let query_allowed = method == Method::GET
        && ((segments.first() == Some(&"backups") && segments.last() == Some(&"download"))
            || path == "broadcaster/screenshot"
            || path == "addons/icon"
            || path == "store/image");
    // Browser-driven GETs (downloads, images) and WebSocket upgrades cannot
    // set headers; they carry a short-lived, purpose-bound download token
    // instead of the long-lived login bearer, so a URL that leaks into a
    // browser cache or an intermediary's access log is worthless within
    // minutes.
    let user = if query_allowed && let Some(dt) = query.get("dt").filter(|t| !t.trim().is_empty()) {
        state.auth.resolve_download_user(dt).await?
    } else {
        state.auth.resolve_user(header_token.as_deref()).await?
    };
    // Resolve the scoped instance only for routes that actually consume it
    // (`instance_scoped`): strict 404 on an unknown id belongs there alone.
    // Panel-level endpoints resolve the always-present default instead, so a
    // stale X-Instance header can no longer break auth, identity, or the
    // instance list itself.
    let svc = if instance_scoped(&segments) {
        state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?
    } else {
        state.resolve_instance(None, &user).await?
    };
    let result =
        dispatch_authenticated(&state, &svc, &user, &method, &segments, &query, &body).await?;
    // Never include passwords, JWTs, or full edited documents in the audit trail.
    if method != Method::GET && result.status().is_success() {
        let (action, detail, category) = audit_action(&segments, &method, &body);
        if let Err(e) = state
            .audit_raw(
                &user.username,
                &action,
                &detail,
                category,
                instance_id.as_deref().unwrap_or_default(),
                &ip,
            )
            .await
        {
            tracing::error!(%e,"Action succeeded but audit persistence failed");
        }
    }
    Ok(result)
}
