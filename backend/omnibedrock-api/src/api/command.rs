//! Single-request console command endpoint: `POST /instances/{id}/command`
//! pipes one line to the server and returns the console output it produced,
//! so scripts and agents don't need the WebSocket + download-token dance for
//! simple commands. The WS console stays the live/interactive path.
use axum::{
    Json,
    http::header,
    response::{IntoResponse, Response},
};
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::{instances::Instance, server::Console};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};

use super::helpers::value_string;

/// Poll cadence while waiting for the server's response lines.
const POLL: Duration = Duration::from_millis(50);
/// Output is considered complete after this much silence once lines arrived.
const QUIET: Duration = Duration::from_millis(250);
/// Cap on returned lines, so `burst`-style commands can't flood the response.
const MAX_LINES: usize = 200;
const DEFAULT_WAIT_MS: u64 = 1500;

pub(super) async fn run_command(
    svc: &Arc<Instance>,
    b: &Value,
    q: &HashMap<String, String>,
) -> Result<Response> {
    let command = value_string(b, "command")?.trim();
    if command.is_empty() {
        return Err(ApiError::bad("'command' must be a non-empty single line"));
    }
    let wait_ms = match q.get("wait_ms").map(|v| v.parse::<u64>()) {
        None => DEFAULT_WAIT_MS,
        Some(Ok(v)) => v.clamp(100, 5000),
        Some(Err(_)) => return Err(ApiError::bad("Invalid 'wait_ms'")),
    };
    // Everything the console publishes after this sequence belongs to this
    // command; `send_command` validates the running state and rejects
    // multi-line input, mapping to 409 exactly like /console/command.
    let before = svc.server.console.replay(None, 0).latest;
    svc.server
        .send_command(command)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    let lines = collect_output(&svc.server.console, before, Duration::from_millis(wait_ms)).await;
    if q.get("format").map(String::as_str) == Some("text") {
        let mut body = lines.join("\n");
        body.push('\n');
        return Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response());
    }
    Ok(Json(json!({
        "command": command,
        "lines": lines,
        "count": lines.len(),
        "data": {"command": command, "lines": lines.clone(), "count": lines.len()},
    }))
    .into_response())
}

/// Collect console lines published after `after` until the stream goes quiet
/// or the wait budget expires. Output that never arrives yields an empty
/// list — the command was still delivered (fire-and-forget semantics of the
/// underlying stdin write remain).
async fn collect_output(console: &Console, after: u64, wait: Duration) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut lines = Vec::new();
    let mut sequence = after;
    let mut progress = tokio::time::Instant::now();
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        let remaining = MAX_LINES - lines.len();
        let batch = console.replay(Some(sequence), remaining);
        if batch.lines.is_empty() {
            if !lines.is_empty() && progress.elapsed() >= QUIET {
                break;
            }
            continue;
        }
        for event in batch.lines {
            sequence = event.sequence;
            lines.push(event.line.clone());
        }
        progress = tokio::time::Instant::now();
        if lines.len() >= MAX_LINES {
            break;
        }
    }
    lines
}
