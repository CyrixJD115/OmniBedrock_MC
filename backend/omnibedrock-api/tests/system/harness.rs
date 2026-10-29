//! Shared system-test harness: fake server fixtures, temp-directory setup,
//! server-manager construction, in-process HTTP call helpers, login helpers,
//! and console wait helpers used across the API-area test modules.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use omnibedrock_api::api::{AppState, router};
use omnibedrock_core::config::Config;
use omnibedrock_server::server::{ServerManager, Status};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

pub(crate) const FAKE: &str = r#"#!/bin/sh
printf '[INFO] Endstone fixture ready\n'
printf '[ERROR] fixture 42 failed at 1.2.3\n' >&2
while IFS= read -r command; do
  case "$command" in
    stop) printf '[INFO] stopping\n'; exit 0;;
    crash) exit 7;;
    'list') printf '[INFO] There are 2/10 players online: Alex, Steve\n';;
    'save query') printf '[INFO] Data saved. Files are now ready to be copied.\n';;
    'burst') i=0; while [ "$i" -lt 10000 ]; do printf '[INFO] burst %s\n' "$i"; i=$((i+1)); done; printf '[INFO] burst complete\n';;
    'long') i=0; while [ "$i" -lt 10000 ]; do printf x; i=$((i+1)); done; printf '\n';;
    'partial') printf 'unterminated'; exit 0;;
    *) printf '[INFO] command=%s\n' "$command";;
  esac
done
"#;
pub(crate) fn config(dir: &TempDir) -> Config {
    let root = dir.path();
    Config {
        bedrock_server_dir: root.join("bedrock"),
        data_dir: root.join("data"),
        backups_dir: root.join("backups"),
        logs_dir: root.join("logs"),
        endstone_bin: Some(root.join("endstone-fixture")),
        console_max_lines: 128,
        console_max_line_bytes: 1024,
        console_max_bytes: 256 * 1024,
        ws_heartbeat_interval: 1,
        ..Config::default()
    }
}
pub(crate) async fn setup() -> (TempDir, Config) {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let c = config(&dir);
    for path in [
        &c.data_dir,
        &c.logs_dir,
        &c.backups_dir,
        &c.bedrock_server_dir.join("worlds/Fixture/db"),
    ] {
        tokio::fs::create_dir_all(path).await.unwrap();
    }
    tokio::fs::write(c.endstone_bin.as_ref().unwrap(), FAKE)
        .await
        .unwrap();
    tokio::fs::set_permissions(
        c.endstone_bin.as_ref().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .await
    .unwrap();
    tokio::fs::write(
        c.bedrock_server_dir.join("worlds/Fixture/level.dat"),
        "original world",
    )
    .await
    .unwrap();
    (dir, c)
}
pub(crate) async fn manager(c: Config) -> ServerManager {
    ServerManager::new(Arc::new(c), CancellationToken::new())
        .await
        .unwrap()
}
pub(crate) async fn wait_line(manager: &ServerManager, needle: &str) {
    let mut changes = manager.console.changed.subscribe();
    let mut after = None;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let batch = manager.console.replay_reliable(after, 512).await.unwrap();
            for l in batch.lines {
                after = Some(l.sequence);
                if l.line.contains(needle) {
                    return;
                }
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
pub(crate) async fn wait_status(manager: &ServerManager, status: Status) {
    let mut state = manager.state.clone();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if state.borrow_and_update().status == status {
                return;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
pub(crate) async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
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
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(json!(String::from_utf8_lossy(&bytes)));
    (status, body)
}
pub(crate) async fn call_text(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
/// GET a text body with optional X-Instance scoping.
pub(crate) async fn call_text_instance(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    instance: Option<&str>,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {token}"));
    if let Some(instance) = instance {
        request = request.header("X-Instance", instance);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

pub(crate) async fn call_bytes(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
) -> (StatusCode, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, bytes.to_vec())
}
pub(crate) async fn login_owner(app: &Router, s: &AppState) -> String {
    s.auth
        .mutate_user(
            &owner(),
            "test-owner",
            Some(json!({"password":"test-password","role":"owner"})),
            true,
        )
        .await
        .unwrap();
    let (status, login) = call(
        app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"test-owner","password":"test-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    login["token"].as_str().unwrap().to_owned()
}
pub(crate) async fn state() -> (TempDir, AppState, Router, String) {
    let (dir, c) = setup().await;
    let state = AppState::new(c, CancellationToken::new()).await.unwrap();
    let app = router(state.clone()).unwrap();
    state
        .auth
        .mutate_user(
            &owner(),
            "test-owner",
            Some(json!({"password":"test-password","role":"owner"})),
            true,
        )
        .await
        .unwrap();
    let (status, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"test-owner","password":"test-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = login["token"].as_str().unwrap().to_owned();
    (dir, state, app, token)
}
pub(crate) fn owner() -> omnibedrock_api::auth::User {
    omnibedrock_api::auth::User {
        token_version: 0,
        username: "admin".into(),
        role: "owner".into(),
        roles: vec!["owner".into()],
        password_hash: String::new(),
        display_name: "Admin".into(),
        created_at: String::new(),
        last_login: String::new(),
        quotas: Default::default(),
    }
}

pub(crate) const FAKE_JAVA: &str = r#"#!/bin/sh
if [ "$1" = "-version" ]; then printf 'openjdk version "17.0.0" 2024-01-01\n' >&2; exit 0; fi
printf '[INFO] Broadcaster fixture ready\n'
printf '[INFO] To sign in, use a web browser to open the page https://www.microsoft.com/link and enter the code ABCD1234 to authenticate.\n'
printf '[Sub-Session alt1] [INFO] To sign in, use a web browser to open the page https://microsoft.com/devicelogin and enter the code EFGH5678 to authenticate.\n'
while IFS= read -r command; do
  case "$command" in
    stop|exit) exit 0;;
    'accounts list') printf '[INFO] main (0 friends), alt1 (0 friends)\n';;
    *) printf '[INFO] cmd=%s\n' "$command";;
  esac
done
"#;

/// Call with an extra header (X-Instance scoping).
pub(crate) async fn call_instance(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    instance: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(instance) = instance {
        request = request.header("X-Instance", instance);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(json!(String::from_utf8_lossy(&bytes)));
    (status, parsed)
}
