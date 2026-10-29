//! Broadcaster process plumbing: build the java (or `docker logs`) command,
//! spawn the child with kill_on_drop semantics, route commands through a
//! stdin writer task or the sidecar FIFO, and tear the process group down
//! with escalating SIGTERM/SIGKILL. Also probes the configured Java runtime.

use crate::server::{Console, read_pipe};
use anyhow::{Context, bail};
use omnibedrock_core::config::Config;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::consts::{JAR_NAME, STOP_TIMEOUT};
use super::types::{DockerSpec, Running};

fn java_command(config: &Config, jar: &std::path::Path) -> Command {
    let mut cmd = Command::new(
        config
            .java_bin
            .as_deref()
            .unwrap_or(std::path::Path::new("java")),
    );
    cmd.arg("-jar")
        .arg(jar)
        .current_dir(&config.broadcaster_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

pub(super) async fn launch(
    jar: &std::path::Path,
    config: &Config,
    console: &Console,
    docker: Option<&DockerSpec>,
) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.broadcaster_dir).await?;
    if !jar.exists() {
        bail!(
            "broadcaster jar not found at {}; place {} there or set BROADCASTER_JAR",
            jar.display(),
            JAR_NAME
        );
    }
    let container = docker.map(|spec| spec.container.clone());
    let fifo = docker.map(|_| crate::docker::host_stdin_fifo(&config.broadcaster_dir));
    let (mut cmd, with_stdin) = match docker {
        // The sidecar's output streams through `docker logs -f`; the child
        // has no stdin of its own.
        Some(spec) => {
            let argv = crate::docker::logs_argv(&spec.container, 200);
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]);
            (cmd, false)
        }
        None => (java_command(config, jar), true),
    };
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if with_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().context(if container.is_some() {
        "failed to read the broadcaster container logs"
    } else {
        "java not found; install Java or set JAVA_BIN"
    })?;
    let pid = child.id().context("child has no PID")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let (tx, writer) = if with_stdin {
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let (tx, mut rx) = mpsc::channel::<String>(config.command_queue_capacity);
        let writer = tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if stdin.write_all(text.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });
        (Some(tx), Some(writer))
    } else {
        (None, None)
    };
    let readers = vec![
        tokio::spawn(read_pipe(
            stdout,
            console.clone(),
            "stdout",
            config.console_max_line_bytes,
        )),
        tokio::spawn(read_pipe(
            stderr,
            console.clone(),
            "stderr",
            config.console_max_line_bytes,
        )),
    ];
    tracing::info!(pid, container = ?container, "broadcaster started");
    Ok(Running {
        child,
        pid,
        stdin: tx,
        writer,
        readers,
        container,
        fifo,
    })
}

