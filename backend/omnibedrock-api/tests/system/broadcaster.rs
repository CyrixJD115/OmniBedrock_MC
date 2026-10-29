//! Broadcaster tests: process lifecycle with device codes, config editing,
//! accounts, and screenshots; the WebSocket status/device-code/console
//! stream; and per-instance broadcaster isolation.
use crate::harness::{
    FAKE_JAVA, call, call_bytes, call_instance, call_text, call_text_instance, config, login_owner,
    setup,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use omnibedrock_api::api::{AppState, router};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

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
