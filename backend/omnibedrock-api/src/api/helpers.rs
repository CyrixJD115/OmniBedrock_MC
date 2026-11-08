//! Small request/response helpers shared by the dispatch modules.
use axum::{
    Json,
    extract::Multipart,
    response::{IntoResponse, Response},
};
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::instances::Instance;
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn value_string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| ApiError::bad(format!("'{key}' must be a string")))
}
pub(super) fn optional_string<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v[key].as_str().unwrap_or(default)
}
pub(super) fn data_response(value: Value) -> Response {
    Json(value).into_response()
}

pub(super) fn success(message: impl Into<String>) -> Value {
    json!({"success":true,"message":message.into()})
}

/// Pull the first file field out of a multipart form, streaming chunks so the
/// byte cap is enforced before anything is buffered past it. Non-file fields
/// (plain form metadata) are skipped. Shared by the plugin and explorer
/// upload routes; the caller owns the filename sanitization and confinement
/// (via `omnibedrock_core::files::sanitize_filename` + `safe_path`).
pub(super) async fn read_upload_field(
    multipart: Multipart,
    cap: u64,
    label: &str,
) -> Result<(String, Vec<u8>)> {
    let mut multipart = multipart;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad(format!("Invalid multipart upload: {e}")))?
    {
        let Some(name) = field.file_name().map(str::to_owned) else {
            continue;
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::bad(format!("Upload interrupted: {e}")))?
        {
            if bytes.len() as u64 + chunk.len() as u64 > cap {
                return Err(ApiError::bad(format!(
                    "{label} exceeds the {} MiB upload limit",
                    cap / (1024 * 1024)
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err(ApiError::bad("Upload body is empty"));
        }
        return Ok((name, bytes));
    }
    Err(ApiError::bad("The upload holds no file field"))
}

/// Best-effort detection of "the running server has this file open": scan
/// the instance's server process `/proc/<pid>/fd` links (Linux only; other
/// hosts and a stopped server report false — this is the "where detectable"
/// guard, not a guarantee). Callers refuse the write with a conflict when it
/// fires, so an in-use file is never truncated under a live server.
pub(super) fn file_open_by_server(svc: &Instance, target: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let pid = svc.server.snapshot().pid;
        let Some(pid) = pid else {
            return false;
        };
        let Ok(resolved) = target.canonicalize() else {
            return false;
        };
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            return false;
        };
        fds.filter_map(|e| e.ok())
            .filter_map(|e| std::fs::read_link(e.path()).ok())
            .any(|link| link == resolved)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (svc, target);
        false
    }
}