/// Run the broadcaster as a sidecar container on the host network so its
/// ICE/UDP behavior matches the native setup (Linux only; see
/// `instances::broadcaster_docker_spec`). The broadcaster directory is
/// mounted at /data (config.yml, token cache, screenshots) and the shared
/// jar read-only, so the container itself stays stateless.
pub(super) async fn launch_docker(
    spec: &DockerSpec,
    jar: &std::path::Path,
    config: &Config,
    console: &Console,
) -> anyhow::Result<()> {
    crate::docker::ensure_image(
        config,
        &spec.image,
        crate::docker::DOCKERFILE_BROADCASTER,
        console,
    )
    .await?;
    let jar_text = jar.to_string_lossy();
    anyhow::ensure!(
        !jar_text.contains(':'),
        "broadcaster jar path {jar_text} contains ':' and cannot be bind-mounted"
    );
    match crate::docker::state(&spec.container).await.as_deref() {
        Some("running") => {
            console
                .publish(
                    format!(
                        "[LOCAL] Broadcaster container {} is already running; reattaching.",
                        spec.container
                    ),
                    "panel",
                    false,
                )
                .await;
        }
        _ => {
            crate::docker::rm(&spec.container).await;
            // Commands reach the sidecar through a FIFO inside the mounted
            // broadcaster directory (same mechanism as the systemd backend).
            let fifo = crate::docker::host_stdin_fifo(&config.broadcaster_dir);
            crate::systemd::ensure_fifo(&fifo).await?;
            let mut args: Vec<std::ffi::OsString> = vec![
                "docker".into(),
                "run".into(),
                "-d".into(),
                "--name".into(),
                spec.container.clone().into(),
            ];
            if cfg!(target_os = "linux") {
                // Host networking keeps ICE/UDP behavior identical to the
                // native broadcaster process.
                args.push("--network".into());
                args.push("host".into());
            }
            #[cfg(unix)]
            {
                let uid = nix::unistd::getuid();
                let gid = nix::unistd::getgid();
                args.push("--user".into());
                args.push(format!("{}:{}", uid.as_raw(), gid.as_raw()).into());
            }
            args.push("-v".into());
            args.push(
                format!(
                    "{}:/data",
                    config.broadcaster_dir.to_string_lossy().replace('\\', "/")
                )
                .into(),
            );
            args.push("-v".into());
            args.push(format!("{jar_text}:/data/{JAR_NAME}:ro").into());
            args.push("-w".into());
            args.push("/data".into());
            args.push(spec.image.clone().into());
            args.extend(crate::docker::command_override(&[
                "java".into(),
                "-jar".into(),
                format!("/data/{JAR_NAME}").into(),
            ]));
            let output = tokio::time::timeout(
                Duration::from_secs(120),
                Command::new(&args[0])
                    .args(&args[1..])
                    .stdin(Stdio::null())
                    .output(),
            )
            .await
            .context("docker run timed out")?
            .context("failed to run docker")?;
            if !output.status.success() {
                bail!(
                    "docker run for the broadcaster failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            console
                .publish(
                    format!(
                        "[LOCAL] Broadcaster container {} started (host network).",
                        spec.container
                    ),
                    "panel",
                    false,
                )
                .await;
        }
    }
    Ok(())
}

fn signal_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let signal = if force {
            Signal::SIGKILL
        } else {
            Signal::SIGTERM
        };
        if let Err(e) = killpg(Pid::from_raw(pid as i32), signal)
            && e != nix::errno::Errno::ESRCH
        {
            tracing::warn!(%e, "broadcaster group signal failed");
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}

pub(super) async fn terminate(running: &mut Running) -> anyhow::Result<()> {
    if let Some(container) = running.container.clone() {
        // The sidecar has no restart policy, but the daemon owns the
        // process: graceful `stop` through the command FIFO, then `docker
        // stop` (SIGTERM), then `docker kill`.
        let Some(fifo) = running.fifo.clone() else {
            return Ok(());
        };
        if let Err(e) = write_fifo(&fifo, "stop").await {
            tracing::debug!(%e, "broadcaster fifo stop failed; escalating");
        }
        if tokio::time::timeout(Duration::from_secs(3), running.child.wait())
            .await
            .is_ok()
        {
            return Ok(());
        }
        if let Err(e) = crate::docker::stop(&container, 5).await {
            tracing::warn!(%e, container, "broadcaster docker stop failed; escalating");
            crate::docker::kill(&container).await;
        }
        if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
            .await
            .is_err()
        {
            crate::docker::kill(&container).await;
            let _ = tokio::time::timeout(STOP_TIMEOUT, running.child.wait()).await;
        }
        return Ok(());
    }
    let _ = running
        .stdin
        .as_ref()
        .context("broadcaster lost its stdin")?
        .send("stop".into())
        .await;
    if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    signal_tree(running.pid, false);
    #[cfg(not(unix))]
    running.child.start_kill()?;
    if tokio::time::timeout(STOP_TIMEOUT, running.child.wait())
        .await
        .is_ok()
    {
        return Ok(());
    }
    signal_tree(running.pid, true);
    running.child.kill().await.context("kill broadcaster")
}

pub(super) async fn drain(mut running: Running) {
    // For a logs reader this only tears down panel plumbing; the
    // container's lifetime belongs to the daemon.
    signal_tree(running.pid, true);
    if let Some(writer) = running.writer.take() {
        writer.abort();
        let _ = writer.await;
    }
    for mut reader in running.readers.drain(..) {
        if tokio::time::timeout(Duration::from_secs(10), &mut reader)
            .await
            .is_err()
        {
            reader.abort();
            let _ = reader.await;
        }
    }
}

/// Write one command line into a FIFO, opening read+write so the call never
/// blocks waiting for a reader.
pub(super) async fn write_fifo(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut fifo = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .await?;
        fifo.write_all(text.as_bytes()).await?;
        fifo.write_all(b"\n").await?;
        fifo.flush().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("FIFO command timed out")?
}

/// Probe the configured Java runtime; returns the first `java -version` line.
pub async fn detect_java(config: &Config) -> Option<String> {
    let mut cmd = Command::new(
        config
            .java_bin
            .as_deref()
            .unwrap_or(std::path::Path::new("java")),
    );
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().next().unwrap_or_default().trim();
    (!line.is_empty()).then(|| line.to_string())
}
