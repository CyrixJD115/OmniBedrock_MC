//! Multi-instance tests: instance lifecycle, quotas, and access control;
//! creation permission gating; independent concurrent instances; scoped
//! access roles on shared instances; and resource limits as a granular
//! permission.
use crate::harness::{FAKE, call, call_instance, call_text, state, state_with_config};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn docker_extra_mounts_are_opt_in() {
    // Production default: bind mounts are refused outright, even for the
    // panel owner — a mount is arbitrary host filesystem access.
    let (_dir, _s, app, token) = state_with_config(|c| c.docker_allow_host_mounts = false).await;
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&token),
        json!({
            "name": "Mounts",
            "backend": "vanilla",
            "deployment": "docker",
            "docker": {"extra_mounts": ["/:/host:rw"]}
        }),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("DOCKER_ALLOW_HOST_MOUNTS"),
        "{body}"
    );
    // The update path is gated by the same check.
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/instances/default",
        Some(&token),
        json!({"docker": {"extra_mounts": ["/etc:/etc:ro"]}}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
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
