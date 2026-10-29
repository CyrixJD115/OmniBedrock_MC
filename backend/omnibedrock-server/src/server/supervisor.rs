//! The supervisor event loop: exclusively owns the single running instance,
//! services control actions (start/stop/kill/restart/adopt/settings/
//! shutdown), reacts to exits, and schedules auto-restart and docker
//! recheck timers. Also persists the console lock state.

use omnibedrock_core::{config::Config, files::atomic_write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::commands::{CommandSink, Control};
use super::console::Console;
use super::docker::attach_running_container;
use super::launch::{launch, wait_ports_free};
use super::runtime::{RunKind, Running, drain, terminate, wait_running};
use super::types::{LaunchContext, ServerPaths, ServerSettings, Snapshot, Status};
use super::unit::{attach_unit, unit_fifo};
use crate::instances::Deployment;

async fn lock_state(state_dir: &std::path::Path, locked: bool) {
    let data = format!(
        "console:\n  state: {}\n  timestamp: {}\n",
        if locked { "locked" } else { "unlocked" },
        chrono::Utc::now().timestamp()
    );
    if let Err(e) = atomic_write(&state_dir.join("console_lock_state.yaml"), data.as_bytes()).await
    {
        tracing::error!(%e, "console lock persistence failed");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn supervise(
    config: Arc<Config>,
    paths: ServerPaths,
    mut controls: mpsc::Receiver<Control>,
    state: watch::Sender<Snapshot>,
    settings: watch::Sender<ServerSettings>,
    launch_tx: watch::Sender<LaunchContext>,
    commands: watch::Sender<Option<CommandSink>>,
    console: Console,
    shutdown: CancellationToken,
) {
    let mut running: Option<Running> = None;
    // Next automatic relaunch: the flag marks a docker RECHECK (attach only,
    // never create/start) vs a native relaunch.
    let mut restart_at: Option<(tokio::time::Instant, bool)> = None;
    lock_state(&paths.state_dir, false).await;
    loop {
        enum Wake {
            Control(Option<Control>),
            /// Human-readable description of how the instance ended.
            Exit(String),
            Restart,
            Shutdown,
        }
        let wake = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Wake::Shutdown,
            c = controls.recv() => Wake::Control(c),
            exit = async { match running.as_mut() { Some(r) => wait_running(r).await, None => std::future::pending().await } } => Wake::Exit(exit),
            _ = async { match restart_at { Some((at, _)) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Wake::Restart,
        };
        match wake {
            Wake::Shutdown | Wake::Control(None) => {
                commands.send_replace(None);
                let options = settings.borrow().clone();
                if let Some(mut r) = running.take() {
                    state.send_modify(|s| s.status = Status::Stopping);
                    if matches!(r.kind, RunKind::Unit { .. }) {
                        // A systemd service outlives the panel on purpose:
                        // detach and leave it running for the user manager.
                        tracing::info!(
                            "detaching unit instance at panel shutdown; service keeps running"
                        );
                    } else if matches!(
                        &r.kind,
                        RunKind::Process {
                            container: Some(_),
                            ..
                        }
                    ) {
                        // A docker container outlives the panel on purpose:
                        // detach and leave it to its restart policy.
                        tracing::info!(
                            "detaching container instance at panel shutdown; restart policy keeps it running"
                        );
                    } else if let Err(e) = terminate(&mut r, &options, false).await {
                        tracing::error!(%e, "shutdown stop failed");
                    }
                    drain(r).await;
                }
                lock_state(&paths.state_dir, false).await;
                break;
            }
            Wake::Control(Some(Control::Shutdown(reply))) => {
                state.send_modify(|s| s.status = Status::Stopping);
                commands.send_replace(None);
                let options = settings.borrow().clone();
                let result = if let Some(mut r) = running.take() {
                    let result = if matches!(r.kind, RunKind::Unit { .. }) {
                        // Explicit supervisor shutdown detaches a systemd
                        // service without stopping it (panel restart).
                        tracing::info!("detaching unit instance; service keeps running");
                        Ok(())
                    } else if matches!(
                        &r.kind,
                        RunKind::Process {
                            container: Some(_),
                            ..
                        }
                    ) {
                        tracing::info!(
                            "detaching container instance; restart policy keeps it running"
                        );
                        Ok(())
                    } else {
                        terminate(&mut r, &options, false).await
                    };
                    drain(r).await;
                    result
                } else {
                    Ok(())
                };
                lock_state(&paths.state_dir, false).await;
                state.send_modify(|s| {
                    s.status = Status::Stopped;
                    s.pid = None;
                    s.started_at = None;
                });
                let _ = reply.send(result.map(|_| "Server stopped".into()));
                break;
            }
            Wake::Control(Some(Control::Adopt(reply))) => {
                // After a panel restart, adopt a unit or container that is
                // still active instead of reporting the instance stopped.
                let launch_ctx = launch_tx.borrow().clone();
                let result = if running.is_some() {
                    Ok("Server is already running".into())
                } else if launch_ctx.deployment == Deployment::Docker {
                    state.send_modify(|s| s.status = Status::Starting);
                    let current = settings.borrow().clone();
                    match attach_running_container(&config, &console, &launch_ctx, &current).await {
                        Ok(r) => {
                            let pid = r.pid;
                            let container =
                                crate::instances::container_name(&launch_ctx.instance_id);
                            commands.send_replace(Some(r.sink.clone()));
                            state.send_modify(|s| {
                                s.status = Status::Running;
                                s.pid = Some(pid);
                                s.started_at = Some(Instant::now());
                                s.crash_count = 0;
                            });
                            running = Some(r);
                            lock_state(&paths.state_dir, true).await;
                            Ok(format!(
                                "Adopted running container {container} (PID: {pid})"
                            ))
                        }
                        Err(_) => {
                            state.send_modify(|s| s.status = Status::Stopped);
                            Ok("nothing to adopt".into())
                        }
                    }
                } else if launch_ctx.deployment != Deployment::Systemd {
                    Ok("nothing to adopt".into())
                } else {
                    state.send_modify(|s| s.status = Status::Starting);
                    let fifo = unit_fifo(&paths);
                    match attach_unit(
                        &launch_ctx.instance_id,
                        &fifo,
                        &console,
                        config.console_max_line_bytes,
                    )
                    .await
                    {
                        Ok(r) => {
                            let pid = r.pid;
                            commands.send_replace(Some(r.sink.clone()));
                            state.send_modify(|s| {
                                s.status = Status::Running;
                                s.pid = Some(pid);
                                s.started_at = Some(Instant::now());
                                s.crash_count = 0;
                            });
                            running = Some(r);
                            lock_state(&paths.state_dir, true).await;
                            Ok(format!("Adopted running unit (PID: {pid})"))
                        }
                        Err(e) => {
                            state.send_modify(|s| s.status = Status::Stopped);
                            Err(e)
                        }
                    }
                };
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Settings(new, reply))) => {
                let result = async {
                    atomic_write(
                        &paths.settings_path,
                        serde_yaml::to_string(&new)?.as_bytes(),
                    )
                    .await?;
                    Ok::<_, anyhow::Error>("Server settings updated".into())
                }
                .await;
                if result.is_ok() {
                    if !new.auto_restart {
                        restart_at = None;
                        state.send_modify(|s| s.crash_count = 0);
                    }
                    state.send_modify(|s| {
                        s.auto_restart = new.auto_restart;
                        s.backend = new.backend;
                    });
                    settings.send_replace(new);
                }
                let _ = reply.send(result);
            }
            Wake::Control(Some(Control::Action(action, reply))) => {
                if !["start", "stop", "kill", "restart"].contains(&action.as_str()) {
                    let _ = reply.send(Err(anyhow::anyhow!("Unknown action: {action}")));
                    continue;
                }
                restart_at = None;
                if action != "start" {
                    state.send_modify(|s| s.status = Status::Stopping);
                    commands.send_replace(None);
                    if let Some(mut r) = running.take() {
                        let options = settings.borrow().clone();
                        let result = terminate(&mut r, &options, action == "kill").await;
                        drain(r).await;
                        if let Err(e) = result {
                            tracing::warn!(%e, "stop required process cleanup");
                        }
                    }
                    state.send_modify(|s| {
                        s.status = Status::Stopped;
                        s.pid = None;
                        s.started_at = None;
                        s.crash_count = 0;
                    });
                    lock_state(&paths.state_dir, false).await;
                }
                let result = if action == "start" || action == "restart" {
                    if running.is_some() {
                        Ok("Server is already running".into())
                    } else {
                        state.send_modify(|s| {
                            s.status = Status::Starting;
                            s.crash_count = 0;
                        });
                        console.reset_stats();
                        let current = settings.borrow().clone();
                        let launch_ctx = launch_tx.borrow().clone();
                        let launched = match wait_ports_free(launch_ctx.port, &console).await {
                            Ok(()) => {
                                launch(&config, &paths, &current, &console, &launch_ctx).await
                            }
                            // Precise reason instead of a bind failure inside
                            // the server process (which would read "crashed").
                            Err(e) => Err(e),
                        };
                        match launched {
                            Ok(r) => {
                                let pid = r.pid;
                                commands.send_replace(Some(r.sink.clone()));
                                state.send_modify(|s| {
                                    s.status = Status::Running;
                                    s.pid = Some(pid);
                                    s.started_at = Some(Instant::now());
                                });
                                running = Some(r);
                                lock_state(&paths.state_dir, true).await;
                                Ok(format!("Server started (PID: {pid})"))
                            }
                            Err(e) => {
                                state.send_modify(|s| s.status = Status::Stopped);
                                console
                                    .publish(format!("[LOCAL] Start failed: {e:#}"), "panel", false)
                                    .await;
                                Err(e)
                            }
                        }
                    }
                } else {
                    Ok(if action == "kill" {
                        "Server killed"
                    } else {
                        "Server stopped"
                    }
                    .into())
                };
                let _ = reply.send(result);
            }
            Wake::Exit(exit) => {
                commands.send_replace(None);
                if let Some(r) = running.take() {
                    drain(r).await;
                }
                let launch_ctx = launch_tx.borrow().clone();
                if launch_ctx.deployment == Deployment::Docker {
                    // Supervision split: the daemon's restart policy owns
                    // revival; the panel only re-attaches and reports.
                    if exit == "reattach needed" {
                        // The container is still running; only the panel's
                        // attach plumbing died. Re-attach without a crash.
                        console
                            .publish(
                                "[LOCAL] Container still running; reattaching console.".into(),
                                "panel",
                                false,
                            )
                            .await;
                        state.send_modify(|s| s.status = Status::Starting);
                        let current = settings.borrow().clone();
                        match attach_running_container(&config, &console, &launch_ctx, &current)
                            .await
                        {
                            Ok(r) => {
                                let pid = r.pid;
                                commands.send_replace(Some(r.sink.clone()));
                                state.send_modify(|s| {
                                    s.status = Status::Running;
                                    s.pid = Some(pid);
                                    s.started_at = Some(Instant::now());
                                });
                                running = Some(r);
                                lock_state(&paths.state_dir, true).await;
                            }
                            Err(e) => {
                                state.send_modify(|s| s.status = Status::Crashed);
                                console
                                    .publish(
                                        format!("[LOCAL] Reattach failed: {e:#}"),
                                        "panel",
                                        false,
                                    )
                                    .await;
                                // The container may be mid-restart or the
                                // daemon briefly gone; keep checking.
                                let delay = settings.borrow().auto_restart_delay.max(1);
                                restart_at = Some((
                                    tokio::time::Instant::now() + Duration::from_secs(delay),
                                    true,
                                ));
                            }
                        }
                    } else {
                        let clean = exit.ends_with("(exit code 0)");
                        state.send_modify(|s| {
                            s.status = if clean {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                            s.pid = None;
                            s.started_at = None;
                            if !clean {
                                s.crash_count += 1;
                            }
                        });
                        lock_state(&paths.state_dir, false).await;
                        let crashes = state.borrow().crash_count;
                        if clean {
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] {exit}; the container stays down unless its \
                                         restart policy revives it."
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                        } else {
                            console
                                .publish(
                                    format!(
                                        "[LOCAL] {exit}, crash #{crashes}; the restart policy owns \
                                         revival and the panel reattaches once it runs again."
                                    ),
                                    "panel",
                                    false,
                                )
                                .await;
                        }
                        // Recheck soon: attach when the daemon brought the
                        // container back, stay hands-off when it did not.
                        let delay = settings.borrow().auto_restart_delay.max(1);
                        restart_at = Some((
                            tokio::time::Instant::now() + Duration::from_secs(delay),
                            true,
                        ));
                    }
                    continue;
                }
                state.send_modify(|s| {
                    s.status = Status::Crashed;
                    s.pid = None;
                    s.started_at = None;
                    s.crash_count += 1;
                });
                lock_state(&paths.state_dir, false).await;
                let options = settings.borrow().clone();
                let crashes = state.borrow().crash_count;
                console
                    .publish(
                        format!("[LOCAL] Server exited unexpectedly ({exit}), crash #{crashes}"),
                        "panel",
                        false,
                    )
                    .await;
                if options.auto_restart && crashes <= options.max_crashes {
                    restart_at = Some((
                        tokio::time::Instant::now()
                            + Duration::from_secs(options.auto_restart_delay),
                        false,
                    ));
                }
            }
            Wake::Restart => {
                let (_at, attach_only) = restart_at
                    .take()
                    .unwrap_or((tokio::time::Instant::now(), false));
                state.send_modify(|s| s.status = Status::Starting);
                console.reset_stats();
                let current = settings.borrow().clone();
                let launch_ctx = launch_tx.borrow().clone();
                let result = if attach_only {
                    // Docker recheck: attach when the restart policy revived
                    // the container, never start or create anything.
                    attach_running_container(&config, &console, &launch_ctx, &current).await
                } else {
                    match wait_ports_free(launch_ctx.port, &console).await {
                        Ok(()) => launch(&config, &paths, &current, &console, &launch_ctx).await,
                        Err(e) => Err(e),
                    }
                };
                match result {
                    Ok(r) => {
                        let pid = r.pid;
                        commands.send_replace(Some(r.sink.clone()));
                        // crash_count is deliberately NOT reset here: the
                        // auto-restart budget counts across relaunches (only
                        // an explicit start/adopt starts a fresh streak).
                        state.send_modify(|s| {
                            s.status = Status::Running;
                            s.pid = Some(pid);
                            s.started_at = Some(Instant::now());
                        });
                        running = Some(r);
                        lock_state(&paths.state_dir, true).await;
                    }
                    Err(e) => {
                        // A recheck failure means the container is genuinely
                        // not running (the policy did not revive it): report
                        // stopped unless this was a real crash.
                        state.send_modify(|s| {
                            s.status = if attach_only && s.crash_count == 0 {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                        });
                        console
                            .publish(
                                format!("[LOCAL] Auto-restart failed: {e:#}"),
                                "panel",
                                false,
                            )
                            .await;
                    }
                }
            }
        }
    }
}
