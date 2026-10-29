//! Server property and file API tests: the properties/files/worlds/addons/
//! audit API surface, and property writes scoped to an X-Instance server
//! directory.
use crate::harness::{call, call_instance, call_text_instance, state};
use axum::http::StatusCode;
use serde_json::json;

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
