//! Launching one instance for each deployment: argv construction for the
//! Endstone and vanilla backends, the bounded UDP-port wait, and the
//! per-deployment start paths (native child, docker container, systemd user
//! unit).

use anyhow::{Context, bail};
use omnibedrock_core::config::Config;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

use super::console::Console;
use super::docker::{attach_docker, docker_run_args};
use super::runtime::Running;
use super::spawn::spawn_process;
use super::types::{Backend, LaunchContext, ServerPaths, ServerSettings};
use super::unit::{attach_unit, unit_fifo};
use crate::docker;
use crate::instances::Deployment;

fn base_argv_endstone(config: &Config) -> anyhow::Result<Vec<std::ffi::OsString>> {
    let Some(candidate) = crate::endstone::resolve_launch(config) else {
        anyhow::bail!(
            "Endstone is not installed — install the Endstone toolchain from \
             Settings → Server backend, or set ENDSTONE_BIN to an endstone executable"
        );
    };
    Ok(match candidate {
        crate::endstone::EndstoneLaunch::Binary(path) => {
            let mut argv = vec![path.into()];
            argv.push("--no-confirm".into());
            argv.push("-s".into());
            argv.push(config.bedrock_server_dir.clone().into());
            argv
        }
        crate::endstone::EndstoneLaunch::PythonModule(python) => {
            let mut argv: Vec<std::ffi::OsString> =
                vec![python.into(), "-m".into(), "endstone".into()];
            argv.push("--no-confirm".into());
            argv.push("-s".into());
            argv.push(config.bedrock_server_dir.clone().into());
            argv
        }
    })
}
fn vanilla_binary() -> &'static std::path::Path {
    if cfg!(windows) {
        std::path::Path::new("bedrock_server.exe")
    } else {
        std::path::Path::new("bedrock_server")
    }
}
fn base_argv_vanilla(config: &Config) -> anyhow::Result<Vec<std::ffi::OsString>> {
    let binary = config.bedrock_server_dir.join(vanilla_binary());
    if !binary.exists() {
        bail!(
            "vanilla bedrock_server binary not found at {}; install a version from Settings → Backend",
            binary.display()
        );
    }
    Ok(vec![binary.into()])
}

