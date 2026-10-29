//! Addon management tests: .mcpack/.mcaddon upload with icons and duplicate
//! name suffixing, ordering pins, delete cleanup, and traversal or
//! non-archive upload rejection.
use crate::harness::{call, download_token, state};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

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

    // Icon endpoint serves the embedded PNG via a short-lived download token.
    let dt = download_token(&app, &token).await;
    let icon_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/addons/icon?path={}&dt={dt}",
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
