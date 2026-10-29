//! Docker deployment tests (self-skipping without a docker daemon):
//! container lifecycle with host networking, the unless-stopped restart
//! policy, and panel-restart adoption; backend switching that preserves
//! instance data; and the broadcaster host-network sidecar.
use crate::harness::{
    FAKE, FAKE_JAVA, call, call_instance, config, login_owner, setup, wait_line, wait_status,
};
use axum::http::StatusCode;
use omnibedrock_api::api::{AppState, router};
use omnibedrock_server::server::{Console, Status};
use serde_json::{Value, json};
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

// ---- Docker backend tests (skipped when no docker daemon is reachable) ----

const GAME_FIXTURE_IMAGE: &str = "omnibedrock-test-game:ci";
const BROADCASTER_FIXTURE_IMAGE: &str = "omnibedrock-test-broadcaster:ci";

async fn docker_available() -> bool {
    tokio::process::Command::new("docker")
        .arg("version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Runtime-only fixture images: the fake server/jar scripts all come from the
/// bind-mounted instance directories; the images only provide the runtime,
/// exactly like the panel's own omnibedrock-* images.
async fn build_fixture_images() {
    let ctx = tempfile::tempdir().unwrap();
    std::fs::write(
        ctx.path().join("Dockerfile"),
        "FROM alpine\nWORKDIR /data\nCMD [\"./fake-server\"]\n",
    )
    .unwrap();
    let out = tokio::process::Command::new("docker")
        .args(["build", "-q", "-t", GAME_FIXTURE_IMAGE])
        .arg(ctx.path())
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "game fixture image build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The panel's sidecar command runs `java -jar <jar>`; provide a fake
    // java that executes -jar targets as shell scripts (the fixture jar IS
    // the FAKE_JAVA script).
    std::fs::write(
        ctx.path().join("Dockerfile"),
        "FROM alpine\nRUN printf '#!/bin/sh\\nwhile [ \"$1\" != \"-jar\" ]; do shift; done\\nshift\\nexec sh \"$@\"\\n' > /usr/local/bin/java && chmod +x /usr/local/bin/java\nWORKDIR /data\n",
    )
    .unwrap();
    let out = tokio::process::Command::new("docker")
        .args(["build", "-q", "-t", BROADCASTER_FIXTURE_IMAGE])
        .arg(ctx.path())
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "broadcaster fixture image build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// (state, network mode, restart policy) for one container; None when the
/// container does not exist or the daemon is unreachable.
async fn docker_facts(container: &str) -> Option<(String, String, String)> {
    let out = tokio::process::Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{.State.Status}}|{{.HostConfig.NetworkMode}}|{{.HostConfig.RestartPolicy.Name}}",
            container,
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = text.split('|');
    Some((
        parts.next().unwrap_or_default().into(),
        parts.next().unwrap_or_default().into(),
        parts.next().unwrap_or_default().into(),
    ))
}

async fn docker_rm(container: &str) {
    let _ = tokio::process::Command::new("docker")
        .args(["rm", "-f", container])
        .output()
        .await;
}

/// Remove every container a previous (failed) test run may have left behind:
/// instance ids are stable slugs, so leftovers would be adopted by the next
/// run instead of started fresh.
async fn clean_test_containers() {
    for name in [
        "omnibedrock-docker-fixture",
        "omnibedrock-switch-fixture",
        "omnibedrock-bc-docker",
        "omnibedrock-bc-docker-broadcaster",
    ] {
        docker_rm(name).await;
    }
}

async fn wait_console_line(console: &Console, needle: &str) {
    let mut changes = console.changed.subscribe();
    let mut after = None;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let batch = console.replay_reliable(after, 512).await.unwrap();
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

/// Seed an instance's server directory with the fake server binary and a
/// world file, mirroring what a real BDS install looks like on disk.
async fn seed_instance_server(dir: &TempDir, id: &str) {
    use std::os::unix::fs::PermissionsExt;
    let server_dir = dir.path().join("data/instances").join(id).join("server");
    tokio::fs::create_dir_all(server_dir.join("worlds/Fixture"))
        .await
        .unwrap();
    tokio::fs::write(server_dir.join("worlds/Fixture/level.dat"), "fixture world")
        .await
        .unwrap();
    // Named bedrock_server: the vanilla container command runs ./bedrock_server
    // and the API's vanilla bootstrap sees the binary and no-ops.
    tokio::fs::write(server_dir.join("bedrock_server"), FAKE)
        .await
        .unwrap();
    tokio::fs::set_permissions(
        server_dir.join("bedrock_server"),
        std::fs::Permissions::from_mode(0o755),
    )
    .await
    .unwrap();
}

/// The docker deployment end to end: detached container from a bind-mounted
/// instance directory, host networking, the unless-stopped restart policy,
/// console + commands through the attach client, panel-restart adoption, and
/// crash revival owned by the daemon's policy.
#[tokio::test]
async fn docker_instance_lifecycle_restart_policy_and_adopt() {
    if !docker_available().await {
        eprintln!("docker unavailable; skipping docker lifecycle test");
        return;
    }
    build_fixture_images().await;
    clean_test_containers().await;
    let (dir, _c) = setup().await;
    let s = AppState::new(config(&dir), CancellationToken::new())
        .await
        .unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&token),
        json!({
            "name": "Docker Fixture",
            "backend": "vanilla",
            "deployment": "docker",
            "port": 21881,
            "docker": {"image": GAME_FIXTURE_IMAGE, "extra_mounts": []}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deployment"], "docker");
    assert_eq!(body["docker"]["image"], GAME_FIXTURE_IMAGE);
    let id = body["id"].as_str().unwrap().to_string();
    let instance = s.instances.get(&id).await.unwrap();
    seed_instance_server(&dir, &id).await;

    // Start: the container runs the bind-mounted script; the console streams
    // through the panel's attach client.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true, "{body}");
    wait_line(&instance.server, "Endstone fixture ready").await;
    wait_status(&instance.server, Status::Running).await;

    let container = format!("omnibedrock-{id}");
    let (state, network, policy) = docker_facts(&container).await.unwrap();
    assert_eq!(state, "running");
    assert_eq!(
        network, "host",
        "Linux docker instances use host networking"
    );
    assert_eq!(policy, "unless-stopped");

    // Console commands reach the container's stdin through the attach client.
    let (status, body) = call_instance(
        &app,
        "POST",
        "/api/v1/console/command",
        Some(&token),
        Some(&id),
        json!({"command": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_line(&instance.server, "There are 2/10 players").await;

    // Panel restart: drop the whole panel and boot a fresh one from the same
    // data directory. The container keeps running (docker instances detach)
    // and the new boot adopts it instead of reporting it stopped.
    let instance_config = instance.config.clone();
    drop(app);
    drop(s);
    drop(instance);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        docker_facts(&container).await.unwrap().0,
        "running",
        "container survives panel shutdown"
    );
    let s2 = AppState::new((*instance_config).clone(), CancellationToken::new())
        .await
        .unwrap();
    let app2 = router(s2.clone()).unwrap();
    // The owner user already exists on disk; just sign in again.
    let (_, login2) = call(
        &app2,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"test-owner","password":"test-password"}),
    )
    .await;
    assert!(login2["token"].is_string(), "{login2}");
    let _token2 = login2["token"].as_str().unwrap().to_owned();
    let instance2 = s2.instances.get(&id).await.unwrap();
    wait_status(&instance2.server, Status::Running).await;
    let snapshot = instance2.server.snapshot();
    assert_eq!(snapshot.status, Status::Running);
    assert_eq!(
        snapshot.deployment,
        omnibedrock_server::instances::Deployment::Docker
    );

    // Crash: the daemon's restart policy revives the container and the panel
    // re-attaches; the panel itself never starts a stopped container.
    instance2.server.send_command("crash").await.unwrap();
    let mut state_rx = instance2.server.state.clone();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if state_rx.borrow_and_update().status == Status::Running {
                return;
            }
            state_rx.changed().await.unwrap();
        }
    })
    .await
    .expect("container revived and reattached");
    let mut revived = false;
    for _ in 0..40 {
        if instance2.server.send_command("list").await.is_ok() {
            revived = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(revived, "commands never reached the revived container");

    // Panel stop: `docker stop` records the manual stop the policy respects.
    let (status, body) = call(
        &app2,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&_token2),
        json!({"action": "stop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_status(&instance2.server, Status::Stopped).await;
    let (state, _, policy) = docker_facts(&container).await.unwrap();
    assert_eq!(state, "exited");
    assert_eq!(policy, "unless-stopped", "policy survives docker stop");

    docker_rm(&container).await;
}

/// Switching backends only relaunches: the portable instance directory (worlds,
/// packs, properties) is untouched in both directions.
#[tokio::test]
async fn docker_switch_back_and_forth_preserves_instance_data() {
    if !docker_available().await {
        eprintln!("docker unavailable; skipping switch test");
        return;
    }
    build_fixture_images().await;
    clean_test_containers().await;
    let (dir, _c) = setup().await;
    let s = AppState::new(config(&dir), CancellationToken::new())
        .await
        .unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&token),
        json!({
            "name": "Switch Fixture",
            "backend": "vanilla",
            "deployment": "native",
            "port": 21883
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    let instance = s.instances.get(&id).await.unwrap();
    seed_instance_server(&dir, &id).await;
    let world = dir
        .path()
        .join("data/instances")
        .join(&id)
        .join("server/worlds/Fixture/level.dat");

    // Native round trip first.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_line(&instance.server, "Endstone fixture ready").await;
    wait_status(&instance.server, Status::Running).await;
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "stop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_status(&instance.server, Status::Stopped).await;

    // Native → docker while stopped.
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/v1/instances/{id}"),
        Some(&token),
        json!({"deployment": "docker", "docker": {"image": GAME_FIXTURE_IMAGE}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deployment"], "docker");
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_line(&instance.server, "Endstone fixture ready").await;
    wait_status(&instance.server, Status::Running).await;
    let container = format!("omnibedrock-{id}");
    assert_eq!(docker_facts(&container).await.unwrap().0, "running");
    assert_eq!(
        tokio::fs::read_to_string(&world).await.unwrap(),
        "fixture world"
    );

    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "stop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_status(&instance.server, Status::Stopped).await;

    // Docker → native: same data, plain child process again.
    let (status, body) = call(
        &app,
        "PUT",
        &format!("/api/v1/instances/{id}"),
        Some(&token),
        json!({"deployment": "native", "docker": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["docker"], Value::Null);
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/v1/instances/{id}/action"),
        Some(&token),
        json!({"action": "start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_line(&instance.server, "Endstone fixture ready").await;
    wait_status(&instance.server, Status::Running).await;
    assert_ne!(
        docker_facts(&container).await.map(|(s, _, _)| s).as_deref(),
        Some("running"),
        "native instance must not have a running container"
    );
    assert_eq!(
        tokio::fs::read_to_string(&world).await.unwrap(),
        "fixture world"
    );

    let _ = instance.server.action("stop").await;
    docker_rm(&container).await;
}

/// Under the docker backend the broadcaster runs as a second container on the
/// same host network, with the panel owning its lifecycle.
#[tokio::test]
async fn docker_broadcaster_runs_as_host_network_sidecar() {
    if !docker_available().await {
        eprintln!("docker unavailable; skipping broadcaster sidecar test");
        return;
    }
    build_fixture_images().await;
    clean_test_containers().await;
    let (dir, _c) = setup().await;
    let mut c = config(&dir);
    // The sidecar image comes from the panel config, not the instance record.
    c.docker_image_broadcaster = BROADCASTER_FIXTURE_IMAGE.into();
    let s = AppState::new(c, CancellationToken::new()).await.unwrap();
    let app = router(s.clone()).unwrap();
    let token = login_owner(&app, &s).await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/instances",
        Some(&token),
        json!({
            "name": "Bc Docker",
            "backend": "vanilla",
            "deployment": "docker",
            "port": 21885,
            "docker": {"image": GAME_FIXTURE_IMAGE}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    let instance = s.instances.get(&id).await.unwrap();

    // The fake jar lives in the instance's broadcaster dir (shared-jar
    // resolution) and is mounted read-only into the sidecar.
    let bc_dir = dir
        .path()
        .join("data/instances")
        .join(&id)
        .join("broadcaster");
    tokio::fs::create_dir_all(&bc_dir).await.unwrap();
    tokio::fs::write(bc_dir.join("MCXboxBroadcastStandalone.jar"), FAKE_JAVA)
        .await
        .unwrap();

    let (status, body) = call_instance(
        &app,
        "POST",
        "/api/v1/broadcaster/action",
        Some(&token),
        Some(&id),
        json!({"action": "start"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let bc_container = format!("omnibedrock-{id}-broadcaster");
    wait_console_line(&instance.broadcaster.console, "Broadcaster fixture ready").await;
    let (state, network, policy) = docker_facts(&bc_container).await.unwrap();
    assert_eq!(state, "running");
    assert_eq!(
        network, "host",
        "sidecar shares the host network like native"
    );
    assert_ne!(
        policy, "unless-stopped",
        "the sidecar has no restart policy"
    );

    // Commands reach the sidecar through the attach client.
    let (status, body) = call_instance(
        &app,
        "POST",
        "/api/v1/broadcaster/command",
        Some(&token),
        Some(&id),
        json!({"command": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_console_line(&instance.broadcaster.console, "cmd=hello").await;

    let (status, body) = call_instance(
        &app,
        "POST",
        "/api/v1/broadcaster/action",
        Some(&token),
        Some(&id),
        json!({"action": "stop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if instance.broadcaster.snapshot().status
                == omnibedrock_server::broadcaster::BcStatus::Stopped
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap();
    let (state, _, _) = docker_facts(&bc_container).await.unwrap();
    assert_eq!(state, "exited");

    docker_rm(&bc_container).await;
    docker_rm(&format!("omnibedrock-{id}")).await;
}
