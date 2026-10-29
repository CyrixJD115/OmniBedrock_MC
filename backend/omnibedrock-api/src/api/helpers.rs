//! Small request/response helpers shared by the dispatch modules.
use axum::{
    Json,
    response::{IntoResponse, Response},
};
use omnibedrock_core::error::{ApiError, Result};
use serde_json::{Value, json};

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
