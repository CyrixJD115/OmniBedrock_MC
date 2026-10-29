//! The supervisor event loop that owns the broadcaster process: one handler
//! each for Start / Stop / Restart / TakeOver / Command / Shutdown, foreign-
//! broadcaster replacement on takeover and stop, pidfile bookkeeping so a
//! restarted panel can recognize its own orphans, and the shared start_owned
//! launch path used by Start, Restart, and TakeOver.

use anyhow::bail;
use omnibedrock_core::config::Config;
use omnibedrock_core::files::atomic_write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::config::announce_relay;
use super::external::{find_external, kill_external};
use super::helpers::resolve_jar;
use super::process::{drain, launch, launch_docker, terminate, write_fifo};
use super::types::{BcSnapshot, BcStatus, Control, DockerSpec, Running};
use crate::server::Console;

/// The panel writes its child's PID here so a restarted panel can tell
/// orphaned sidecars ("adopt/replace me") from manually started ones.
fn pidfile_path(config: &Config) -> PathBuf {
    config.broadcaster_dir.join("panel-broadcaster.pid")
}

async fn write_pidfile(config: &Config, pid: u32) {
    let data = format!("{pid}\n");
    if let Err(e) = atomic_write(&pidfile_path(config), data.as_bytes()).await {
        tracing::warn!(%e, "broadcaster pidfile write failed");
    }
}

async fn clear_pidfile(config: &Config) {
    let _ = tokio::fs::remove_file(pidfile_path(config)).await;
}

