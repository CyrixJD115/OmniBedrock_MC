//! Request entry point: the shutdown gate, instance scoping, authentication,
//! and the pre-parse special cases (addon uploads, screenshots, login).
use axum::{
    body::to_bytes,
    extract::{Path, Query, Request, State},
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
use super::helpers::{data_response, success, value_string};
use super::state::AppState;
use super::upload::upload_addons;

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
        state.auth.require(&user, "ADDONS_MANAGE").await?;
        let svc = state
            .resolve_instance(instance_id.as_deref(), &user)
            .await?;
        return upload_addons(state, svc, user, query, request.into_body()).await;
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
            .audit_instance(
                &user,
                "broadcaster.screenshot_update",
                account,
                "broadcaster",
                &svc.record().id,
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
        return Ok(data_response(
            state
                .auth
                .authenticate(
                    value_string(&body, "username")?.into(),
                    value_string(&body, "password")?.into(),
                )
                .await?,
        ));
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
    let token = header_token.or_else(|| {
        if query_allowed {
            query.get("token").cloned()
        } else {
            None
        }
    });
    let user = state.auth.resolve_user(token.as_deref()).await?;
    let svc = state
        .resolve_instance(instance_id.as_deref(), &user)
        .await?;
    let result =
        dispatch_authenticated(&state, &svc, &user, &method, &segments, &query, &body).await?;
    // Never include passwords, JWTs, or full edited documents in the audit trail.
    if method != Method::GET && result.status().is_success() {
        let (action, detail, category) = audit_action(&segments, &method, &body);
        if let Err(e) = state
            .audit_instance(
                &user,
                &action,
                &detail,
                category,
                instance_id.as_deref().unwrap_or_default(),
            )
            .await
        {
            tracing::error!(%e,"Action succeeded but audit persistence failed");
        }
    }
    Ok(result)
}
