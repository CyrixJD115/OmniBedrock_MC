//! Add-on compatibility checker endpoints (`/addons/compat…`).
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::{compat, instances::Instance};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::state::AppState;

/// Console lines to analyse when the server keeps no log files (vanilla).
const CONSOLE_LINES: usize = 20_000;

async fn inputs(s: &AppState, svc: &Arc<Instance>) -> compat::Inputs {
    let installed = svc.bds.installed_version().await;
    let snapshot = svc.server.snapshot();
    // "1.26.65.1 · Endstone 0.11.16"
    let label = snapshot.version.clone().unwrap_or_default();
    let endstone = label
        .split("Endstone ")
        .nth(1)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let server_version = svc
        .server
        .console
        .version()
        .and_then(|v| v.split_whitespace().next().map(str::to_owned))
        .or_else(|| {
            installed
                .as_ref()
                .and_then(|v| v["version"].as_str().map(str::to_owned))
        })
        .or_else(|| {
            label
                .split_whitespace()
                .next()
                .map(str::to_owned)
                .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
        });
    let console = if svc.config.bedrock_server_dir.join("logs").is_dir() {
        Vec::new()
    } else {
        svc.server
            .console
            .replay(None, CONSOLE_LINES)
            .lines
            .into_iter()
            .map(|l| l.line.clone())
            .collect()
    };
    compat::Inputs {
        server_dir: svc.config.bedrock_server_dir.clone(),
        cache_dir: s.config.cache_dir.clone(),
        server_version,
        endstone_version: endstone,
        console,
    }
}

fn options(q: &HashMap<String, String>) -> compat::Options {
    compat::Options {
        world: q
            .get("world")
            .cloned()
            .filter(|w| !w.is_empty() && !w.contains(['/', '\\'])),
        max_files: q.get("files").and_then(|v| v.parse().ok()).unwrap_or(14),
        refresh: q.contains_key("refresh"),
    }
}

pub(super) async fn report(
    s: &AppState,
    svc: &Arc<Instance>,
    q: &HashMap<String, String>,
) -> Result<Value> {
    let r = compat::analyze(inputs(s, svc).await, options(q)).await?;
    Ok(serde_json::to_value(r).map_err(anyhow::Error::from)?)
}

pub(super) async fn export(
    s: &AppState,
    svc: &Arc<Instance>,
    q: &HashMap<String, String>,
) -> Result<Response> {
    let r = compat::analyze(inputs(s, svc).await, options(q)).await?;
    let name = format!(
        "compat-{}-{}.md",
        svc.record.read().map(|r| r.id.clone()).unwrap_or_default(),
        chrono::Utc::now().format("%Y%m%d-%H%M")
    );
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/markdown; charset=utf-8".to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        compat::markdown(&r),
    )
        .into_response())
}

pub(super) async fn prune_content_logs(svc: &Arc<Instance>, b: &Value) -> Result<Value> {
    if b["confirm"] != json!(true) {
        return Err(ApiError::bad(
            "Pass {\"confirm\": true} to delete old ContentLog files",
        ));
    }
    let dir = svc.config.bedrock_server_dir.clone();
    let (files, bytes) = tokio::task::spawn_blocking(move || compat::prune_content_logs(&dir))
        .await
        .map_err(anyhow::Error::from)?;
    Ok(json!({"success": true, "files": files, "bytes": bytes,
        "message": format!("Deleted {files} ContentLog file(s), freed {} MB", bytes / (1024 * 1024))}))
}
