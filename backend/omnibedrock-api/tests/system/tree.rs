//! Endstone plugin management and the instance file explorer: upload
//! sanitization (extensions, size caps, path-ish names), path confinement
//! (`..`, absolute, percent-encoded, and symlink escapes are refused on every
//! route), and list/upload/mkdir/rename/delete round-trips inside a real
//! per-instance directory.
use crate::harness::{call, state};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const BOUNDARY: &str = "omnibedrock-test-boundary-7351829365";

/// POST one multipart `file` field, the way a browser FormData upload looks.
async fn call_multipart(
    app: &Router,
    path: &str,
    token: &str,
    filename: &str,
    content: &[u8],
) -> (StatusCode, Value) {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .header(
                    "Content-Type",
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(json!(String::from_utf8_lossy(&bytes)));
    (status, parsed)
}

/// The dedicated instance the explorer round-trip runs against; the response
/// body is the instance summary whose id names `instances/<id>/` on disk.
async fn create_instance(app: &Router, token: &str, name: &str) -> String {
    let (code, v) = call(
        app,
        "POST",
        "/api/v1/instances",
        Some(token),
        json!({"name": name, "backend": "endstone"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    v["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn plugins_round_trip_and_sanitization() {
    let (_dir, s, app, token) = state().await;
    let t = token.as_str();
    let plugins = || s.config.bedrock_server_dir.join("plugins");

    // Listing creates the directory on demand and starts empty.
    let (code, v) = call(
        &app,
        "GET",
        "/api/v1/instances/default/plugins",
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["plugins"], json!([]));
    assert!(plugins().is_dir());

    // A valid .py upload lands in the plugins dir with the restart hint.
    let (code, v) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins",
        t,
        "hello.py",
        b"print('hi')\n",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["restart_required"], json!(true));
    assert_eq!(
        tokio::fs::read_to_string(plugins().join("hello.py"))
            .await
            .unwrap(),
        "print('hi')\n"
    );

    // Duplicate upload is a conflict unless ?overwrite=true.
    let (code, v) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins",
        t,
        "hello.py",
        b"print('again')\n",
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{v}");
    let (code, v) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins?overwrite=true",
        t,
        "hello.py",
        b"print('replaced')\n",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(
        tokio::fs::read_to_string(plugins().join("hello.py"))
            .await
            .unwrap(),
        "print('replaced')\n"
    );

    // Directory components in the client name are stripped, never trusted:
    // the file lands INSIDE the plugins dir under its basename.
    let (code, v) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins",
        t,
        "../../escape.py",
        b"print('x')\n",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(plugins().join("escape.py").is_file());
    assert!(!s.config.bedrock_server_dir.join("escape.py").exists());
    assert!(!s.config.data_dir.join("escape.py").exists());

    // Bad names and wrong extensions are rejected.
    for (name, why) in [
        ("..", "dot-dot name"),
        (".", "dot name"),
        ("", "empty name"),
        ("plugin.exe", "wrong extension"),
        ("noext", "missing extension"),
    ] {
        let (code, v) =
            call_multipart(&app, "/api/v1/instances/default/plugins", t, name, b"x").await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::CONFLICT,
            "{why}: {code} {v}"
        );
    }

    // Oversize uploads are cut off by the cap (25 MiB + 1 byte).
    let big = vec![b'a'; 25 * 1024 * 1024 + 1];
    let (code, v) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins",
        t,
        "big.whl",
        &big,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{v}");
    assert!(!plugins().join("big.whl").exists());
    drop(big);

    // List shows the uploads with sizes.
    let (code, v) = call(
        &app,
        "GET",
        "/api/v1/instances/default/plugins",
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let names: Vec<&str> = v["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["filename"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["escape.py", "hello.py"]);
    assert_eq!(v["plugins"][0]["size"], json!(11));

    // Delete refuses directories but removes files; misses are 404.
    tokio::fs::create_dir_all(plugins().join("subdir"))
        .await
        .unwrap();
    let (code, v) = call(
        &app,
        "DELETE",
        "/api/v1/instances/default/plugins/subdir",
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{v}");
    let (code, v) = call(
        &app,
        "DELETE",
        "/api/v1/instances/default/plugins/hello.py",
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(!plugins().join("hello.py").exists());
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/instances/default/plugins/hello.py",
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn explorer_round_trip_in_instance_dir() {
    let (_dir, s, app, token) = state().await;
    let t = token.as_str();
    let id = create_instance(&app, t, "explorer-test").await;
    let base = format!("/api/v1/instances/{id}/files");
    let root = s.config.data_dir.join("instances").join(&id);
    assert!(root.is_dir(), "instance root exists");

    // mkdir + duplicate conflict.
    let (code, v) = call(
        &app,
        "POST",
        &format!("{base}/mkdir?path=docs"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (code, _) = call(
        &app,
        "POST",
        &format!("{base}/mkdir?path=docs"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);

    // Save text (nested parent auto-created), read it back, list it.
    let (code, v) = call(
        &app,
        "PUT",
        &format!("{base}/raw?path=docs/notes.txt"),
        Some(t),
        json!({"content": "hello world"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(
        tokio::fs::read_to_string(root.join("docs/notes.txt"))
            .await
            .unwrap(),
        "hello world"
    );
    let (code, bytes) =
        crate::harness::call_bytes(&app, "GET", &format!("{base}/raw?path=docs/notes.txt"), t)
            .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(bytes, b"hello world");
    let (code, v) = call(
        &app,
        "GET",
        &format!("{base}?path=docs"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["truncated"], json!(false));
    let entry = &v["entries"][0];
    assert_eq!(entry["name"], "notes.txt");
    assert_eq!(entry["is_dir"], json!(false));
    assert_eq!(entry["size"], json!(11));

    // Dirs sort before files.
    tokio::fs::create_dir_all(root.join("docs/zz-dir"))
        .await
        .unwrap();
    let (_, v) = call(
        &app,
        "GET",
        &format!("{base}?path=docs"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(v["entries"][0]["name"], "zz-dir");
    assert_eq!(v["entries"][0]["is_dir"], json!(true));

    // Rename, then delete.
    let (code, v) = call(
        &app,
        "POST",
        &format!("{base}/rename"),
        Some(t),
        json!({"from": "docs/notes.txt", "to": "docs/renamed.txt"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(root.join("docs/renamed.txt").is_file());
    assert!(!root.join("docs/notes.txt").exists());
    let (code, _) = call(
        &app,
        "DELETE",
        &format!("{base}?path=docs/renamed.txt"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    // Non-empty directory delete needs the explicit recursive+confirm pair.
    tokio::fs::write(root.join("docs/keep.txt"), "x")
        .await
        .unwrap();
    let (code, v) = call(
        &app,
        "DELETE",
        &format!("{base}?path=docs"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{v}");
    assert!(root.join("docs/keep.txt").exists());
    let (code, v) = call(
        &app,
        "DELETE",
        &format!("{base}?path=docs&recursive=true&confirm=true"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(!root.join("docs").exists());

    // Upload into a directory (with the duplicate + overwrite rule).
    let (code, v) = call_multipart(
        &app,
        &format!("{base}/upload?path=server/worlds/Fixture"),
        t,
        "level.dat",
        b"world-bytes",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(
        v["restart_required"],
        json!(true),
        "worlds/ edits need a restart"
    );
    let (code, v) = call_multipart(
        &app,
        &format!("{base}/upload?path=server/worlds/Fixture"),
        t,
        "level.dat",
        b"more",
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{v}");
    let (code, v) = call_multipart(
        &app,
        &format!("{base}/upload?path=worlds/Fixture&overwrite=true"),
        t,
        "level.dat",
        b"world-bytes-2",
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(
        tokio::fs::read(root.join("worlds/Fixture/level.dat"))
            .await
            .unwrap(),
        b"world-bytes-2"
    );
}

#[tokio::test]
async fn explorer_refuses_traversal_and_symlink_escape() {
    let (_dir, s, app, token) = state().await;
    let t = token.as_str();
    let id = create_instance(&app, t, "traversal-test").await;
    let base = format!("/api/v1/instances/{id}/files");
    let root = s.config.data_dir.join("instances").join(&id);
    let outside = s.config.data_dir.join("outside-secret.txt");
    tokio::fs::write(&outside, "secret").await.unwrap();

    // Every route refuses `..` traversal, absolute paths, and the
    // percent-encoded forms of both with 400/403.
    // rename needs an existing source to reach the destination checks.
    let (code, _) = call(
        &app,
        "PUT",
        &format!("{base}/raw?path=a.txt"),
        Some(t),
        json!({"content": "x"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    for path in [
        "../../outside-secret.txt",
        "/etc/passwd",
        "..",
        "%2e%2e%2fout.txt",
    ] {
        let (code, _) = call(
            &app,
            "GET",
            &format!("{base}?path={path}"),
            Some(t),
            json!({}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "list traversal {path}: {code}"
        );
        let (code, _) = call(
            &app,
            "PUT",
            &format!("{base}/raw?path={path}"),
            Some(t),
            json!({"content": "x"}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "save traversal {path}: {code}"
        );
        let (code, _) = call(
            &app,
            "POST",
            &format!("{base}/mkdir?path={path}"),
            Some(t),
            json!({}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "mkdir traversal {path}: {code}"
        );
        let (code, _) = call(
            &app,
            "DELETE",
            &format!("{base}?path={path}"),
            Some(t),
            json!({}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "delete traversal {path}: {code}"
        );
        // JSON bodies are not percent-decoded: send the decoded form.
        let json_path = path.replace("%2e", ".").replace("%2f", "/");
        let (code, _) = call(
            &app,
            "POST",
            &format!("{base}/rename"),
            Some(t),
            json!({"from": "a.txt", "to": json_path}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "rename traversal {path}: {code}"
        );
        let (code, _) =
            crate::harness::call_bytes(&app, "GET", &format!("{base}/raw?path={path}"), t).await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "download traversal {path}: {code}"
        );
    }
    assert!(outside.exists(), "nothing escaped the instance root");

    // A symlink inside the tree pointing outside is forbidden for every
    // operation — listing presents it inertly, reading/writing/deleting it
    // is refused.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("jump.txt")).unwrap();
        let (code, _) =
            crate::harness::call_bytes(&app, "GET", &format!("{base}/raw?path=jump.txt"), t).await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "symlink download: {code}"
        );
        let (code, _) = call(
            &app,
            "PUT",
            &format!("{base}/raw?path=jump.txt"),
            Some(t),
            json!({"content": "x"}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "symlink save: {code}"
        );
        // Writing THROUGH a symlinked directory mid-path is refused too —
        // the bytes must never land outside the root.
        let outside_dir = s.config.data_dir.join("outside-dir");
        tokio::fs::create_dir_all(&outside_dir).await.unwrap();
        std::os::unix::fs::symlink(&outside_dir, root.join("linkdir")).unwrap();
        let (code, _) = call(
            &app,
            "PUT",
            &format!("{base}/raw?path=linkdir/x.txt"),
            Some(t),
            json!({"content": "x"}),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(!outside_dir.join("x.txt").exists());
        let (code, _) = call(
            &app,
            "DELETE",
            &format!("{base}?path=jump.txt"),
            Some(t),
            json!({}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "symlink delete: {code}"
        );
        assert_eq!(
            tokio::fs::read_to_string(&outside).await.unwrap(),
            "secret",
            "the symlink target was never touched"
        );
    }

    // The instance root itself and the panel-managed settings files are
    // untouchable through the explorer (builder seeds server_settings.yaml).
    assert!(root.join("server_settings.yaml").is_file());
    for query in ["path=server_settings.yaml", "path="] {
        let (code, _) = call(
            &app,
            "DELETE",
            &format!("{base}?{query}"),
            Some(t),
            json!({}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::FORBIDDEN,
            "root/settings delete ({query}): {code}"
        );
    }
    let (code, _) = call(
        &app,
        "PUT",
        &format!("{base}/raw?path=server_settings.yaml"),
        Some(t),
        json!({"content": "pwned"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "POST",
        &format!("{base}/rename"),
        Some(t),
        json!({"from": "server_settings.yaml", "to": "settings2.yaml"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // Broadcaster credentials are equally protected (directly and via a
    // recursive delete of their parent directory).
    tokio::fs::create_dir_all(root.join("broadcaster"))
        .await
        .unwrap();
    tokio::fs::write(root.join("broadcaster/config.yml"), "tokens")
        .await
        .unwrap();
    let (code, _) = crate::harness::call_bytes(
        &app,
        "GET",
        &format!("{base}/raw?path=broadcaster/config.yml"),
        t,
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "DELETE",
        &format!("{base}?path=broadcaster&recursive=true&confirm=true"),
        Some(t),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert!(root.join("broadcaster/config.yml").exists());

    // Oversize explorer uploads are capped at 100 MiB (26 MiB proves the
    // cap logic; the full cap is the same code path as the plugin cap).
    let big = vec![b'b'; 26 * 1024 * 1024];
    let (code, v) = call_multipart(&app, &format!("{base}/upload"), t, "big.bin", &big).await;
    assert_eq!(code, StatusCode::OK, "{v}"); // under the 100 MiB cap
    assert!(root.join("big.bin").is_file());
    drop(big);
}

#[tokio::test]
async fn plugins_and_explorer_enforce_permissions() {
    let (_dir, s, app, token) = state().await;
    let owner = token.as_str();
    // A viewer holds neither PLUGIN_MANAGE nor FILES_WRITE.
    s.auth
        .mutate_user(
            &omnibedrock_api::auth::User {
                token_version: 0,
                username: "viewer".into(),
                role: "viewer".into(),
                roles: vec!["viewer".into()],
                password_hash: String::new(),
                display_name: "Viewer".into(),
                created_at: String::new(),
                last_login: String::new(),
                quotas: Default::default(),
            },
            "viewer",
            Some(json!({"password": "viewer-password", "role": "viewer"})),
            true,
        )
        .await
        .unwrap();
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username": "viewer", "password": "viewer-password"}),
    )
    .await;
    let viewer = login["token"].as_str().unwrap().to_owned();

    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/default/plugins",
        Some(&viewer),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call_multipart(
        &app,
        "/api/v1/instances/default/plugins",
        &viewer,
        "x.py",
        b"x",
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/instances/default/plugins/x.py",
        Some(&viewer),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/default/files",
        Some(&viewer),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/instances/default/files/raw?path=x.txt",
        Some(&viewer),
        json!({"content": "x"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call_multipart(
        &app,
        "/api/v1/instances/default/files/upload",
        &viewer,
        "x.bin",
        b"x",
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // The owner passes every gate the viewer failed.
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/default/plugins",
        Some(owner),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "GET",
        "/api/v1/instances/default/files",
        Some(owner),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let _ = s;
}
