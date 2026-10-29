//! Vanilla backend (BDS) tests: hermetic backend switching round trip,
//! release archive application that preserves user data, and registry and
//! channel metadata parsing.
use crate::harness::{FAKE, call, state, wait_line};
use axum::http::StatusCode;
use omnibedrock_server::server::Status;
use serde_json::json;

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