pub(super) async fn supervise(
    config: Arc<Config>,
    jar_home: Option<PathBuf>,
    mut control: mpsc::Receiver<Control>,
    status: watch::Sender<BcSnapshot>,
    console: Console,
    shutdown: CancellationToken,
    docker_rx: watch::Receiver<Option<DockerSpec>>,
) {
    let mut running: Option<Running> = None;
    loop {
        enum Wake {
            Control(Option<Control>),
            Exit(std::io::Result<std::process::ExitStatus>),
            Shutdown,
        }
        let wake = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Wake::Shutdown,
            c = control.recv() => Wake::Control(c),
            exit = async { match running.as_mut() { Some(r) => r.child.wait().await, None => std::future::pending().await } } => Wake::Exit(exit),
        };
        match wake {
            Wake::Shutdown | Wake::Control(None) => {
                if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    if let Err(e) = terminate(&mut r).await {
                        tracing::error!(%e, "broadcaster shutdown failed");
                    }
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                }
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let _ = terminate(&mut r).await;
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                }
                let _ = reply.send(Ok("Broadcaster stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Start(reply))) => {
                let result = if running.is_some() {
                    Ok("Broadcaster is already running".into())
                } else {
                    match start_owned(&config, &jar_home, &console, &docker_rx, &status).await {
                        Ok(r) => {
                            let pid = r.pid;
                            running = Some(r);
                            Ok(format!("Broadcaster started (PID: {pid})"))
                        }
                        Err(e) => Err(e),
                    }
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::TakeOver(reply))) => {
                let result = async {
                    if running.is_some() {
                        anyhow::bail!("Broadcaster is already running");
                    }
                    let docker_spec = docker_rx.borrow().clone();
                    if docker_spec.is_none() {
                        // Exactly one broadcaster per Xbox account: replace a
                        // manually started (or panel-orphaned) java before
                        // spawning our own.
                        let dir = &config.broadcaster_dir;
                        if let Some(ext) = find_external(dir, None).await {
                            let origin = if ext.orphaned {
                                "left over from a previous panel run"
                            } else {
                                "started outside the panel"
                            };
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] Stopping external broadcaster (PID {}, {}) \
                                         before starting a panel-managed one.",
                                        ext.pid, origin
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                            kill_external(&ext, dir).await?;
                        }
                    }
                    let r = start_owned(&config, &jar_home, &console, &docker_rx, &status).await?;
                    let pid = r.pid;
                    running = Some(r);
                    Ok(format!("Broadcaster taken over (PID: {pid})"))
                }
                .await;
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Stop(reply))) => {
                let result = if let Some(mut r) = running.take() {
                    status.send_modify(|s| s.status = BcStatus::Stopping);
                    let result = terminate(&mut r).await;
                    drain(r).await;
                    mark_stopped(&status, &config).await;
                    result.map(|_| "Broadcaster stopped".into())
                } else {
                    // Nothing panel-owned: a foreign broadcaster for this
                    // instance can still be stopped from here so the UI's
                    // Stop button works for externally started javas too.
                    async {
                        let docker_spec = docker_rx.borrow().clone();
                        if docker_spec.is_none() {
                            let dir = &config.broadcaster_dir;
                            if let Some(ext) = find_external(dir, None).await {
                                console
                                    .publish(
                                        format!(
                                            "[LOCAL] Stopping external broadcaster (PID {})...",
                                            ext.pid
                                        ),
                                        "panel",
                                        false,
                                    )
                                    .await;
                                kill_external(&ext, dir).await?;
                                return Ok("External broadcaster stopped".into());
                            }
                        }
                        Ok("Broadcaster is not running".into())
                    }
                    .await
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Restart(reply))) => {
                let result = async {
                    if let Some(mut r) = running.take() {
                        status.send_modify(|s| s.status = BcStatus::Stopping);
                        let stopped = terminate(&mut r).await;
                        drain(r).await;
                        mark_stopped(&status, &config).await;
                        stopped?;
                    }
                    let r = start_owned(&config, &jar_home, &console, &docker_rx, &status).await?;
                    let pid = r.pid;
                    running = Some(r);
                    Ok(format!("Broadcaster restarted (PID: {pid})"))
                }
                .await;
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Command(text, reply))) => {
                let result = match running.as_ref() {
                    Some(r) => match &r.stdin {
                        Some(stdin) => stdin
                            .send(text)
                            .await
                            .map(|_| ())
                            .map_err(|e| anyhow::anyhow!("stdin closed: {e}")),
                        None => {
                            // Sidecar: write the command into the FIFO the
                            // container's java process reads its stdin from.
                            match &r.fifo {
                                Some(fifo) => write_fifo(fifo, &text).await,
                                None => Err(anyhow::anyhow!("sidecar has no command FIFO")),
                            }
                        }
                    },
                    None => Err(anyhow::anyhow!("Broadcaster is not running")),
                };
                let _ = reply.send(result);
            }
            Wake::Exit(exit) => {
                if let Some(r) = running.take() {
                    drain(r).await;
                }
                clear_pidfile(&config).await;
                status.send_modify(|s| {
                    s.status = BcStatus::Crashed;
                    s.pid = None;
                    s.started_at = None;
                });
                console
                    .publish(
                        format!("[LOCAL] Broadcaster exited unexpectedly ({exit:?})"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    }
}

/// Reset the snapshot to Stopped and retire the pidfile: nothing panel-owned
/// is running anymore.
async fn mark_stopped(status: &watch::Sender<BcSnapshot>, config: &Config) {
    status.send_modify(|s| {
        s.status = BcStatus::Stopped;
        s.pid = None;
        s.started_at = None;
    });
    clear_pidfile(config).await;
}

/// The shared launch path for Start / Restart / TakeOver: docker preflight,
/// duplicate guard against foreign broadcasters, spawn, pidfile, and the
/// relay-target boot banner.
async fn start_owned(
    config: &Arc<Config>,
    jar_home: &Option<PathBuf>,
    console: &Console,
    docker_rx: &watch::Receiver<Option<DockerSpec>>,
    status: &watch::Sender<BcSnapshot>,
) -> anyhow::Result<Running> {
    let docker_spec = docker_rx.borrow().clone();
    if docker_spec.is_none() {
        // Two javas answering one Xbox session race each other and the loser
        // is whoever holds the stale config — refuse to spawn a second one.
        let dir = &config.broadcaster_dir;
        if let Some(ext) = find_external(dir, None).await {
            let origin = if ext.orphaned {
                "left over from a previous panel run"
            } else {
                "started outside the panel"
            };
            bail!(
                "a broadcaster is already running for this instance (PID {}, {origin}); \
                 stop it or use Take over before starting a panel-managed one",
                ext.pid
            );
        }
    }
    let preflight = match &docker_spec {
        Some(spec) => {
            launch_docker(
                spec,
                &resolve_jar(config, jar_home.as_deref()),
                config,
                console,
            )
            .await
        }
        None => Ok(()),
    };
    let r = match preflight {
        Err(e) => return Err(e),
        Ok(()) => {
            launch(
                &resolve_jar(config, jar_home.as_deref()),
                config,
                console,
                docker_spec.as_ref(),
            )
            .await?
        }
    };
    status.send_modify(|s| {
        s.status = BcStatus::Running;
        s.pid = Some(r.pid);
        s.started_at = Some(Instant::now());
    });
    // Only native children get a pidfile: the container sidecar has no host
    // java PID to record and duplicates are reattached, not spawned.
    if docker_spec.is_none() {
        write_pidfile(config, r.pid).await;
    }
    announce_relay(config, console).await;
    Ok(r)
}
