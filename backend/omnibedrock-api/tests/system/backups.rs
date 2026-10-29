//! Backup tests: archive round trip with trash, restore, and dry-run hooks;
//! failed hook commands still resuming saves; traversal-safe world restore;
//! scheduled retention; live save-readiness confirmation; and overlap
//! rejection under request cancellation.
use crate::harness::{call, state, wait_line};
use axum::http::StatusCode;
use omnibedrock_server::backups::BackupRequest;
use omnibedrock_server::server::ServerSettings;
use serde_json::json;
use std::time::Duration;

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
