//! Docker plumbing shared by the game-server supervisor and the broadcaster
//! sidecar: container state inspection, the stop/kill ladder, stale container
//! cleanup, automatic image builds from the panel's generated Dockerfiles, and
//! the console bridge (`docker logs -f` reader + command FIFO in the bind
//! mount — `docker attach` is intentionally avoided: several daemon versions
//! stream nothing to non-interactive attach clients, and a client crash would
//! otherwise look like a container exit).
//!
//! Supervision split for the `docker` deployment: the container's own restart
//! policy (`--restart unless-stopped`) owns keeping the server alive across
//! crashes, reboots, and panel restarts; the panel only attaches console and
//! command plumbing, watches container state, and performs start/stop on
//! request. The panel NEVER auto-restarts a stopped container — a container
//! that exited stays exited unless the daemon's policy revives it.

use crate::server::Console;
use anyhow::{Context, bail};
use omnibedrock_core::config::Config;
use std::{process::Stdio, time::Duration};

/// `docker` CLI calls must never hang the supervisor.
const DOCKER_TIMEOUT: Duration = Duration::from_secs(30);

/// Container status of one panel-managed container, or `None` when `docker
/// inspect` fails (daemon down, container removed).
pub async fn state(container: &str) -> Option<String> {
    state_and_exit(container).await.map(|(s, _)| s)
}

