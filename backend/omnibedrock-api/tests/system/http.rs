//! HTTP-surface tests: the uniform `ok`/`error` envelope, `X-Request-Id`
//! correlation, the machine-readable route list and OpenAPI discovery, the
//! `?format=text` human mode, and the single-request command endpoint.
use crate::harness::{call, call_text, state, wait_line, wait_status};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use omnibedrock_server::server::Status;
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;

/// Like the harness `call` but also returns response headers.
async fn call_headers(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(json!(String::from_utf8_lossy(&bytes)));
    (status, headers, parsed)
}

#[tokio::test]
async fn envelope_adds_ok_and_request_id_to_success_objects() {
    let (_dir, _s, app, _token) = state().await;
    // Public object endpoint: gains ok:true plus a correlation header.
    let (status, headers, body) =
        call_headers(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    let request_id = headers["x-request-id"].to_str().unwrap().to_owned();
    assert!(request_id.starts_with("req-"), "minted id: {request_id}");
    // A caller-supplied id is honored so distributed traces can correlate.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/settings/panel")
                .header("X-Request-Id", "trace-42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["x-request-id"], "trace-42");
}

#[tokio::test]
async fn error_envelope_keeps_detail_and_adds_error() {
    let (_dir, _s, app, _token) = state().await;
    let (status, headers, body) = call_headers(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username": "test-owner", "password": "wrong"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Legacy field preserved for existing clients.
    assert!(body["detail"].as_str().is_some());
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["error"]["code"], json!("unauthorized"));
    assert_eq!(body["error"]["message"], body["detail"]);
    let request_id = headers["x-request-id"].to_str().unwrap().to_owned();
    assert_eq!(body["request_id"], json!(request_id));
}

#[tokio::test]
async fn route_list_and_openapi_are_discoverable() {
    let (_dir, _s, app, _token) = state().await;
    let (status, body) = call(&app, "GET", "/api/v1", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let Value::Array(routes) = body else {
        panic!("route list must be a bare JSON array");
    };
    assert!(routes.len() > 20);
    for route in &routes {
        assert!(route["method"].as_str().is_some());
        assert!(
            route["path"]
                .as_str()
                .unwrap_or_default()
                .starts_with("/api/v1")
        );
        assert!(!route["summary"].as_str().unwrap_or_default().is_empty());
    }
    let command = routes
        .iter()
        .find(|r| r["path"] == json!("/api/v1/instances/{id}/command"))
        .expect("command endpoint must be listed");
    assert_eq!(command["method"], json!("POST"));
    // The command endpoint is also in the OpenAPI document, at the root URL
    // and under the prefix.
    for path in ["/openapi.json", "/api/v1/openapi.json"] {
        let (status, doc) = call(&app, "GET", path, None, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(doc["openapi"], json!("3.0.3"));
        assert!(doc["paths"]["/instances/{id}/command"]["post"].is_object());
        assert!(
            doc["paths"]["/instances/{id}/command"]["post"]["requestBody"]["x-body"]["command"]
                .is_string()
        );
    }
    // /api-docs is a public HTML page linking the document.
    let (status, docs) = call_text(&app, "GET", "/api-docs", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(docs.contains("OmniBedrock Panel API"));
    assert!(docs.contains("/openapi.json"));
    assert!(!docs.contains("__PREFIX__"), "template must be substituted");
}

#[tokio::test]
async fn route_list_text_format() {
    let (_dir, _s, app, _token) = state().await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1?format=text")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("GET    /api/v1/instances"), "{text}");
}

#[tokio::test]
async fn instances_support_text_format() {
    let (_dir, _s, app, token) = state().await;
    let (status, text) = call_text(&app, "GET", "/api/v1/instances?format=text", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("ID"));
    assert!(text.contains("default"));
    // The JSON default stays a bare array — lists are never envelope-wrapped.
    let (status, body) = call(&app, "GET", "/api/v1/instances", Some(&token), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

#[tokio::test]
async fn command_endpoint_returns_console_output() {
    let (_dir, s, app, token) = state().await;
    s.server.action("start").await.unwrap();
    wait_line(&s.server, "ready").await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances/default/command",
        Some(&token),
        json!({"command": "hello world"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["command"], json!("hello world"));
    let lines = body["lines"].as_array().expect("lines array");
    assert!(
        lines.iter().any(|l| l
            .as_str()
            .is_some_and(|l| l.contains("command=hello world"))),
        "lines: {lines:?}"
    );
    assert_eq!(body["count"], json!(lines.len()));
    assert_eq!(body["data"]["lines"], body["lines"]);
    // Text format returns the same output as plain lines.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/instances/default/command?format=text")
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({"command": "hello world"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("command=hello world"));
    s.server.action("stop").await.unwrap();
    wait_status(&s.server, Status::Stopped).await;
    // A stopped server maps to the same 409 conflict as /console/command.
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances/default/command",
        Some(&token),
        json!({"command": "hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["error"]["code"], json!("conflict"));
    s.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn command_endpoint_rejects_missing_command_and_anonymous_callers() {
    let (_dir, s, app, token) = state().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances/default/command",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["error"]["code"], json!("invalid_request"));
    // No token at all is rejected before the body is even parsed.
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances/default/command",
        None,
        json!({"command": "hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], json!("unauthorized"));
    s.shutdown.cancel();
}

#[tokio::test]
async fn command_output_is_bounded_and_prompt() {
    let (_dir, s, app, token) = state().await;
    s.server.action("start").await.unwrap();
    wait_line(&s.server, "ready").await;
    // The fixture floods 10000 lines for `burst`; the endpoint must return
    // promptly with a bounded window of them, not stream the whole flood.
    let started = std::time::Instant::now();
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances/default/command?wait_ms=600",
        Some(&token),
        json!({"command": "burst"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let elapsed = started.elapsed();
    let lines = body["lines"].as_array().expect("lines array");
    assert!(!lines.is_empty());
    assert!(lines.len() <= 200, "capped at 200, got {}", lines.len());
    assert!(
        elapsed < Duration::from_secs(10),
        "endpoint must not wait out the whole burst: {elapsed:?}"
    );
    s.server.shutdown().await.unwrap();
}