pub(super) async fn launch(
    config: &Config,
    paths: &ServerPaths,
    settings: &ServerSettings,
    console: &Console,
    ctx: &LaunchContext,
) -> anyhow::Result<Running> {
    tokio::fs::create_dir_all(&config.bedrock_server_dir).await?;
    // All runtimes read server.properties from the server dir (Endstone
    // wraps BDS). Native/systemd and host-networked Linux containers pin the
    // instance's registered pair (P v4 / P+1 v6); port-mapped containers
    // (Docker Desktop) pin the container-internal stock pair 19132/19133 and
    // publish both through the host port mappings.
    match ctx.deployment {
        Deployment::Docker if !cfg!(target_os = "linux") => {
            crate::instances::enforce_instance_port(config, 19132).await;
        }
        _ => {
            crate::instances::enforce_instance_port(config, ctx.port).await;
        }
    }
    // Backend-aware NetherNet/port guidance replaces the old blanket
    // "nethernet ignores server-port" warning (wrong since Endstone 0.11.15
    // multiplexes NetherNet onto server-port).
    let endstone_version = if settings.backend == Backend::Endstone {
        crate::endstone::read_marker_version(config).await
    } else {
        None
    };
    crate::instances::advise_transport(
        config,
        console,
        settings.backend,
        endstone_version.as_deref(),
    )
    .await;
    // Heal pack-order versions for worlds imported before the sync existed
    // and surface still-missing pack uuids as one console line instead of
    // letting BDS log per-pack "was not found" noise.
    {
        let worlds = config.bedrock_server_dir.join("worlds");
        let sync = tokio::task::spawn_blocking(move || crate::addons::sync_all_worlds(&worlds))
            .await
            .unwrap_or_default();
        if !sync.is_empty() {
            console
                .publish(
                    format!("World pack sync: {}", sync.join("; ")),
                    "panel",
                    false,
                )
                .await;
        }
    }
    match ctx.deployment {
        Deployment::Native => {
            let base_argv = match settings.backend {
                Backend::Endstone => base_argv_endstone(config)?,
                Backend::Vanilla => base_argv_vanilla(config)?,
            };
            spawn_process(
                config,
                console,
                ctx,
                settings.backend,
                base_argv,
                None,
                true,
                None,
            )
            .await
        }
        Deployment::Docker => {
            // The image provides the runtime only; the BDS binary and every
            // server file come from the bind-mounted instance directory, so
            // the host needs no endstone toolchain for docker instances.
            let container = crate::instances::container_name(&ctx.instance_id);
            let host_network = cfg!(target_os = "linux");
            // The command FIFO must exist before the container's shell opens
            // it (the run command fails loudly otherwise).
            crate::systemd::ensure_fifo(&docker::host_stdin_fifo(&config.bedrock_server_dir))
                .await?;
            let image = ctx.docker.image_for(settings.backend, config);
            let dockerfile = match settings.backend {
                Backend::Vanilla => docker::DOCKERFILE_VANILLA,
                Backend::Endstone => docker::DOCKERFILE_ENDSTONE,
            };
            docker::ensure_image(config, &image, dockerfile, console).await?;
            match docker::state(&container).await.as_deref() {
                Some("running") => {
                    console
                        .publish(
                            format!(
                                "[LOCAL] Container {container} is already running; reattaching."
                            ),
                            "panel",
                            false,
                        )
                        .await;
                    attach_docker(config, console, ctx, settings, &container).await
                }
                _ => {
                    // A stopped container is recreated rather than
                    // `docker start`ed so a changed image or mount
                    // configuration takes effect on the next run.
                    docker::rm(&container).await;
                    let args = docker_run_args(config, settings, ctx, host_network);
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
                            "docker run failed: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        );
                    }
                    console
                        .publish(
                            format!(
                                "[LOCAL] Container {container} started (restart policy: {}).",
                                if settings.auto_restart {
                                    "unless-stopped"
                                } else {
                                    "no"
                                }
                            ),
                            "panel",
                            false,
                        )
                        .await;
                    attach_docker(config, console, ctx, settings, &container).await
                }
            }
        }
        Deployment::Systemd => {
            let base_argv = match settings.backend {
                Backend::Endstone => base_argv_endstone(config)?,
                Backend::Vanilla => base_argv_vanilla(config)?,
            };
            // Real user-service hosting: install the unit, let systemd parent
            // the process, then attach console and monitoring. The unit file
            // is regenerated every launch so moves/updates heal themselves.
            let exe = std::env::current_exe()
                .context("locate the panel executable for the unit ExecStart")?;
            let fifo = unit_fifo(paths);
            crate::systemd::ensure_fifo(&fifo).await?;
            let spec = crate::systemd::UnitSpec {
                instance_id: &ctx.instance_id,
                description: &ctx.instance_name,
                exe: &exe,
                fifo: &fifo,
                working_dir: &config.bedrock_server_dir,
                argv: &base_argv,
                resources: ctx.resources,
                stop_timeout: settings.stop_timeout,
                kill_timeout: settings.kill_timeout,
            };
            crate::systemd::install_unit(&ctx.instance_id, &crate::systemd::render_unit(&spec))
                .await?;
            crate::systemd::reset_failed(&ctx.instance_id).await;
            crate::systemd::start_unit(&ctx.instance_id).await?;
            let mut last_state = String::new();
            for _ in 0..100 {
                if let Some(state) = crate::systemd::active_state(&ctx.instance_id).await {
                    last_state = state;
                    if last_state == "active"
                        && let Ok(running) = attach_unit(
                            &ctx.instance_id,
                            &fifo,
                            console,
                            config.console_max_line_bytes,
                        )
                        .await
                    {
                        // The unit file exists now; sync its boot-time enable
                        // state to the instance record's autostart flag.
                        if let Err(e) =
                            crate::systemd::set_enabled(&ctx.instance_id, ctx.autostart).await
                        {
                            tracing::warn!(%e, "sync unit enable state failed");
                        }
                        tracing::info!(
                            instance = %ctx.instance_id,
                            backend = settings.backend.as_str(),
                            "game server started as user unit {}",
                            crate::systemd::unit_name(&ctx.instance_id)
                        );
                        return Ok(running);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            bail!(
                "systemd unit did not become active (state: {last_state}); \
                 check `journalctl --user -u {}`",
                crate::systemd::unit_name(&ctx.instance_id)
            )
        }
    }
}

/// Wait (bounded) for the instance's UDP port pair to become bindable before
/// launching. Starting into a port the previous process is still releasing
/// makes the fresh server die at bind — and the instance report a false
/// "crashed" even though a manual start seconds later works fine.
pub(super) async fn wait_ports_free(port: u16, console: &Console) -> anyhow::Result<()> {
    const BUDGET: Duration = Duration::from_secs(10);
    let deadline = Instant::now() + BUDGET;
    let mut announced = false;
    loop {
        if crate::instances::port_bindable(port).await
            && crate::instances::port_bindable(port + 1).await
        {
            return Ok(());
        }
        if !announced {
            console
                .publish(
                    format!(
                        "[LOCAL] Waiting for ports {port}/{} to be released before starting...",
                        port + 1
                    ),
                    "panel",
                    false,
                )
                .await;
            announced = true;
        }
        if Instant::now() >= deadline {
            bail!(
                "UDP ports {port}/{} are still in use after {}s — stop whatever holds them \
                 (or pick another port) and start again",
                port + 1,
                BUDGET.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