/// `(State.Status, State.ExitCode)` for one container. Exit codes are only
/// meaningful once the container has exited.
pub async fn state_and_exit(container: &str) -> Option<(String, Option<i32>)> {
    let output = tokio::time::timeout(
        DOCKER_TIMEOUT,
        tokio::process::Command::new("docker")
            .args([
                "inspect",
                "-f",
                "{{.State.Status}} {{.State.ExitCode}}",
                container,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.split_whitespace();
    let status = parts.next()?.to_string();
    let code = parts.next().and_then(|c| c.parse::<i32>().ok());
    Some((status, code))
}

/// Graceful stop: SIGTERM to the container process, escalating to SIGKILL
/// after `timeout_secs`. Marks the manual stop that `unless-stopped` respects,
/// unlike a process exiting on its own (the policy would restart that).
pub async fn stop(container: &str, timeout_secs: u64) -> anyhow::Result<()> {
    let output = tokio::time::timeout(
        DOCKER_TIMEOUT + Duration::from_secs(timeout_secs),
        tokio::process::Command::new("docker")
            .args(["stop", "-t", &timeout_secs.to_string(), container])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .context("docker stop timed out")?
    .context("failed to run docker stop")?;
    if !output.status.success() {
        bail!(
            "docker stop {} failed: {}",
            container,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Best-effort SIGKILL of a container (the `kill` action / stop escalation).
pub async fn kill(container: &str) {
    let _ = tokio::time::timeout(
        DOCKER_TIMEOUT,
        tokio::process::Command::new("docker")
            .args(["kill", container])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await;
}

/// Best-effort removal of a container before `docker run` reuses its name (a
/// killed panel can leave containers behind, and stopped containers persist so
/// `docker start` could resurrect stale image/mount configuration).
pub async fn rm(container: &str) {
    let _ = tokio::time::timeout(
        DOCKER_TIMEOUT,
        tokio::process::Command::new("docker")
            .args(["rm", "-f", container])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await;
}

/// `docker logs -f` reader argv: streams the container's console output (the
/// full history plus follow mode) into the panel's normal pipe reader. The
/// stream ends when the container stops, which doubles as a stop signal.
pub fn logs_argv(container: &str, tail: u64) -> Vec<std::ffi::OsString> {
    vec![
        "docker".into(),
        "logs".into(),
        "-f".into(),
        "--tail".into(),
        tail.to_string().into(),
        container.into(),
    ]
}

/// Console commands reach the container's main process through a FIFO in the
/// bind-mounted server directory (the same mechanism the systemd backend
/// uses): the container command redirects the server's stdin from
/// `/data/stdin.fifo`, and the panel writes commands to the very same file on
/// the host. `<>` opens the FIFO read-write so the server's stdin never sees
/// EOF between commands.
pub const CONTAINER_STDIN_FIFO: &str = "/data/stdin.fifo";

/// POSIX-quote one word for `sh -c`: wrap in `'…'` and escape embedded
/// quotes the shell-approved way (`'\''`). Quoting every element means a
/// path with spaces or metacharacters survives verbatim instead of reshaping
/// the command — there is no guard to forget in release builds.
fn sh_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The command override appended after the image: wraps the runtime command
/// in `sh -c 'exec … <> <fifo>'` so PID 1 IS the server (signals reach it)
/// and its stdin is the command FIFO.
pub fn command_override(server_argv: &[std::ffi::OsString]) -> Vec<std::ffi::OsString> {
    let mut inner = String::from("exec");
    for arg in server_argv {
        inner.push(' ');
        inner.push_str(&sh_quote(&arg.to_string_lossy()));
    }
    inner.push_str(" <> ");
    inner.push_str(&sh_quote(CONTAINER_STDIN_FIFO));
    vec!["sh".into(), "-c".into(), inner.into()]
}

/// The FIFO path inside the bind-mounted server directory on the host side.
pub fn host_stdin_fifo(server_dir: &std::path::Path) -> std::path::PathBuf {
    server_dir.join("stdin.fifo")
}

/// Runtime-only Dockerfile bodies for the panel's own images, identical to
/// deploy/docker in the repository. Generated under the cache directory so
/// packaged installations (with no checkout around) can still build them.
pub const DOCKERFILE_VANILLA: &str = "\
# Generated by OmniBedrock; runtime libraries for the official BDS binary.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4 libzip4 curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /data
CMD [\"./bedrock_server\"]
";

pub const DOCKERFILE_ENDSTONE: &str = "\
# Generated by OmniBedrock; Endstone runtime on CPython 3.12.
FROM python:3.12-slim
RUN pip install --no-cache-dir endstone
WORKDIR /data
CMD [\"python\", \"-m\", \"endstone\", \"--no-confirm\", \"-s\", \"/data\"]
";

pub const DOCKERFILE_BROADCASTER: &str = "\
# Generated by OmniBedrock; JRE runtime for the MCXboxBroadcast sidecar.
FROM eclipse-temurin:21-jre
WORKDIR /data
CMD [\"java\", \"-jar\", \"/data/MCXboxBroadcastStandalone.jar\"]
";

/// Make sure the configured image exists; when it does not, generate its
/// Dockerfile under the cache directory and build it, streaming progress
/// into the given console. Custom (registry) images are never built —
/// building only ever targets the panel's own `omnibedrock-*` images.
pub async fn ensure_image(
    config: &Config,
    image: &str,
    dockerfile_body: &str,
    console: &Console,
) -> anyhow::Result<()> {
    let inspect = tokio::process::Command::new("docker")
        .args(["image", "inspect", image])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    if inspect.is_ok_and(|s| s.success()) {
        return Ok(());
    }
    if !image.starts_with("omnibedrock-") {
        bail!(
            "image {image} is missing; pull it or build it manually \
             (the panel only auto-builds its own omnibedrock-* images)"
        );
    }
    let safe: String = image
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let context = config.cache_dir.join("docker").join(safe);
    tokio::fs::create_dir_all(&context).await?;
    let dockerfile = context.join("Dockerfile");
    // Only ever (re)generate our own generated context; anything unexpected
    // in that directory is left alone and reported.
    if tokio::fs::try_exists(&dockerfile).await.unwrap_or(false) {
        let existing = tokio::fs::read_to_string(&dockerfile)
            .await
            .unwrap_or_default();
        anyhow::ensure!(
            existing.starts_with("# Generated by OmniBedrock"),
            "refusing to overwrite {}",
            dockerfile.display()
        );
    }
    tokio::fs::write(&dockerfile, dockerfile_body).await?;
    console
        .publish(
            format!("[LOCAL] Building missing image {image}…"),
            "panel",
            false,
        )
        .await;
    let build = tokio::time::timeout(
        Duration::from_secs(600),
        tokio::process::Command::new("docker")
            // No --progress=plain: minimal Docker installs ship the legacy
            // builder, which rejects the flag; it streams to stderr anyway.
            .args(["build"])
            .arg("-t")
            .arg(image)
            .arg(&context)
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .context("docker build timed out (10 min)")?
    .context("failed to run docker build")?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    for line in output.lines().filter(|l| l.contains("ERROR")).take(20) {
        console
            .publish(format!("[LOCAL] {line}"), "panel", false)
            .await;
    }
    if !build.status.success() {
        bail!("docker build for {image} failed (see console for error lines)");
    }
    console
        .publish(format!("[LOCAL] Image {image} built."), "panel", false)
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn sh_quote_survives_the_shell_verbatim() {
        // Round-trip through a real sh: quoted words must come back byte for
        // byte, whatever metacharacters they carry.
        let words = [
            "plain",
            "with space",
            "it's",
            "$HOME",
            "`id`",
            "a\"b",
            "back\\slash",
            "*",
            "$(reboot)",
        ];
        let composed = format!(
            "printf '%s\\n' {}",
            words
                .iter()
                .map(|w| sh_quote(w))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&composed)
            .output()
            .expect("sh available");
        assert!(output.status.success(), "{composed}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let echoed: Vec<&str> = stdout.lines().collect();
        assert_eq!(echoed, words, "composed: {composed}");
    }

    use super::*;

    #[test]
    fn logs_argv_follows_with_tail() {
        let argv = logs_argv("omnibedrock-test", 200)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            argv,
            ["docker", "logs", "-f", "--tail", "200", "omnibedrock-test"]
        );
    }

    #[test]
    fn command_override_wraps_argv_with_the_fifo_redirect() {
        let argv: Vec<String> = command_override(&["./bedrock_server".into()])
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            argv,
            ["sh", "-c", "exec './bedrock_server' <> '/data/stdin.fifo'"]
        );
        let fifo = host_stdin_fifo(std::path::Path::new("/srv/instances/x/server"));
        assert_eq!(
            fifo,
            std::path::PathBuf::from("/srv/instances/x/server/stdin.fifo")
        );
    }

    #[test]
    fn generated_dockerfiles_carry_the_managed_marker() {
        for body in [
            DOCKERFILE_VANILLA,
            DOCKERFILE_ENDSTONE,
            DOCKERFILE_BROADCASTER,
        ] {
            assert!(body.starts_with("# Generated by OmniBedrock"));
            assert!(body.contains("WORKDIR /data"));
        }
    }
}
