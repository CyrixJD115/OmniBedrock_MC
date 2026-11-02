//! `GET {prefix}` — the machine-readable route inventory. Derived at runtime
//! from the checked-in `openapi.json`, so the list and the document can never
//! disagree: one source of truth for humans, scripts, and agents.
use axum::{
    Json,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

use super::state::AppState;

static DOC: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../openapi.json")).expect("openapi.json is valid JSON")
});

/// Flatten the OpenAPI `paths` object into `[{method, path, summary}]` with
/// the live API prefix applied.
pub(super) fn entries(prefix: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(paths) = DOC.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            let Some(operations) = item.as_object() else {
                continue;
            };
            for (method, operation) in operations {
                let method = method.to_ascii_uppercase();
                if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "DELETE") {
                    continue;
                }
                out.push(json!({
                    "method": method,
                    "path": if path == "/" {
                        prefix.to_owned()
                    } else {
                        format!("{prefix}{path}")
                    },
                    "summary": operation
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                }));
            }
        }
    }
    out.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["path"].as_str().unwrap_or_default().to_owned(),
                v["method"].as_str().unwrap_or_default().to_owned(),
            )
        };
        key(a).cmp(&key(b))
    });
    out
}

/// Aligned plain-text rendering for `?format=text` — one route per line.
pub(super) fn entries_text(list: &[Value]) -> String {
    let method_width = list
        .iter()
        .map(|r| r["method"].as_str().unwrap_or_default().len())
        .max()
        .unwrap_or(0);
    let path_width = list
        .iter()
        .map(|r| r["path"].as_str().unwrap_or_default().len())
        .max()
        .unwrap_or(0);
    let mut body = String::new();
    for route in list {
        body.push_str(&format!(
            "{:method_width$} {:path_width$}  {}\n",
            route["method"].as_str().unwrap_or_default(),
            route["path"].as_str().unwrap_or_default(),
            route["summary"].as_str().unwrap_or_default()
        ));
    }
    body
}

pub(super) async fn route_list(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let list = entries(&state.config.api_prefix);
    // Structured content negotiation via query (?format=text), matching
    // /console/tail and /instances; the default stays JSON for machines.
    if query.get("format").map(String::as_str) == Some("text") {
        return (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            entries_text(&list),
        )
            .into_response();
    }
    Json(list).into_response()
}
