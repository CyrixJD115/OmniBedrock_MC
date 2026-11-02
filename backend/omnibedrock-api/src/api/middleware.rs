//! Cross-cutting response post-processing: the uniform `ok`/`error` envelope
//! and per-request `X-Request-Id` correlation.
//!
//! The envelope is ADDITIVE by design: successful JSON-object bodies gain a
//! top-level `"ok": true` and error bodies gain `"ok": false` plus a structured
//! `"error": {"code", "message"}` — while every field legacy clients read
//! (`detail`, `success`, `message`, payloads) stays exactly where it was.
//! Non-object JSON (the bare array/list endpoints) and non-JSON bodies
//! (text tails, images, archives, HTML) pass through untouched, so nothing
//! that parses the old shapes can break. Every response carries
//! `X-Request-Id`; errors also echo it in the body for log correlation.
use axum::{
    body::to_bytes,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

/// Response-body cap for envelope post-processing. JSON responses here are
/// console tails and management payloads, far below this; larger bodies are
/// binary downloads whose content type skips the envelope anyway.
const MAX_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const REQUEST_ID_HEADER: &str = "x-request-id";

/// Generate or honor a request id, stamp it into the tracing span (so every
/// log line inside the request carries it), run the inner service, then apply
/// the envelope and the response header.
pub(super) async fn request_context(request: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let request_id = request_id(&request);
    tracing::Span::current().record("request_id", tracing::field::display(&request_id));
    let mut response = next.run(request).await;
    if let Ok(value) = header::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    let status = response.status();
    if is_json(&response) && status != StatusCode::SWITCHING_PROTOCOLS {
        let headers = response.headers().clone();
        response = match to_bytes(response.into_body(), MAX_ENVELOPE_BYTES).await {
            Ok(bytes) => envelope(status, &headers, &bytes, &request_id),
            Err(_) => payload_too_large(&request_id),
        };
    }
    // method/path/request_id are fields of the surrounding `http` span, so
    // the event itself only adds the outcome and duration.
    tracing::info!(
        status = status.as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "request"
    );
    response
}

fn payload_too_large(request_id: &str) -> Response {
    let body = json!({
        "ok": false,
        "detail": "Response body too large to envelope",
        "error": {"code": "payload_too_large", "message": "Response body too large to envelope"},
        "request_id": request_id,
    });
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_vec(&body).unwrap_or_default(),
    )
        .into_response()
}

fn request_id(request: &Request) -> String {
    const ALLOWED: &str = "-_.";
    request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || ALLOWED.contains(c))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("req-{}", uuid::Uuid::new_v4().simple()))
}

fn is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("application/json"))
}

/// Rewrite one JSON body into the envelope. Objects gain `ok` (and, on
/// errors, the structured `error` object plus `request_id`); any other JSON
/// shape is re-serialized untouched. Existing keys — including a `detail`
/// message — are preserved verbatim.
fn envelope(
    status: StatusCode,
    headers: &axum::http::HeaderMap,
    bytes: &[u8],
    request_id: &str,
) -> Response {
    let mut headers = headers.clone();
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        // Not object-shaped JSON (or not JSON at all): pass the bytes back.
        return (status, headers, axum::body::Body::from(bytes.to_vec())).into_response();
    };
    if value.as_object().is_none() {
        // Bare arrays (instance lists, audit entries, the route list) keep
        // their legacy top-level shape; HTTP status already carries success.
        return (status, headers, axum::body::Body::from(bytes.to_vec())).into_response();
    }
    let object = value.as_object_mut().expect("checked above");
    let failed = status.is_client_error() || status.is_server_error();
    object.insert("ok".into(), Value::Bool(!failed));
    if failed {
        let message = ["detail", "message", "error"]
            .iter()
            .find_map(|k| object.get(*k).and_then(Value::as_str))
            .unwrap_or("Request failed")
            .to_owned();
        object.insert(
            "error".into(),
            json!({"code": error_code(status), "message": message}),
        );
        object.insert("request_id".into(), json!(request_id));
    }
    let Ok(body) = serde_json::to_vec(&value) else {
        return (status, headers, axum::body::Body::from(bytes.to_vec())).into_response();
    };
    // The replacement body has a different length; hyper recomputes it.
    headers.remove(header::CONTENT_LENGTH);
    (status, headers, axum::body::Body::from(body)).into_response()
}

/// Stable snake_case machine code for each status the API raises, so scripts
/// can branch without parsing messages.
fn error_code(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "invalid_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        409 => "conflict",
        413 => "payload_too_large",
        422 => "invalid_request",
        429 => "rate_limited",
        500 => "internal_error",
        502 => "bad_gateway",
        503 => "service_unavailable",
        _ => "error",
    }
}
