//! The running-instance handle: what one started server looks like for each
//! deployment (owned child process, docker logs reader, systemd unit), plus
//! teardown of the panel's plumbing (drain), the unified exit watcher, and
//! the graceful-to-forced stop ladder.

use anyhow::{Context, bail};
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::commands::{CommandSink, WriteCommand, dispatch_command, write_command};
use super::spawn::signal_tree;
use super::types::ServerSettings;
use crate::docker;

/// One running instance, however it is hosted: a child process the panel
/// owns (native, a `docker attach` client, or a systemd user service).
pub(super) struct Running {
    pub(super) pid: u32,
    pub(super) sink: CommandSink,
    pub(super) kind: RunKind,
}

pub(super) enum RunKind {
    /// A process the panel supervises directly, with piped stdio. For docker
    /// instances this process is the `docker logs -f` reader: no stdin child
    /// exists (commands go through the FIFO) and its exit means the container
    /// stopped streaming, not that the server is gone.
    Process {
        child: Child,
        stdin: Option<mpsc::Sender<WriteCommand>>,
        writer: Option<JoinHandle<()>>,
        readers: Vec<JoinHandle<()>>,
        /// Docker container name when the process reads a container's logs.
        container: Option<String>,
        /// Reports the moment the container stops running, independent of
        /// the logs reader's own lifetime.
        watch: Option<oneshot::Receiver<String>>,
    },
    /// A `omnibedrock-<id>.service` user unit owned by systemd. The panel
    /// attaches its journal and monitors its state but never parents the
    /// process, so the server survives panel restarts.
    Unit {
        id: String,
        exit: oneshot::Receiver<String>,
        monitor: JoinHandle<()>,
        readers: Vec<JoinHandle<()>>,
    },
}

/// Tear down the panel's own plumbing around a running instance WITHOUT
/// stopping a systemd service: journal/monitor tasks are detached so a unit
/// keeps running when the panel shuts down (that is the point of service
/// hosting). Process-based instances are cleaned up as before.
pub(super) async fn drain(r: Running) {
    let pid = r.pid;
    match r.kind {
        RunKind::Process {
            child,
            writer,
            mut readers,
            ..
        } => {
            // A launcher can exit while its descendants retain the pipes.
            // Clean up its group. For a docker logs reader this only tears
            // down panel plumbing: the container keeps running (its restart
            // policy owns its lifetime).
            signal_tree(pid, true);
            drop(child);
            if let Some(writer) = writer {
                writer.abort();
                let _ = writer.await;
            }
            for mut reader in readers.drain(..) {
                if tokio::time::timeout(Duration::from_secs(30), &mut reader)
                    .await
                    .is_err()
                {
                    reader.abort();
                    let _ = reader.await;
                }
            }
        }
        RunKind::Unit {
            monitor,
            mut readers,
            ..
        } => {
            monitor.abort();
            let _ = monitor.await;
            for reader in readers.drain(..) {
                reader.abort();
                let _ = reader.await;
            }
        }
    }
}
/// Unified "how did it end" future for the supervisor's exit branch.
pub(super) async fn wait_running(r: &mut Running) -> String {
    match &mut r.kind {
        RunKind::Process {
            child,
            container: Some(container),
            watch,
            ..
        } => {
            let container = container.clone();
            let reader = child.wait();
            let watch = watch.as_mut();
            tokio::select! {
                // The logs reader ended: either the container stopped (the
                // stream closes with it) or the daemon restarted the
                // container under its policy. Container state decides which.
                _ = reader => {
                    match docker::state_and_exit(&container).await {
                        Some((status, _)) if status == "running" => "reattach needed".to_string(),
                        Some((_, Some(0))) => "container exited (exit code 0)".to_string(),
                        Some((_, Some(code))) => format!("container exited (exit code {code})"),
                        _ => "container is gone".to_string(),
                    }
                }
                msg = async {
                    match watch {
                        Some(w) => w.await.unwrap_or_else(|_| "container watch ended".into()),
                        None => std::future::pending().await,
                    }
                } => msg,
            }
        }
        RunKind::Process { child, .. } => match child.wait().await {
            // ExitStatus's own Display already reads "exit status: 1"; wrapping
            // it again produced the "exit status exit status: 1" duplication.
            Ok(status) => format!("{status}"),
            Err(e) => format!("wait failed: {e}"),
        },
        RunKind::Unit { exit, .. } => exit.await.unwrap_or_else(|_| "unit ended".into()),
    }
}
pub(super) async fn terminate(
    r: &mut Running,
    settings: &ServerSettings,
    force: bool,
) -> anyhow::Result<()> {
    let stop_budget = Duration::from_secs(settings.stop_timeout);
    let kill_budget = Duration::from_secs(settings.kill_timeout);
    let pid = r.pid;
    let sink = r.sink.clone();
    match &mut r.kind {
        RunKind::Process {
            child,
            stdin,
            container,
            ..
        } => {
            // Docker stop ladder: graceful `stop` through the command FIFO
            // (identical to native), then ALWAYS `docker stop` — it stops a
            // still-running process AND records the manual stop that keeps
            // the restart policy from reviving the container after its own
            // exit (a bare process exit WOULD be revived). `docker kill` is
            // the force shortcut.
            if let Some(container) = container {
                if !force {
                    let _ = dispatch_command(&sink, "stop").await;
                    let _ = tokio::time::timeout(stop_budget, child.wait()).await;
                }
                if force {
                    docker::kill(container).await;
                } else if let Err(e) = docker::stop(container, settings.stop_timeout).await {
                    tracing::warn!(%e, container, "docker stop failed; escalating");
                    docker::kill(container).await;
                }
                // The logs reader exits once the container stops.
                if tokio::time::timeout(kill_budget, child.wait())
                    .await
                    .is_err()
                {
                    docker::kill(container).await;
                    let _ = tokio::time::timeout(kill_budget, child.wait()).await;
                }
                return Ok(());
            }
            if !force {
                let _ = write_command(
                    stdin
                        .as_ref()
                        .context("native process lost its stdin pipe")?,
                    "stop",
                )
                .await;
                if tokio::time::timeout(stop_budget, child.wait())
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                signal_tree(pid, false);
                #[cfg(not(unix))]
                child.start_kill()?;
                if tokio::time::timeout(kill_budget, child.wait())
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            signal_tree(pid, true);
            child.kill().await.context("kill child")?;
            Ok(())
        }
        RunKind::Unit { id, exit, .. } => {
            // Same ladder as owned processes: graceful `stop` command, then
            // systemctl stop (SIGTERM via the unit helper), then SIGKILL.
            if !force {
                let _ = dispatch_command(&sink, "stop").await;
                if tokio::time::timeout(stop_budget, &mut *exit).await.is_ok() {
                    return Ok(());
                }
            }
            let _ = crate::systemd::stop_unit(id).await;
            if tokio::time::timeout(kill_budget, &mut *exit).await.is_ok() {
                return Ok(());
            }
            let _ = crate::systemd::kill_unit(id).await;
            if tokio::time::timeout(kill_budget, &mut *exit).await.is_ok() {
                return Ok(());
            }
            bail!("systemd unit {id} did not stop")
        }
    }
}
