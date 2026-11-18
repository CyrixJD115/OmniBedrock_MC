//! Personal API keys: create over the login session, authenticate on the
//! same URL/port with `X-API-User` + `X-API-Key` (or a bearer `omb_` key),
//! read-only scoping, owner-name binding, no key-minting with a key, revoke.
use crate::harness::{call, state};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn with_key(
    app: &axum::Router,
    method: &str,
    path: &str,
    user: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Content-Type", "application/json")
                .header("X-API-User", user)
                .header("X-API-Key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn api_keys_authenticate_scope_and_revoke() {
    let (_dir, _s, app, token) = state().await;
    let (status, created) = call(
        &app,
        "POST",
        "/api/v1/auth/api-keys",
        Some(&token),
        json!({"name": "ci", "scope": "full"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let key = created["key"].as_str().unwrap().to_owned();
    assert!(key.starts_with("omb_"));
    let id = created["record"]["id"].as_str().unwrap().to_owned();

    // Header pair on the regular API.
    let (status, me) = with_key(
        &app,
        "GET",
        "/api/v1/auth/me",
        "test-owner",
        &key,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["username"], "test-owner");

    // Bearer form works too.
    let (status, _) = call(&app, "GET", "/api/v1/auth/me", Some(&key), json!({})).await;
    assert_eq!(status, StatusCode::OK);

    // The username must match the key's owner.
    let (status, _) = with_key(
        &app,
        "GET",
        "/api/v1/auth/me",
        "someone-else",
        &key,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A key cannot mint more keys.
    let (status, _) = with_key(
        &app,
        "POST",
        "/api/v1/auth/api-keys",
        "test-owner",
        &key,
        json!({"name": "escalate"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Read-only keys refuse writes.
    let (_, ro) = call(
        &app,
        "POST",
        "/api/v1/auth/api-keys",
        Some(&token),
        json!({"name": "dash", "scope": "read_only"}),
    )
    .await;
    let ro_key = ro["key"].as_str().unwrap().to_owned();
    let (status, _) = with_key(
        &app,
        "GET",
        "/api/v1/server/status",
        "test-owner",
        &ro_key,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = with_key(
        &app,
        "POST",
        "/api/v1/console/command",
        "test-owner",
        &ro_key,
        json!({"command": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Listing never exposes hashes or plaintext.
    let (_, list) = call(
        &app,
        "GET",
        "/api/v1/auth/api-keys",
        Some(&token),
        json!({}),
    )
    .await;
    let text = list.to_string();
    assert!(!text.contains(&key) && !text.contains("hash"));
    assert_eq!(list.as_array().unwrap().len(), 2);

    // Revoke, then the key stops working.
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/auth/api-keys/{id}"),
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = with_key(
        &app,
        "GET",
        "/api/v1/auth/me",
        "test-owner",
        &key,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
