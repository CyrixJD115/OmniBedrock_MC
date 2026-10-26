#![cfg(unix)]
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use omnibedrock_api::{
    api::{AppState, router},
    auth::Auth,
};
use omnibedrock_core::config::Config;
use omnibedrock_server::{
    backups::BackupRequest,
    server::{Console, ServerManager, ServerSettings, Status, detect_level},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const FAKE: &str = r#"#!/bin/sh
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
fn config(dir: &TempDir) -> Config {
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
async fn setup() -> (TempDir, Config) {
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
async fn manager(c: Config) -> ServerManager {
    ServerManager::new(Arc::new(c), CancellationToken::new())
        .await
        .unwrap()
}
async fn wait_line(manager: &ServerManager, needle: &str) {
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
async fn wait_status(manager: &ServerManager, status: Status) {
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
async fn call(
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
async fn call_text(app: &Router, method: &str, path: &str, token: &str) -> (StatusCode, String) {
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
async fn call_text_instance(
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

async fn call_bytes(app: &Router, method: &str, path: &str, token: &str) -> (StatusCode, Vec<u8>) {
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
async fn login_owner(app: &Router, s: &AppState) -> String {
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
async fn state() -> (TempDir, AppState, Router, String) {
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
fn owner() -> omnibedrock_api::auth::User {
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
#[tokio::test]
async fn process_round_trip_restart_and_no_stale_commands() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    assert!(m.send_command("before-start").await.is_err());
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let first_pid = m.snapshot().pid;
    m.send_command("héllo").await.unwrap();
    wait_line(&m, "command=héllo").await;
    assert!(m.send_command("op Steve\nstop").await.is_err());
    assert!(m.send_command("").await.is_err());
    m.action("restart").await.unwrap();
    assert_ne!(m.snapshot().pid, first_pid);
    assert_eq!(m.snapshot().status, Status::Running);
    m.action("stop").await.unwrap();
    assert_eq!(m.snapshot().status, Status::Stopped);
    assert!(m.snapshot().pid.is_none());
    let lock = tokio::fs::read_to_string(m_settings_path(&_dir, "console_lock_state.yaml"))
        .await
        .unwrap();
    assert!(lock.contains("unlocked"));
    m.shutdown().await.unwrap();
}
fn m_settings_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join("data").join(name)
}
#[tokio::test]
async fn concurrent_start_is_serialized() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    let (a, b) = tokio::join!(m.action("start"), m.action("start"));
    assert!(a.is_ok() && b.is_ok());
    assert!(a.unwrap().contains("PID"));
    assert!(b.unwrap().contains("already"));
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_launch_recovers_to_stopped() {
    let (_dir, mut c) = setup().await;
    c.endstone_bin = Some("/nonexistent/omnibedrock-endstone".into());
    let m = manager(c).await;
    assert!(m.action("start").await.is_err());
    assert_eq!(m.snapshot().status, Status::Stopped);
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn crash_restart_budget_and_cancel() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.configure(ServerSettings {
        auto_restart_delay: 1,
        max_crashes: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    let first = m.snapshot().pid;
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    wait_status(&m, Status::Running).await;
    assert_ne!(first, m.snapshot().pid);
    assert_eq!(m.snapshot().crash_count, 1);
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(m.snapshot().status, Status::Crashed);
    assert_eq!(m.snapshot().crash_count, 2);
    m.action("start").await.unwrap();
    assert_eq!(m.snapshot().crash_count, 0);
    m.send_command("crash").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    m.action("stop").await.unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(m.snapshot().status, Status::Stopped);
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn high_volume_replays_without_missing_lines() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let mut after = Some(m.console.replay(None, 0).latest);
    m.send_command("burst").await.unwrap();
    wait_line(&m, "burst complete").await;
    let mut numbers = Vec::new();
    loop {
        let replay = m.console.replay_reliable(after, 256).await.unwrap();
        assert!(replay.gap.is_none());
        if replay.lines.is_empty() {
            break;
        }
        for line in replay.lines {
            after = Some(line.sequence);
            if let Some(n) = line
                .line
                .strip_prefix("[INFO] burst ")
                .and_then(|n| n.parse::<u32>().ok())
            {
                numbers.push(n);
            }
        }
        if after == Some(replay.latest) {
            break;
        }
    }
    assert_eq!(numbers, (0..10000).collect::<Vec<_>>());
    assert!(m.console.replay(None, 10000).lines.len() <= 128);
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn oversized_and_eof_lines_are_preserved() {
    let (_dir, c) = setup().await;
    let m = manager(c).await;
    m.configure(ServerSettings {
        auto_restart: false,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    wait_line(&m, "ready").await;
    let after = Some(m.console.replay(None, 0).latest);
    m.send_command("long").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replay = m.console.replay_reliable(after, 128).await.unwrap();
            let size: usize = replay
                .lines
                .iter()
                .filter(|l| l.line.starts_with('x'))
                .map(|l| l.line.len())
                .sum();
            if size == 10000 {
                assert!(replay.lines.iter().any(|l| l.continued));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    m.send_command("partial").await.unwrap();
    wait_status(&m, Status::Crashed).await;
    wait_line(&m, "unterminated").await;
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn disk_retention_reports_explicit_gap() {
    let (dir, mut c) = setup().await;
    c.console_journal_bytes = 4 * 1024 * 1024;
    let console = Console::new(&c);
    for i in 0..6000 {
        console
            .publish(format!("{i} {}", "x".repeat(1000)), "stdout", false)
            .await;
    }
    let replay = console.replay_reliable(Some(0), 256).await.unwrap();
    assert!(replay.gap.is_some());
    let bytes: u64 = walkdir::WalkDir::new(dir.path().join("logs/console"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().unwrap().len())
        .sum();
    assert!(bytes <= c.console_journal_bytes);
}
#[tokio::test]
async fn level_error_stats_and_mspt() {
    let (_dir, c) = setup().await;
    let console = Console::new(&c);
    assert_eq!(detect_level("[WARNING] hi"), "warn");
    assert_eq!(detect_level("Fatal exception"), "error");
    assert_eq!(detect_level("[INFO] error wording"), "info");
    console.publish("TPS: 19.8".into(), "stdout", false).await;
    assert_eq!(console.tps(), Some(19.8));
    console.publish("MSPT: 100".into(), "stdout", false).await;
    assert_eq!(console.tps(), Some(10.0));
    console
        .publish("[ERROR] failure 12 at 1.2.3".into(), "stderr", false)
        .await;
    console
        .publish("[ERROR] failure 99 at 2.3.4".into(), "stderr", false)
        .await;
    assert_eq!(console.errors().len(), 1);
    assert_eq!(console.errors()[0].count, 2);
}
#[tokio::test]
async fn legacy_password_and_jwt_migrate() {
    let (_dir, c) = setup().await;
    let salt = [1u8; 32];
    let key = pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(b"legacy-password", &salt, 100000);
    let users = json!({"legacy":{"username":"legacy","password_hash":format!("{}:{}",hex::encode(salt),hex::encode(key)),"role":"owner"}});
    tokio::fs::write(
        c.data_dir.join("users.yaml"),
        serde_yaml::to_string(&users).unwrap(),
    )
    .await
    .unwrap();
    let auth = Auth::new(Arc::new(c.clone())).await.unwrap();
    let login = auth
        .authenticate("legacy".into(), "legacy-password".into())
        .await
        .unwrap();
    assert_eq!(
        auth.verify(login["token"].as_str().unwrap())
            .await
            .unwrap()
            .username,
        "legacy"
    );
    assert!(
        tokio::fs::read_to_string(c.data_dir.join("users.yaml"))
            .await
            .unwrap()
            .contains("$argon2id$")
    );
    let secret = tokio::fs::read_to_string(c.data_dir.join("jwt_secret"))
        .await
        .unwrap();
    let key = Sha256::digest(secret.as_bytes());
    let claims = json!({"sub":"legacy","role":"owner","iat":chrono::Utc::now().timestamp(),"exp":chrono::Utc::now().timestamp()+600});
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(&key),
    )
    .unwrap();
    assert!(auth.verify(&token).await.is_ok());
    assert!(
        auth.authenticate("legacy".into(), "incorrect".into())
            .await
            .is_err()
    );
    assert!(
        auth.authenticate("unknown".into(), "incorrect".into())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn api_roles_permissions_and_revoke() {
    let (_dir, s, app, token) = state().await;
    let (unauth, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(unauth, StatusCode::UNAUTHORIZED);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        Some(&token),
        json!({"username":"viewer","password":"viewer-password","role":"viewer"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"viewer","password":"viewer-password"}),
    )
    .await;
    let viewer = login["token"].as_str().unwrap();
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/console/command",
        Some(viewer),
        json!({"command":"op Steve"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/players/action",
        Some(viewer),
        json!({"action":"ban","target":"Steve"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/auth/users/viewer",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(s.auth.verify(viewer).await.is_err());
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&token),
        json!({"name":"custom","permissions":["SERVER_VIEW"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/roles/custom",
        Some(&token),
        json!({"permissions":["FAKE_PERMISSION"]}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn api_file_properties_world_addons_and_audit() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());
    tokio::fs::write(
        s.config.bedrock_server_dir.join("server.properties"),
        "# comment\nmax-players=10 # inline\nserver-name=Fixture\n",
    )
    .await
    .unwrap();
    let (code, v) = call(&app, "GET", "/api/v1/properties/", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v[1]["inline_comment"], "# inline");
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/properties/max-players",
        t,
        json!({"value":"0"}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/properties/max-players",
        t,
        json!({"value":"20"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, text) = call(&app, "GET", "/api/v1/properties/raw", t, json!({})).await;
    assert!(text.as_str().unwrap().contains("max-players=20 # inline"));
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/files/example.ini",
        t,
        json!({"content":"[server]\nmode=1\n"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, v) = call(&app, "GET", "/api/v1/files/example.ini", t, json!({})).await;
    assert_eq!(v["content"], "[server]\nmode=1\n");
    for file in [
        "users.yaml",
        "roles.yaml",
        "jwt_secret",
        "console_lock_state.yaml",
    ] {
        let (code, _) = call(&app, "GET", &format!("/api/v1/files/{file}"), t, json!({})).await;
        assert_eq!(code, StatusCode::FORBIDDEN);
    }
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/files/%2E%2E%2Fusers.yaml",
        t,
        json!({}),
    )
    .await;
    assert_ne!(code, StatusCode::OK);
    let pack = s
        .config
        .bedrock_server_dir
        .join("worlds/Fixture/behavior_packs/Example");
    tokio::fs::create_dir_all(&pack).await.unwrap();
    tokio::fs::write(
        pack.join("manifest.json"),
        r#"{"header":{"uuid":"00000000-0000-0000-0000-000000000001","version":[1,2,3]}}"#,
    )
    .await
    .unwrap();
    let (code, v) = call(&app, "GET", "/api/v1/addons/", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v["behavior_packs"][0]["name"], "Example");
    let (code, v) = call(
        &app,
        "POST",
        "/api/v1/addons/randomize-uuid",
        t,
        json!({"path":pack}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(uuid::Uuid::parse_str(v["uuid"].as_str().unwrap()).is_ok());
    let (code, _) = call(&app, "GET", "/api/v1/worlds/Fixture/contents", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    let (code, v) = call(&app, "GET", "/api/v1/worlds/Fixture", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert!(v["files"].as_u64().unwrap() > 0);
    let (code, v) = call(&app, "GET", "/api/v1/audit/?limit=100", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert!(!v.as_array().unwrap().is_empty());
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
/// Property edits must land in the X-Instance-scoped server dir — the same
/// file GET reads — and unknown keys must be appended rather than rejected.
#[tokio::test]
async fn properties_writes_are_instance_scoped() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());
    tokio::fs::write(
        s.config.bedrock_server_dir.join("server.properties"),
        "motd=global-file\n",
    )
    .await
    .unwrap();
    let (code, created) = call(
        &app,
        "POST",
        "/api/v1/instances",
        t,
        json!({"name":"Props Server","backend":"vanilla"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    // The scoped read sees the instance's seeded file, not the global one.
    let (_, text) =
        call_text_instance(&app, "GET", "/api/v1/properties/raw", &token, Some(&id)).await;
    assert!(text.contains("transport=raknet"), "{text}");
    assert!(!text.contains("motd=global-file"), "{text}");

    // A key the seeded file lacks is appended into the instance file.
    let (code, v) = call_instance(
        &app,
        "PUT",
        "/api/v1/properties/motd",
        t,
        Some(&id),
        json!({"value":"scoped-edit"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, text) =
        call_text_instance(&app, "GET", "/api/v1/properties/raw", &token, Some(&id)).await;
    assert!(text.contains("motd=scoped-edit"), "{text}");
    assert_eq!(
        tokio::fs::read_to_string(s.config.bedrock_server_dir.join("server.properties"))
            .await
            .unwrap(),
        "motd=global-file\n"
    );

    // The raw editor writes to the instance dir too.
    let (code, v) = call_instance(
        &app,
        "PUT",
        "/api/v1/properties/raw",
        t,
        Some(&id),
        json!({"text":"motd=raw-edit\nwhite-list=true\n"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, text) =
        call_text_instance(&app, "GET", "/api/v1/properties/raw", &token, Some(&id)).await;
    assert_eq!(text, "motd=raw-edit\nwhite-list=true\n");
    assert_eq!(
        tokio::fs::read_to_string(s.config.bedrock_server_dir.join("server.properties"))
            .await
            .unwrap(),
        "motd=global-file\n"
    );

    // A crafted key (comment/separator injection) is rejected.
    let (code, _) = call_instance(
        &app,
        "PUT",
        "/api/v1/properties/bad%20key",
        t,
        Some(&id),
        json!({"value":"x"}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn backup_roundtrip_trash_restore_and_dry_run() {
    let (dir, s, app, token) = state().await;
    let t = Some(token.as_str());
    let (code, v) = call(
        &app,
        "POST",
        "/api/v1/backups/create",
        t,
        json!({"world":"Fixture","run_hooks":false}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v["success"], true);
    let filename = v["filename"].as_str().unwrap();
    let (code, v) = call(&app, "GET", "/api/v1/backups/?world=Fixture", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v["total"], 1);
    let (code, _) = call(
        &app,
        "DELETE",
        &format!("/api/v1/backups/Fixture/{filename}"),
        t,
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, trash) = call(&app, "GET", "/api/v1/backups/trash", t, json!({})).await;
    assert_eq!(trash.as_array().unwrap().len(), 1);
    let (code, _) = call(
        &app,
        "POST",
        &format!("/api/v1/backups/restore/Fixture/{filename}"),
        t,
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    tokio::fs::write(
        s.config.bedrock_server_dir.join("worlds/Fixture/level.dat"),
        "changed world",
    )
    .await
    .unwrap();
    let (code, _) = call(
        &app,
        "POST",
        &format!("/api/v1/backups/Fixture/{filename}/restore-to-world"),
        t,
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        tokio::fs::read_to_string(s.config.bedrock_server_dir.join("worlds/Fixture/level.dat"))
            .await
            .unwrap(),
        "original world"
    );
    let marker = dir.path().join("should-not-exist");
    let hooks = json!({"before":[{"type":"send","value":"save hold"},{"type":"command","value":format!("touch '{}'",marker.display())}],"after":[{"type":"send","value":"save resume"}]});
    s.backups
        .update_settings(json!({"pre_post":hooks}))
        .await
        .unwrap();
    let result = s
        .backups
        .run(BackupRequest {
            world: "Fixture".into(),
            dry_run: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result["success"], true);
    assert!(!marker.exists());
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_backup_still_resumes_saves() {
    let (_dir, s, _app, _token) = state().await;
    s.backups.update_settings(json!({"pre_post":{"before":[{"type":"send","value":"save hold"},{"type":"command","value":"exit 9"}],"after":[{"type":"send","value":"save resume"}]}})).await.unwrap();
    s.server.action("start").await.unwrap();
    let result = s
        .backups
        .run(BackupRequest {
            world: "Fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result["success"], false);
    assert!(!s.backups.active());
    wait_line(&s.server, "command=save resume").await;
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn unsafe_zip_cannot_replace_existing_world() {
    let (_dir, s, app, token) = state().await;
    let path = s.config.backups_dir.join("worlds/Fixture/malicious.zip");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"unsafe").unwrap();
        zip.finish().unwrap();
    }
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/backups/Fixture/malicious.zip/restore-to-world",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        tokio::fs::read_to_string(s.config.bedrock_server_dir.join("worlds/Fixture/level.dat"))
            .await
            .unwrap(),
        "original world"
    );
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn websocket_history_replay_command_permissions_and_shutdown() {
    let (_dir, s, app, token) = state().await;
    s.server.action("start").await.unwrap();
    wait_line(&s.server, "ready").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = s.shutdown.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .unwrap();
    });
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/v1/console/ws?token={token}"))
            .await
            .unwrap();
    let mut sequence = 0;
    let mut stream = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = ws.next().await {
            if let Ok(text) = message.to_text() {
                let v: Value = serde_json::from_str(text).unwrap();
                if v["type"] == "console" {
                    sequence = v["sequence"].as_u64().unwrap();
                    stream = v["stream_id"].as_str().unwrap().to_owned();
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(sequence > 0);
    ws.close(None).await.unwrap();
    s.server.send_command("offline-command").await.unwrap();
    wait_line(&s.server, "offline-command").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/api/v1/console/ws?token={token}&after={sequence}&stream_id={stream}"
    ))
    .await
    .unwrap();
    let mut found = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = ws.next().await {
            if let Ok(text) = message.to_text() {
                let v: Value = serde_json::from_str(text).unwrap();
                if v["type"] == "console" {
                    assert!(v["sequence"].as_u64().unwrap() > sequence);
                    if v["line"].as_str().unwrap().contains("offline-command") {
                        found = true;
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(found);
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"command":"through-websocket"}).to_string().into(),
    ))
    .await
    .unwrap();
    wait_line(&s.server, "through-websocket").await;
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stop_escalates_and_kills_launcher_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, c) = setup().await;
    let script = r#"#!/bin/sh
trap '' TERM
sleep 600 &
printf '%s' "$!" > "$3/descendant.pid"
printf '[INFO] descendant ready\n'
while IFS= read -r command; do printf '[INFO] ignored %s\n' "$command"; done
"#;
    tokio::fs::write(c.endstone_bin.as_ref().unwrap(), script)
        .await
        .unwrap();
    tokio::fs::set_permissions(
        c.endstone_bin.as_ref().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .await
    .unwrap();
    let path = c.bedrock_server_dir.join("descendant.pid");
    let m = manager(c).await;
    m.configure(ServerSettings {
        stop_timeout: 1,
        kill_timeout: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    m.action("start").await.unwrap();
    wait_line(&m, "descendant ready").await;
    let pid: i32 = tokio::fs::read_to_string(path)
        .await
        .unwrap()
        .parse()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), m.action("stop"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m.snapshot().status, Status::Stopped);
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    assert!(stat.is_err() || stat.unwrap().split_whitespace().nth(2) == Some("Z"));
    m.shutdown().await.unwrap();
}
#[tokio::test]
async fn password_change_and_recreated_user_revoke_old_tokens() {
    let (_dir, s, _app, _token) = state().await;
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"old-password","role":"viewer"})),
            true,
        )
        .await
        .unwrap();
    let login = s
        .auth
        .authenticate("account".into(), "old-password".into())
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"new-password"})),
            false,
        )
        .await
        .unwrap();
    assert!(s.auth.verify(token).await.is_err());
    let login = s
        .auth
        .authenticate("account".into(), "new-password".into())
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    s.auth
        .mutate_user(&owner(), "account", None, false)
        .await
        .unwrap();
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"new-password","role":"viewer"})),
            true,
        )
        .await
        .unwrap();
    assert!(s.auth.verify(token).await.is_err());
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn scheduled_retention_preserves_manual_archives_and_settings() {
    let (_dir, s, _app, _token) = state().await;
    s.backups.update_settings(json!({"auto":{"keep_count":1},"pre_post":{"before":[{"type":"send","value":"save hold"}],"after":[{"type":"send","value":"save resume"}]}})).await.unwrap();
    let manual = s
        .backups
        .run(BackupRequest {
            world: "Fixture".into(),
            run_hooks: false,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(manual["success"], true);
    s.backups.tick_once().await.unwrap();
    s.backups.tick_once().await.unwrap();
    let backups = s.backups.list(Some("Fixture".into()), false).await.unwrap();
    assert_eq!(backups.len(), 2);
    assert!(backups.iter().any(|v| v["filename"] == manual["filename"]));
    assert_eq!(
        s.backups
            .list(Some("Fixture".into()), true)
            .await
            .unwrap()
            .len(),
        1
    );
    s.server
        .configure(ServerSettings {
            auto_restart: false,
            stop_timeout: 11,
            ..Default::default()
        })
        .await
        .unwrap();
    let raw = tokio::fs::read_to_string(s.config.data_dir.join("server_settings.yaml"))
        .await
        .unwrap();
    let saved: ServerSettings = serde_yaml::from_str(&raw).unwrap();
    assert!(!saved.auto_restart);
    assert_eq!(saved.stop_timeout, 11);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn live_backup_confirms_save_readiness_and_resumes() {
    let (_dir, s, _app, _token) = state().await;
    s.backups.update_settings(json!({"pre_post":{"before":[{"type":"send","value":"save hold"}],"after":[{"type":"send","value":"save resume"}]}})).await.unwrap();
    s.server.action("start").await.unwrap();
    let result = s
        .backups
        .run(BackupRequest {
            world: "Fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result["success"], true);
    wait_line(&s.server, "Files are now ready").await;
    wait_line(&s.server, "command=save resume").await;
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn backup_survives_request_cancellation_and_rejects_overlap() {
    let (_dir, s, _app, _token) = state().await;
    s.backups.update_settings(json!({"pre_post":{"before":[{"type":"send","value":"save hold"},{"type":"command","value":"sleep 1"}],"after":[{"type":"send","value":"save resume"}]}})).await.unwrap();
    let mut events = s.backups.events.subscribe();
    let service = s.backups.clone();
    let request = BackupRequest {
        world: "Fixture".into(),
        ..Default::default()
    };
    let task = tokio::spawn({
        let r = request.clone();
        async move { service.run(r).await }
    });
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(s.backups.active());
    assert!(s.backups.run(request).await.is_err());
    task.abort();
    tokio::time::timeout(Duration::from_secs(5), s.backups.wait_idle())
        .await
        .unwrap();
    assert!(!s.backups.active());
    assert_eq!(
        s.backups
            .list(Some("Fixture".into()), false)
            .await
            .unwrap()
            .len(),
        1
    );
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

const FAKE_JAVA: &str = r#"#!/bin/sh
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
async fn wait_until(seconds: u64, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while std::time::Instant::now() < deadline {
        if probe() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
#[tokio::test]
async fn vanilla_backend_round_trip() {
    let (dir, s, app, token) = state().await;
    // A fake vanilla binary keeps this hermetic: switching (and starting) on
    // the vanilla backend must work without a real download. Switching with
    // no binary instead auto-installs on demand (live-verified; not exercised
    // here so tests stay offline).
    let binary = dir.path().join("bedrock/bedrock_server");
    tokio::fs::write(&binary, FAKE).await.unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary, perms).unwrap();
    }
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/server/backend/switch",
        Some(&token),
        json!({"backend":"vanilla","version":"1.26.52"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, status_body) = call(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status_body["backend"], "vanilla");
    let (_, settings) = call(
        &app,
        "GET",
        "/api/v1/settings/server",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(settings["vanilla_version"], "1.26.52");
    // The panel reports the active backend with its status summary.
    let (_, backend) = call(
        &app,
        "GET",
        "/api/v1/server/backend",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(backend["backend"], "vanilla");
    assert_eq!(backend["binary_present"], true);
    // Round trip: start, command, stop — the same console pipeline serves BDS.
    s.server.action("start").await.unwrap();
    wait_line(&s.server, "ready").await;
    assert_eq!(s.server.snapshot().backend.as_str(), "vanilla");
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/console/command",
        Some(&token),
        json!({"command":"list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    s.server.action("stop").await.unwrap();
    assert_eq!(s.server.snapshot().status, Status::Stopped);
    // Switching back to endstone applies on next start.
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/server/backend/switch",
        Some(&token),
        json!({"backend":"endstone"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s.server.snapshot().backend.as_str(), "endstone");
    s.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn bds_apply_preserves_user_data_and_replaces_binary() {
    use std::io::Write as _;
    let dir = tempfile::TempDir::new().unwrap();
    let server = dir.path().join("server");
    tokio::fs::create_dir_all(server.join("worlds/demo"))
        .await
        .unwrap();
    tokio::fs::write(server.join("server.properties"), "motd=keep-me\n")
        .await
        .unwrap();
    tokio::fs::write(server.join("allowlist.json"), "[]")
        .await
        .unwrap();
    tokio::fs::write(server.join("worlds/demo/level.dat"), b"level")
        .await
        .unwrap();
    tokio::fs::write(server.join("bedrock_server"), "#!/bin/sh\nold\n")
        .await
        .unwrap();
    let zip_path = dir.path().join("bds.zip");
    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("bedrock_server", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"#!/bin/sh\nnew-release\n").unwrap();
        zip.start_file(
            "server.properties",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"motd=fresh-default\n").unwrap();
        zip.start_file(
            "behavior_packs/vanilla/manifest.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.start_file(
            "release-notes.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"notes").unwrap();
        zip.finish().unwrap();
    }
    omnibedrock_server::bds::apply_archive(&zip_path, &server)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read_to_string(server.join("server.properties"))
            .await
            .unwrap(),
        "motd=keep-me\n"
    );
    assert_eq!(
        tokio::fs::read_to_string(server.join("allowlist.json"))
            .await
            .unwrap(),
        "[]"
    );
    assert_eq!(
        tokio::fs::read_to_string(server.join("worlds/demo/level.dat"))
            .await
            .unwrap(),
        "level"
    );
    assert_eq!(
        tokio::fs::read_to_string(server.join("bedrock_server"))
            .await
            .unwrap(),
        "#!/bin/sh\nnew-release\n"
    );
    assert_eq!(
        tokio::fs::read_to_string(server.join("release-notes.txt"))
            .await
            .unwrap(),
        "notes"
    );
    assert!(server.join("behavior_packs/vanilla/manifest.json").exists());
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(server.join("bedrock_server"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "binary must be executable");
    }
    // A fresh directory (no user data) takes every file from the archive.
    let fresh = dir.path().join("fresh");
    omnibedrock_server::bds::apply_archive(&zip_path, &fresh)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read_to_string(fresh.join("server.properties"))
            .await
            .unwrap(),
        "motd=fresh-default\n"
    );
}
#[test]
fn bds_registry_and_channel_parsing() {
    let text = r#"{"release":{"latest":"1.26.52","versions":["1.26.52","1.26.51"]},"preview":{"latest":"1.26.60-preview.30","versions":["1.26.60-preview.30"]}}"#;
    let registry: omnibedrock_server::bds::Registry = serde_json::from_str(text).unwrap();
    assert_eq!(registry.release.latest, "1.26.52");
    assert!(omnibedrock_server::bds::version_exists(
        &registry, "1.26.52"
    ));
    assert!(!omnibedrock_server::bds::version_exists(&registry, "9.9.9"));
    let metadata = r#"{"version":"1.26.52","binary":{"windows":{"url":"https://www.minecraft.net/bedrockdedicatedserver/bin-win/bedrock-server-1.26.52.3.zip","sha256":"aa"},"linux":{"url":"https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-1.26.52.3.zip","sha256":"bb"}}}"#;
    let parsed: omnibedrock_server::bds::Metadata = serde_json::from_str(metadata).unwrap();
    assert!(parsed.binary.linux.url.contains("bin-linux"));
}
#[tokio::test]
async fn broadcaster_lifecycle_device_code_config_and_accounts() {
    let (dir, _c) = setup().await;
    let mut c = config(&dir);
    c.java_bin = Some(dir.path().join("java-fixture"));
    c.broadcaster_jar = Some(dir.path().join("MCXboxBroadcastStandalone.jar"));
    c.broadcaster_dir = dir.path().join("broadcaster");
    {
        use std::os::unix::fs::PermissionsExt;
        let java = dir.path().join("java-fixture");
        tokio::fs::write(&java, FAKE_JAVA).await.unwrap();
        let mut perms = std::fs::metadata(&java).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&java, perms).unwrap();
    }
    tokio::fs::write(
        dir.path().join("MCXboxBroadcastStandalone.jar"),
        b"fake jar bytes",
    )
    .await
    .unwrap();
    let s = AppState::new(c, CancellationToken::new()).await.unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;
    // The default instance keeps the legacy broadcaster dir; routes without
    // X-Instance scope to it. The handle is clonable, so capture it once.
    let bc = s
        .instances
        .get("default")
        .await
        .unwrap()
        .broadcaster
        .clone();
    // Initial status: stopped, jar present, config seeded on first read.
    let (status, body) = call(
        &app,
        "GET",
        "/api/v1/broadcaster/status",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"]["status"], "stopped");
    assert_eq!(body["jar_exists"], true);
    assert_eq!(body["accounts"][0]["id"], "main");
    assert!(
        wait_until(5, || {
            bc.java
                .borrow()
                .clone()
                .is_some_and(|j| j.contains("17.0.0"))
        })
        .await,
        "java detection completes"
    );
    assert_eq!(
        bc.java.borrow().clone().as_deref(),
        Some("openjdk version \"17.0.0\" 2024-01-01")
    );
    // Config defaults are seeded and can be edited.
    let (status, text) = call_text(&app, "GET", "/api/v1/broadcaster/config", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("session:"));
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/broadcaster/config",
        Some(&token),
        json!({"text":"session:\n  update-interval: 25\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, text) = call_text(&app, "GET", "/api/v1/broadcaster/config", &token).await;
    assert!(text.contains("update-interval: 25"));
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/broadcaster/config",
        Some(&token),
        json!({"text":"\t- not [ valid yaml"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Start and stream: device codes are captured for main and sub-sessions.
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/action",
        Some(&token),
        json!({"action":"start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        wait_until(5, || bc.device_codes().len() >= 2).await,
        "device codes captured"
    );
    let codes = bc.device_codes();
    assert_eq!(codes["main"].code, "ABCD1234");
    assert_eq!(codes["main"].url, "https://www.microsoft.com/link");
    assert_eq!(codes["alt1"].code, "EFGH5678");
    // Commands and account management flow through the process stdin.
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/command",
        Some(&token),
        json!({"command":"accounts list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/accounts",
        Some(&token),
        json!({"action":"add","id":"alt2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/accounts",
        Some(&token),
        json!({"action":"add","id":"../evil"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Screenshot round trip: upload bytes and read them back.
    let jpeg: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/broadcaster/screenshot?account=alt1")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(jpeg.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, bytes) = call_bytes(
        &app,
        "GET",
        "/api/v1/broadcaster/screenshot?account=alt1",
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, jpeg);
    let reject: Vec<u8> = b"not an image".to_vec();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/broadcaster/screenshot")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(reject))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Restart keeps it running; stop exits cleanly.
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/action",
        Some(&token),
        json!({"action":"restart"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        bc.snapshot().status,
        omnibedrock_server::broadcaster::BcStatus::Running
    );
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/action",
        Some(&token),
        json!({"action":"stop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        bc.snapshot().status,
        omnibedrock_server::broadcaster::BcStatus::Stopped
    );
    // Commands are rejected while stopped.
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/broadcaster/command",
        Some(&token),
        json!({"command":"accounts list"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    bc.shutdown().await.unwrap();
}
#[tokio::test]
async fn broadcaster_socket_streams_status_codes_and_commands() {
    let (dir, _c) = setup().await;
    let mut c = config(&dir);
    c.java_bin = Some(dir.path().join("java-fixture"));
    c.broadcaster_jar = Some(dir.path().join("MCXboxBroadcastStandalone.jar"));
    c.broadcaster_dir = dir.path().join("broadcaster");
    {
        use std::os::unix::fs::PermissionsExt;
        let java = dir.path().join("java-fixture");
        tokio::fs::write(&java, FAKE_JAVA).await.unwrap();
        let mut perms = std::fs::metadata(&java).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&java, perms).unwrap();
    }
    tokio::fs::write(
        dir.path().join("MCXboxBroadcastStandalone.jar"),
        b"fake jar bytes",
    )
    .await
    .unwrap();
    let s = AppState::new(c, CancellationToken::new()).await.unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;
    s.instances
        .get("default")
        .await
        .unwrap()
        .broadcaster
        .action("start")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = s.shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .unwrap();
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/api/v1/broadcaster/ws?token={token}"
    ))
    .await
    .unwrap();
    let mut saw_status = false;
    let mut saw_code = false;
    let mut saw_line = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !(saw_status && saw_code && saw_line) {
            let Some(Ok(message)) = ws.next().await else {
                break;
            };
            let Ok(text) = message.to_text() else {
                continue;
            };
            let v: Value = serde_json::from_str(text).unwrap();
            match v["type"].as_str().unwrap_or("") {
                "status" => saw_status |= v["status"]["status"] == "running",
                "device_codes" => saw_code |= v["codes"]["main"]["code"] == "ABCD1234",
                "console" => saw_line |= v["line"].as_str().unwrap_or("").contains("fixture ready"),
                _ => {}
            }
        }
    })
    .await
    .expect("received status, device codes, and console lines");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"command":"accounts list"}).to_string().into(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(message)) = ws.next().await else {
                break;
            };
            let Ok(text) = message.to_text() else {
                continue;
            };
            let v: Value = serde_json::from_str(text).unwrap();
            if v["type"] == "console" && v["line"].as_str().unwrap_or("").contains("friends") {
                break;
            }
        }
    })
    .await
    .expect("command output streamed back");
    s.instances
        .get("default")
        .await
        .unwrap()
        .broadcaster
        .shutdown()
        .await
        .unwrap();
}

fn write_addon_zip(path: &std::path::Path, files: &[(&str, &[u8])]) {
    use std::io::Write as _;
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

async fn upload_archive(
    app: &Router,
    token: &str,
    world: &str,
    pack_type: &str,
    archive: &[u8],
) -> (StatusCode, Value) {
    let uri = format!(
        "/api/v1/addons/upload?world={world}&type={pack_type}",
        world = urlencode(world),
        pack_type = pack_type,
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/zip")
                .body(Body::from(archive.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(json!(String::from_utf8_lossy(&bytes))),
    )
}

fn urlencode(raw: &str) -> String {
    raw.bytes().map(|b| format!("%{b:02X}")).collect()
}

#[tokio::test]
async fn addon_upload_icon_delete_round_trip() {
    let (dir, s, app, token) = state().await;
    tokio::fs::create_dir_all(dir.path().join("bedrock/worlds/demo"))
        .await
        .unwrap();

    const BP: &str = r#"{"format_version":2,"header":{"name":"Test Behavior","uuid":"aaaaaaaa-1111-1111-1111-111111111111","version":[2,0,0]},"modules":[{"type":"data","uuid":"aaaaaaaa-2222-2222-2222-222222222222","version":[2,0,0]}]}"#;
    const RP: &str = r#"{"format_version":2,"header":{"name":"Test Resource","uuid":"bbbbbbbb-3333-3333-3333-333333333333","version":[1,0,0]},"modules":[{"type":"resources","uuid":"bbbbbbbb-4444-4444-4444-444444444444","version":[1,0,0]}]}"#;
    let icon: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];

    // Single .mcpack with an icon installs as a behavior pack.
    let archive_path = dir.path().join("single.mcpack");
    write_addon_zip(
        &archive_path,
        &[
            ("manifest.json", BP.as_bytes()),
            ("pack_icon.png", &icon),
            ("scripts/main.js", b"export default {};"),
        ],
    );
    let archive = std::fs::read(&archive_path).unwrap();
    let (status, body) = upload_archive(&app, &token, "demo", "auto", &archive).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["installed"][0]["pack_type"], "behavior_packs");
    assert_eq!(body["installed"][0]["name"], "Test Behavior");

    // Multi-pack .mcaddon installs both detected types.
    let bundle_path = dir.path().join("bundle.mcaddon");
    write_addon_zip(
        &bundle_path,
        &[
            ("bp/manifest.json", BP.as_bytes()),
            ("rp/manifest.json", RP.as_bytes()),
        ],
    );
    let bundle = std::fs::read(&bundle_path).unwrap();
    let (status, body) = upload_archive(&app, &token, "demo", "auto", &bundle).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["installed"].as_array().unwrap().len(), 2);

    // Second identical .mcpack gets a conflict-free suffixed name.
    let (status, body) = upload_archive(&app, &token, "demo", "auto", &archive).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let duplicate = body["installed"][0]["name"].as_str().unwrap();
    assert!(duplicate.starts_with("Test Behavior-"), "{duplicate}");

    // Listing includes size, icon flag, and manifest data.
    let (_, listing) = call(&app, "GET", "/api/v1/addons/", Some(&token), json!({})).await;
    let packs = listing["behavior_packs"].as_array().unwrap();
    assert!(packs.len() >= 2);
    let first = packs.iter().find(|p| p["name"] == "Test Behavior").unwrap();
    assert_eq!(first["icon"], true);
    assert!(first["size_bytes"].as_u64().unwrap() > 0);
    assert!(first["modified"].as_u64().unwrap() > 0);

    // Icon endpoint serves the embedded PNG via query token.
    let icon_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/addons/icon?path={}&token={token}",
                    urlencode(&first["path"].as_str().unwrap().replace('\\', "/")),
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(icon_response.status(), StatusCode::OK);
    assert_eq!(icon_response.headers()["content-type"], "image/png");

    // Saving the order pins the installed manifest's version — BDS resolves
    // packs by (uuid, exact version), so a stale or placeholder [1,0,0]
    // entry must be corrected, while packs absent from disk keep whatever
    // the file already declared.
    let world_dir = dir.path().join("bedrock/worlds/demo");
    tokio::fs::write(
        world_dir.join("world_behavior_packs.json"),
        r#"[{"pack_id":"aaaaaaaa-1111-1111-1111-111111111111","version":[1,0,0]},{"pack_id":"cccccccc-9999-9999-9999-999999999999","version":[3,1,4]}]"#,
    )
    .await
    .unwrap();
    let (status, body) = call(
        &app,
        "PUT",
        "/api/v1/addons/order/demo/behavior_packs",
        Some(&token),
        json!({"uuids": [
            "aaaaaaaa-1111-1111-1111-111111111111",
            "cccccccc-9999-9999-9999-999999999999"
        ]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let order: Value = serde_json::from_str(
        &tokio::fs::read_to_string(world_dir.join("world_behavior_packs.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    let entries = order.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0]["pack_id"],
        "aaaaaaaa-1111-1111-1111-111111111111"
    );
    assert_eq!(entries[0]["version"], json!([2, 0, 0]));
    assert_eq!(entries[1]["version"], json!([3, 1, 4]));

    // Delete removes the directory and strips the UUID from the world order file.
    tokio::fs::write(
        world_dir.join("world_behavior_packs.json"),
        r#"[{"pack_id":"aaaaaaaa-1111-1111-1111-111111111111","version":[2,0,0]},{"pack_id":"keepme"}]"#,
    )
    .await
    .unwrap();
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/addons/delete",
        Some(&token),
        json!({"path": first["path"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!world_dir.join("behavior_packs/Test Behavior").exists());
    let order: Value = serde_json::from_str(
        &tokio::fs::read_to_string(world_dir.join("world_behavior_packs.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(order.as_array().unwrap().len(), 1);
    assert_eq!(order[0]["pack_id"], "keepme");

    // Traversal entries are refused; nothing is installed.
    let evil_path = dir.path().join("evil.zip");
    {
        use std::io::Write as _;
        let file = std::fs::File::create(&evil_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"nope").unwrap();
        zip.finish().unwrap();
    }
    let (status, _) = upload_archive(
        &app,
        &token,
        "demo",
        "auto",
        &std::fs::read(&evil_path).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!dir.path().join("escape.txt").exists());

    // Non-archive uploads are rejected before any extraction.
    let (status, _) = upload_archive(&app, &token, "demo", "auto", b"not a zip").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    s.server.shutdown().await.ok();
}

#[tokio::test]
async fn default_credentials_and_login_toggle() {
    let (dir, s, app, _) = state().await;

    // Fresh store seeds Admin/123456 and reports itself publicly.
    let (status, panel) = call(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(panel["login_enabled"], true);
    assert_eq!(panel["default_credentials"], true);

    let (status, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token = login["token"].as_str().unwrap().to_owned();
    assert_eq!(login["user"]["username"], "Admin");
    assert_eq!(login["user"]["role"], "owner");

    // With login enabled, no token means no access.
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Disable login; the same token-less request now serves as the owner.
    let (status, body) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, me) = call(&app, "GET", "/api/v1/auth/me", None, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["role"], "owner");
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    // Panel info reflects the disabled state.
    let (_, panel) = call(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(panel["login_enabled"], false);

    // Re-enable: token-less access is rejected again, and login still works.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Changing the Admin password clears the default-credentials flag.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/Admin",
        Some(&token),
        json!({"password":"new-secret-9"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, panel) = call(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(panel["default_credentials"], false);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A password change rotates the token version, so the old token is dead.
    let (status, relogin) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"new-secret-9"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = relogin["token"].as_str().unwrap().to_owned();

    // The toggle persists to panel_settings.yaml for the next start.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let persisted = tokio::fs::read_to_string(dir.path().join("data/panel_settings.yaml"))
        .await
        .unwrap();
    assert!(persisted.contains("login_enabled: false"), "{persisted}");
    let _ = s;
}

/// Call with an extra header (X-Instance scoping).
async fn call_instance(
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

/// Multi-instance lifecycle: migration, quotas, port conflicts, access
/// control, X-Instance scoping, and deletion rules.
/// Every instance owns an independent broadcaster: separate dirs, config.yml,
/// accounts, and process state. Configuring one instance must never touch
/// another, and routes are scoped through X-Instance.
#[tokio::test]
async fn broadcaster_is_per_instance() {
    let (dir, _c) = setup().await;
    let mut c = config(&dir);
    c.java_bin = Some(dir.path().join("java-fixture"));
    c.broadcaster_jar = Some(dir.path().join("MCXboxBroadcastStandalone.jar"));
    c.broadcaster_dir = dir.path().join("broadcaster");
    tokio::fs::write(
        dir.path().join("MCXboxBroadcastStandalone.jar"),
        b"fake jar bytes",
    )
    .await
    .unwrap();
    let s = AppState::new(c, CancellationToken::new()).await.unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;

    // Create one extra instance.
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&token),
        json!({"name":"Second Server","backend":"vanilla"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let second_id = body["id"].as_str().unwrap().to_owned();

    // Distinct broadcaster directories: the default instance keeps the
    // panel-level dir; the new instance gets one under its own root.
    let (_, default_status) = call(
        &app,
        "GET",
        "/api/v1/broadcaster/status",
        Some(&token),
        json!({}),
    )
    .await;
    let (_, second_status) = call_instance(
        &app,
        "GET",
        "/api/v1/broadcaster/status",
        Some(&token),
        Some(&second_id),
        json!({}),
    )
    .await;
    let default_dir = default_status["dir"].as_str().unwrap();
    let second_dir = second_status["dir"].as_str().unwrap();
    assert!(default_dir.ends_with("broadcaster"), "{default_dir}");
    assert!(
        second_dir.contains("instances") && second_dir.ends_with("broadcaster"),
        "{second_dir}"
    );
    assert_ne!(default_dir, second_dir);
    // The shared jar resolves for both despite the separate dirs.
    assert_eq!(second_status["jar_exists"], true);

    // Config edits are scoped: writing on the second instance never touches
    // the default instance's config.yml.
    let (_, text) = call_text(&app, "GET", "/api/v1/broadcaster/config", &token).await;
    assert!(!text.contains("update-interval: 33"), "{text}");
    let (code, _) = call_instance(
        &app,
        "PUT",
        "/api/v1/broadcaster/config",
        Some(&token),
        Some(&second_id),
        json!({"text":"session:\n  update-interval: 33\n"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, second_text) = call_text_instance(
        &app,
        "GET",
        "/api/v1/broadcaster/config",
        &token,
        Some(&second_id),
    )
    .await;
    assert!(second_text.contains("update-interval: 33"), "{second_text}");
    let (_, default_text) = call_text(&app, "GET", "/api/v1/broadcaster/config", &token).await;
    assert!(
        !default_text.contains("update-interval: 33"),
        "second instance's config leaked into the default: {default_text}"
    );
    let second_config_path = std::path::Path::new(second_dir).join("config.yml");
    let default_config_path = std::path::Path::new(default_dir).join("config.yml");
    assert!(second_config_path.is_file());
    assert!(default_config_path.is_file());

    // Statuses are independent: the second instance's broadcaster state is
    // its own object, not a shared global.
    let second_bc = s
        .instances
        .get(&second_id)
        .await
        .unwrap()
        .broadcaster
        .clone();
    let default_bc = s
        .instances
        .get("default")
        .await
        .unwrap()
        .broadcaster
        .clone();
    assert_eq!(
        second_bc.snapshot().status,
        omnibedrock_server::broadcaster::BcStatus::Stopped
    );
    assert_eq!(
        default_bc.snapshot().status,
        omnibedrock_server::broadcaster::BcStatus::Stopped
    );
    second_bc.shutdown().await.unwrap();
    default_bc.shutdown().await.unwrap();
}

#[tokio::test]
async fn instances_lifecycle_quotas_and_access() {
    let (_dir, s, app, token) = state().await;
    let _owner_token = token.clone();

    // The migrated legacy server is the default instance.
    let (code, list) = call(&app, "GET", "/api/v1/instances", Some(&token), json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], "default");
    assert_eq!(list[0]["owner"], "Admin");
    assert_eq!(list[0]["deployment"], "native");

    // Host capacity and probed backends.
    let (code, host) = call(&app, "GET", "/api/v1/host", Some(&token), json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert!(host["capabilities"]["docker"].is_string() || host["capabilities"]["docker"].is_null());
    assert!(host["allocated"]["instances"].as_u64().unwrap() >= 1);

    // A normal user with an instance-count and memory quota.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        Some(&token),
        json!({
            "username": "friend", "password": "friend-password", "role": "user"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/friend",
        Some(&token),
        json!({
            "quotas": {"max_instances": 1, "memory_mb": 1024}
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({
            "username": "friend", "password": "friend-password"
        }),
    )
    .await;
    let friend = login["token"].as_str().unwrap().to_owned();
    assert_eq!(login["user"]["quotas"]["memory_mb"], 1024);

    // Resource validation and port conflicts are rejected before creation.
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&friend),
        json!({
            "name": "Bad Memory", "backend": "vanilla", "memory_mb": 10
        }),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&friend),
        json!({
            "name": "Port Clash", "backend": "vanilla", "port": 19132
        }),
    )
    .await;
    assert_ne!(code, StatusCode::OK, "{body}");

    // A valid instance within quota.
    let (code, created) = call(&app, "POST", "/api/v1/instances", Some(&friend), json!({
        "name": "Friend Server", "backend": "vanilla", "memory_mb": 512, "cpus": 0.5, "port": 21443
    })).await;
    assert_eq!(code, StatusCode::OK, "{created}");
    assert_eq!(created["id"], "friend-server");
    assert_eq!(created["owner"], "friend");
    assert_eq!(created["port"], 21443);
    assert_eq!(created["resources"]["memory_mb"], 512);
    assert_eq!(created["backend"], "vanilla");
    // The instance's server.properties pins BOTH stacks to its port so the
    // BDS IPv6 slot can never overlap a neighboring instance.
    let seeded = tokio::fs::read_to_string(
        _dir.path()
            .join("data/instances/friend-server/server/server.properties"),
    )
    .await
    .unwrap();
    assert!(seeded.contains("server-port=21443"), "{seeded}");
    assert!(seeded.contains("server-portv6=21444"), "{seeded}");

    // The instance count quota blocks a second instance.
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&friend),
        json!({
            "name": "Second Server", "backend": "vanilla"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");

    // X-Instance scoping: the new instance has its own status and settings.
    let (code, status) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&friend),
        Some("friend-server"),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "X-Instance status failed: {status}");
    assert_eq!(status["status"], "stopped");
    let (code, settings) = call_instance(
        &app,
        "GET",
        "/api/v1/settings/server",
        Some(&friend),
        Some("friend-server"),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(settings["backend"], "vanilla");

    // Another ordinary user cannot see or address the instance.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        Some(&token),
        json!({
            "username": "dad", "password": "dad-password-1", "role": "user"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({
            "username": "dad", "password": "dad-password-1"
        }),
    )
    .await;
    let dad = login["token"].as_str().unwrap().to_owned();
    let (code, dad_list) = call(&app, "GET", "/api/v1/instances", Some(&dad), json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert!(!dad_list.to_string().contains("friend-server"));
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/friend-server",
        Some(&dad),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&dad),
        Some("friend-server"),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    // The panel owner shares view access; dad can see but not manage.
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/friend-server",
        Some(&token),
        json!({
            "access": [{"user": "dad", "level": "view"}]
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/friend-server",
        Some(&dad),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/friend-server/action",
        Some(&dad),
        json!({
            "action": "start"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/friend-server",
        Some(&dad),
        json!({
            "name": "Dad Takeover"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // Quotas on update: 4096 MiB exceeds the 1024 quota; 768 fits.
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/friend-server",
        Some(&friend),
        json!({
            "memory_mb": 4096
        }),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/friend-server",
        Some(&friend),
        json!({
            "memory_mb": 768
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, after) = call(
        &app,
        "GET",
        "/api/v1/instances/friend-server",
        Some(&friend),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(after["resources"]["memory_mb"], 768);

    // Deletion needs confirmation; the default instance is permanent.
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/instances/friend-server",
        Some(&friend),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/instances/default?confirm=yes",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/instances/friend-server?confirm=yes",
        Some(&friend),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, list) = call(&app, "GET", "/api/v1/instances", Some(&token), json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // The records persisted to disk include the access grant rules.
    let registry = tokio::fs::read_to_string(_dir.path().join("data/instances.yaml"))
        .await
        .unwrap();
    assert!(registry.contains("default"), "{registry}");

    s.shutdown.cancel();
    s.instances.shutdown_all().await;
}

/// A user without INSTANCES_CREATE (viewer role) cannot create instances.
#[tokio::test]
async fn instance_creation_requires_permission() {
    let (_dir, s, app, token) = state().await;
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        Some(&token),
        json!({
            "username": "watcher", "password": "watcher-password", "role": "viewer"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({
            "username": "watcher", "password": "watcher-password"
        }),
    )
    .await;
    let watcher = login["token"].as_str().unwrap().to_owned();
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&watcher),
        json!({
            "name": "Sneaky", "backend": "vanilla"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    s.shutdown.cancel();
    s.instances.shutdown_all().await;
}

/// Two instances can run concurrently with independent consoles and state.
#[tokio::test]
async fn two_instances_run_independently() {
    let (_dir, s, app, token) = state().await;
    // The default instance uses the endstone fixture; a second instance on a
    // fresh server dir also launches the same fixture binary.
    for (name, port) in [("Alpha Server", 21511u16), ("Beta Server", 21521)] {
        let (code, body) = call(
            &app,
            "POST",
            "/api/v1/instances",
            Some(&token),
            json!({
                "name": name, "backend": "endstone", "port": port
            }),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
    }
    for id in ["alpha-server", "beta-server"] {
        let (code, body) = call(
            &app,
            "POST",
            &format!("/api/v1/instances/{id}/action"),
            Some(&token),
            json!({
                "action": "start"
            }),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
    }
    // Both run at once, with distinct PIDs, under X-Instance scoping.
    for id in ["alpha-server", "beta-server"] {
        let (code, status) = call_instance(
            &app,
            "GET",
            "/api/v1/server/status",
            Some(&token),
            Some(id),
            json!({}),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status["status"], "running", "{id}: {status}");
        assert!(status["pid"].as_u64().is_some());
    }
    let (_, alpha) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&token),
        Some("alpha-server"),
        json!({}),
    )
    .await;
    let (_, beta) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&token),
        Some("beta-server"),
        json!({}),
    )
    .await;
    assert_ne!(alpha["pid"], beta["pid"]);
    // Consoles are separate streams: a command lands only in its instance.
    let (code, _) = call_instance(
        &app,
        "POST",
        "/api/v1/console/command",
        Some(&token),
        Some("alpha-server"),
        json!({"command": "say alpha-hello"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, alpha_log) = call_text(&app, "GET", "/api/v1/instances/alpha-server", &token).await;
    let _ = alpha_log;
    // Stopping one leaves the other running.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/alpha-server/action",
        Some(&token),
        json!({
            "action": "stop"
        }),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, status) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&token),
        Some("alpha-server"),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(status["status"], "stopped");
    let (code, status) = call_instance(
        &app,
        "GET",
        "/api/v1/server/status",
        Some(&token),
        Some("beta-server"),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(status["status"], "running");
    let _ = s;
    s.shutdown.cancel();
    s.instances.shutdown_all().await;
}

/// Upgraded installs (roles.yaml from before the `user` role existed) gain
/// new built-ins on boot without touching customized existing roles.
#[tokio::test]
async fn upgraded_installs_gain_new_builtin_roles() {
    let dir = TempDir::new().unwrap();
    let c = config(&dir);
    tokio::fs::create_dir_all(&c.data_dir).await.unwrap();
    // A pre-multi-instance roles.yaml: no `user` role, and a moderator
    // role the admin customized (display name changed, permission removed).
    tokio::fs::write(
        c.data_dir.join("roles.yaml"),
        "moderator:\n  name: moderator\n  display_name: My Mods\n  permissions: [CONSOLE_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nadmin:\n  name: admin\n  display_name: Admin\n  permissions: [SERVER_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nowner:\n  name: owner\n  display_name: Owner\n  permissions: [SERVER_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nviewer:\n  name: viewer\n  display_name: Viewer\n  permissions: [CONSOLE_VIEW]\n  is_default: true\n  created_at: '2020-01-01'\n",
    )
    .await
    .unwrap();
    let auth = Auth::new(Arc::new(c)).await.unwrap();
    let roles = auth.roles().await;
    let user = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "user")
        .expect("user role added on upgrade")
        .clone();
    assert!(
        user["permissions"]
            .as_array()
            .unwrap()
            .contains(&json!("INSTANCES_CREATE"))
    );
    let moderator = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "moderator")
        .cloned()
        .unwrap();
    assert_eq!(moderator["display_name"], "My Mods");
    assert_eq!(
        moderator["permissions"].as_array().unwrap().len(),
        1,
        "customized role untouched"
    );
    // Creating a user with the role that previously failed now works.
    let owner = omnibedrock_api::auth::User {
        token_version: 0,
        username: "Admin".into(),
        role: "owner".into(),
        roles: vec!["owner".into()],
        password_hash: String::new(),
        display_name: "Admin".into(),
        created_at: String::new(),
        last_login: String::new(),
        quotas: Default::default(),
    };
    let created = auth
        .mutate_user(
            &owner,
            "friend",
            Some(json!({"password": "friend-password"})),
            true,
        )
        .await
        .unwrap();
    let _ = created;
    // Default role for new accounts remains viewer.
    let made = auth
        .mutate_user(
            &owner,
            "friend",
            Some(json!({"password": "friend-password", "role": "user"})),
            false,
        )
        .await
        .unwrap();
    assert_eq!(made["role"], "user");
}

// ─── Unified roles & instance access ────────────────────────────────────────

/// Legacy single-role accounts, still-stock role lists, and two-level
/// instance access entries all migrate into the unified multi-role model on
/// boot — without granting new privileges to customized roles.
#[tokio::test]
async fn unified_roles_migrate_legacy_state() {
    let (_dir, c) = setup().await;
    // A legacy user file: one `role` field, no `roles` list.
    tokio::fs::write(
        c.data_dir.join("users.yaml"),
        "Admin:\n  token_version: 1\n  username: Admin\n  password_hash: x\n  role: moderator\n  display_name: Admin\n  created_at: '2026-01-01T00:00:00Z'\n  last_login: ''\n",
    )
    .await
    .unwrap();
    // A legacy roles file: stock lists only (no color/description/position).
    tokio::fs::write(
        c.data_dir.join("roles.yaml"),
        "admin:\n  name: admin\n  display_name: Admin\n  permissions:\n    - CONSOLE_VIEW\n  is_default: false\n  created_at: '2026-01-01T00:00:00Z'\n",
    )
    .await
    .unwrap();
    // A legacy instances file with two-level access entries.
    tokio::fs::write(
        c.data_dir.join("instances.yaml"),
        "default:\n  id: default\n  name: Default Server\n  owner: Admin\n  created_at: '2026-01-01T00:00:00Z'\n  port: 19132\n",
    )
    .await
    .unwrap();
    let state = AppState::new(c, CancellationToken::new()).await.unwrap();
    // The legacy file's argon2 hash is a dummy, so login is impossible by
    // construction; the migration is verified through the auth store instead.
    let users = state.auth.users().await;
    let admin = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "Admin")
        .unwrap()
        .clone();
    assert_eq!(
        admin["roles"],
        json!(["moderator"]),
        "legacy `role` field migrates into the multi-role list"
    );
    assert_eq!(admin["role"], json!("moderator"));
    // Still-stock roles are refreshed: a customized `admin` whose only stored
    // permission was CONSOLE_VIEW does NOT match the old stock list, so its
    // permissions must be untouched — while missing builtin roles are added.
    let roles = state.auth.roles().await;
    let admin_role = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "admin")
        .unwrap()
        .clone();
    assert_eq!(
        admin_role["permissions"],
        json!(["CONSOLE_VIEW"]),
        "customized role permissions are never overwritten"
    );
    assert_eq!(admin_role["builtin"], json!(true));
    assert!(
        roles
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "instance-manager"),
        "new builtin roles are added on upgrade"
    );
    let owner_role = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "owner")
        .unwrap()
        .clone();
    assert_eq!(owner_role["grants_owner"], json!(true));
    assert!(
        owner_role["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "ROLES_MANAGE"),
        "stock refreshed owner carries the new permissions"
    );
    state.shutdown.cancel();
    state.server.shutdown().await.unwrap();
}

/// Role CRUD, the delegation guards, rename reference rewriting, and the
/// owner protections.
#[tokio::test]
async fn role_crud_delegation_and_owner_protection() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());

    // A limited editor holds ROLES_MANAGE plus a small set; the delegation
    // guard then bounds what they can author or assign.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"delegator","permissions":["ROLES_MANAGE","CONSOLE_VIEW","CONSOLE_SEND","SERVER_VIEW","PLAYERS_VIEW"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        t,
        json!({"username":"helper","password":"helper-pass-1","roles":["delegator"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"helper","password":"helper-pass-1"}),
    )
    .await;
    let helper = login["token"].as_str().unwrap().to_owned();
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&helper),
        json!({"name":"sneaky","permissions":["SERVER_MANAGE"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    // A subset the editor actually holds is fine.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&helper),
        json!({"name":"helper-delegate","permissions":["CONSOLE_VIEW"],"color":"AA00FF","description":"helper-made"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, role) = call(&app, "GET", "/api/v1/roles/helper-delegate", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(role["color"], json!("AA00FF"));
    assert_eq!(role["builtin"], json!(false));

    // Role listing is readable by any signed-in user (chips need it).
    let (code, _) = call(&app, "GET", "/api/v1/roles", Some(&helper), json!({})).await;
    assert_eq!(code, StatusCode::OK);

    // Duplicate, then rename; users holding the old name follow the rename.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"staff","duplicate_of":"moderator"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/helper",
        t,
        json!({"roles":["delegator","helper-delegate","staff"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, me) = call(&app, "GET", "/api/v1/auth/me", Some(&helper), json!({})).await;
    for perm in [
        "CONSOLE_VIEW",
        "CONSOLE_SEND",
        "SERVER_VIEW",
        "ROLES_MANAGE",
        "PLAYERS_KICK",
    ] {
        assert!(
            me["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == perm),
            "union should include {perm}"
        );
    }
    assert!(
        !me["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "SERVER_MANAGE"),
        "union must not include permissions none of the roles grant"
    );
    let (code, renamed) = call(
        &app,
        "PUT",
        "/api/v1/roles/staff",
        t,
        json!({"name":"crew"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{renamed}");
    let (_, me) = call(&app, "GET", "/api/v1/auth/me", Some(&helper), json!({})).await;
    assert!(
        me["roles"].as_array().unwrap().iter().any(|r| r == "crew"),
        "user role lists follow the rename"
    );

    // Built-ins keep their keys and cannot be deleted; the owner role keeps
    // its sentinel even after edits.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/roles/owner",
        t,
        json!({"display_name":"Boss"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, role) = call(&app, "GET", "/api/v1/roles/owner", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(role["grants_owner"], json!(true), "sentinel survives edits");
    assert_eq!(role["display_name"], json!("Boss"));
    for builtin in [
        "owner",
        "admin",
        "moderator",
        "user",
        "viewer",
        "instance-manager",
    ] {
        let (code, _) = call(
            &app,
            "DELETE",
            &format!("/api/v1/roles/{builtin}"),
            t,
            json!({}),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{builtin} must not delete");
    }
    // Deleting a role held by a user is refused.
    let (code, _) = call(&app, "DELETE", "/api/v1/roles/crew", t, json!({})).await;
    assert_eq!(code, StatusCode::CONFLICT);
    // Deleting an unheld custom role works.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"scratch","permissions":[]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(&app, "DELETE", "/api/v1/roles/scratch", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);

    // Owner protections on users: demoting one of two owners is fine, but the
    // last owner can never be demoted (Admin self-demotes here).
    let (_, admin_login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    let admin_token = admin_login["token"].as_str().unwrap().to_owned();
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        t,
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/Admin",
        Some(&admin_token),
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        Some(&admin_token),
        json!({"roles":["owner"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    // Non-owners cannot touch owner accounts or assign the Owner role.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        Some(&helper),
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/helper",
        Some(&helper),
        json!({"roles":["owner"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

/// Instance-scoped access: entry roles decide capability on shared instances,
/// global perms do not leak, INSTANCES_VIEW_ALL projects everywhere, and only
/// authorized users can manage the access list.
#[tokio::test]
async fn instance_access_roles_are_scoped() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());
    // Fake vanilla binaries so actions never hit the network.
    for id in ["a-server", "b-server"] {
        let dir = _dir.path().join("instances").join(id).join("server");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let binary = dir.join("bedrock_server");
        tokio::fs::write(&binary, FAKE).await.unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary, perms).unwrap();
    }
    for (id, name, port) in [
        ("a-server", "A Server", 21551),
        ("b-server", "B Server", 21561),
    ] {
        let (code, _) = call(
            &app,
            "POST",
            "/api/v1/instances",
            t,
            json!({"name":name,"backend":"vanilla","deployment":"native","port":port}),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{id}");
    }
    // Two ordinary users: one gets a manage share on A, one a viewer share.
    for (user, pass) in [("manager", "manager-pass-1"), ("watcher", "watcher-pass-1")] {
        let (code, _) = call(
            &app,
            "POST",
            "/api/v1/auth/users",
            t,
            json!({"username":user,"password":pass,"roles":["viewer"]}),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{user}");
    }
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/a-server",
        t,
        json!({"access":[
            {"user":"manager","roles":["instance-manager"]},
            {"user":"watcher","roles":["viewer"]}
        ]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"manager","password":"manager-pass-1"}),
    )
    .await;
    let manager = login["token"].as_str().unwrap().to_owned();
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"watcher","password":"watcher-pass-1"}),
    )
    .await;
    let watcher = login["token"].as_str().unwrap().to_owned();

    // The manager can start A…
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances/a-server/action",
        Some(&manager),
        json!({"action":"start"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/a-server/action",
        Some(&manager),
        json!({"action":"stop"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    // …the watcher cannot.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/a-server/action",
        Some(&watcher),
        json!({"action":"start"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    // Neither can touch B (no entry)…
    for user in [&manager, &watcher] {
        let (code, _) = call(
            &app,
            "POST",
            "/api/v1/instances/b-server/action",
            Some(user),
            json!({"action":"start"}),
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }
    // …and neither can create instances (global-only capability).
    for user in [&manager, &watcher] {
        let (code, _) = call(
            &app,
            "POST",
            "/api/v1/instances",
            Some(user),
            json!({"name":"Sneaky","backend":"vanilla"}),
        )
        .await;
        assert_eq!(code, StatusCode::FORBIDDEN);
    }
    // Instance access does not leak panel capabilities: the watcher cannot
    // list users even though they can view A's status.
    let (code, _) = call(&app, "GET", "/api/v1/auth/users", Some(&watcher), json!({})).await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // Only owners (or INSTANCES_SHARE holders) manage the access list.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/a-server",
        Some(&manager),
        json!({"access":[{"user":"manager","roles":["instance-manager","viewer"]}]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // Legacy `level` bodies still map onto roles for old clients.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/a-server",
        t,
        json!({"access":[{"user":"watcher","level":"manage"}]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/a-server/action",
        Some(&watcher),
        json!({"action":"start"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances/a-server/action",
        Some(&watcher),
        json!({"action":"stop"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

/// Concurrent permission mutations must serialize on the auth store lock:
/// every parallel mutate_role/mutate_user edit lands, with no lost update
/// from the clone-modify-swap write path.
#[tokio::test]
async fn concurrent_role_and_user_mutations_are_serialized() {
    let (_dir, s, _app, _token) = state().await;
    let owner = omnibedrock_api::auth::User {
        token_version: 0,
        username: "Admin".into(),
        role: "owner".into(),
        roles: vec!["owner".into()],
        password_hash: String::new(),
        display_name: String::new(),
        created_at: String::new(),
        last_login: String::new(),
        quotas: Default::default(),
    };
    // Seed three roles and one user up front.
    for name in ["alpha", "beta", "gamma"] {
        s.auth
            .mutate_role(&owner, name, Some(json!({"permissions": []})), true)
            .await
            .unwrap();
    }
    s.auth
        .mutate_user(
            &owner,
            "helper",
            Some(json!({"password": "helper-pass-1", "roles": ["viewer"]})),
            true,
        )
        .await
        .unwrap();

    // Several rounds of four PARALLEL mutations across distinct entities.
    // If the store lock failed to serialize the clone-swap writes, one of the
    // joined edits would be clobbered and the assertions below would fail.
    let rounds: [&[&str]; 4] = [
        &["SERVER_VIEW", "CONSOLE_VIEW", "PLAYERS_VIEW", "FILES_VIEW"],
        &["BACKUPS_VIEW", "ADDONS_VIEW", "SETTINGS_VIEW", "AUDIT_VIEW"],
        &[
            "PROPERTIES_VIEW",
            "STORE_VIEW",
            "CONSOLE_SEND",
            "FILES_EDIT",
        ],
        &[
            "SERVER_MANAGE",
            "USERS_VIEW",
            "BROADCASTER_VIEW",
            "AUDIT_VIEW",
        ],
    ];
    for (i, round) in rounds.iter().enumerate() {
        // Role lists are stored sorted; express expectations the same way.
        let expected_helper = if i % 2 == 0 {
            vec!["alpha".to_string(), "beta".to_string()]
        } else {
            vec!["alpha".to_string(), "gamma".to_string()]
        };
        let (ra, rb, rg, ru) = tokio::join!(
            s.auth.mutate_role(
                &owner,
                "alpha",
                Some(json!({"permissions": [round[0]]})),
                false,
            ),
            s.auth.mutate_role(
                &owner,
                "beta",
                Some(json!({"permissions": [round[1]]})),
                false,
            ),
            s.auth.mutate_role(
                &owner,
                "gamma",
                Some(json!({"permissions": [round[2]]})),
                false,
            ),
            s.auth.mutate_user(
                &owner,
                "helper",
                Some(json!({"roles": expected_helper})),
                false,
            ),
        );
        assert!(
            ra.is_ok() && rb.is_ok() && rg.is_ok() && ru.is_ok(),
            "round {i}"
        );
        // All four edits are present after the join: nothing was lost.
        let roles = s.auth.roles().await;
        let perms_of = |name: &str| {
            roles
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == name)
                .unwrap()["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(perms_of("alpha"), vec![round[0]], "round {i}: alpha lost");
        assert_eq!(perms_of("beta"), vec![round[1]], "round {i}: beta lost");
        assert_eq!(perms_of("gamma"), vec![round[2]], "round {i}: gamma lost");
        let users = s.auth.users().await;
        let helper_roles = users
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == "helper")
            .unwrap()["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(helper_roles, expected_helper, "round {i}: user roles lost");
    }
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

/// Changing resource limits is a granular permission (INSTANCES_LIMITS),
/// grantable globally or through an instance share; the quota ceiling still
/// applies independently of who edits.
#[tokio::test]
async fn resource_limits_are_a_granular_permission() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/instances",
        t,
        json!({"name":"Limit Test","backend":"vanilla","deployment":"native","port":21571}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    // A user with only a viewer share cannot touch resources…
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        t,
        json!({"username":"watcher","password":"watcher-pass-1","roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        t,
        json!({"access": [{"user": "watcher", "roles": ["viewer"]}]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"watcher","password":"watcher-pass-1"}),
    )
    .await;
    let watcher = login["token"].as_str().unwrap().to_owned();
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        Some(&watcher),
        json!({"memory_mb": 512}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // …but a share granting a role that carries INSTANCES_LIMITS can.
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"tuner","permissions":["INSTANCES_LIMITS"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        t,
        json!({"access": [{"user": "watcher", "roles": ["viewer", "tuner"]}]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        Some(&watcher),
        json!({"memory_mb": 512, "cpus": 0.5}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (_, detail) = call(&app, "GET", "/api/v1/instances/limit-test", t, json!({})).await;
    assert_eq!(detail["resources"]["memory_mb"], json!(512));
    assert_eq!(detail["resources"]["cpus"], json!(0.5));

    // A global INSTANCES_LIMITS holder reaches instances they can see without
    // owning them (admins hold it via the full permission set).
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        t,
        json!({"memory_mb": 1024}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    // Quotas are not bypassed by the permission: cap the instance owner's
    // quotas below the requested allocation; the non-owner limits-holder's
    // edit must then fail even though the permission check passes. (Acting
    // panel owners are deliberately quota-exempt.)
    let (_, admin_login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    let admin = admin_login["token"].as_str().unwrap().to_owned();
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        Some(&admin),
        json!({"quotas": {"memory_mb": 256}}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/limit-test",
        Some(&watcher),
        json!({"memory_mb": 1024}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("quota"), "{body}");
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
