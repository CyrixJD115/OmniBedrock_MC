//! Docker deployment plumbing: the detached `docker run` argv builder
//! (limits, networking, mounts, restart policy), the container state
//! monitor, and attaching console/command plumbing to a running container.

use omnibedrock_core::config::Config;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::commands::CommandSink;
use super::console::Console;
use super::runtime::{RunKind, Running};
use super::spawn::spawn_process;
use super::types::{Backend, LaunchContext, ServerSettings};
use crate::docker;

/// Full `docker run` argv for an instance: detached, `-i` so the panel's
/// attach client can forward console commands to the server's stdin, and the
/// restart policy that makes Docker the instance's supervisor across crashes
/// and host reboots. The instance's server directory is mounted read-write at
/// /data (the SAME directory the panel uses); the image only provides the
/// runtime (see deploy/docker).
pub(super) fn docker_run_args(
    config: &Config,
    settings: &ServerSettings,
    ctx: &LaunchContext,
    host_network: bool,
) -> Vec<std::ffi::OsString> {
    let name = crate::instances::container_name(&ctx.instance_id);
    let mut args: Vec<std::ffi::OsString> = vec![
        "docker".into(),
        "run".into(),
        "-d".into(),
        "--restart".into(),
        // The panel's auto_restart setting maps onto the daemon policy: off
        // means the container stays down after any exit.
        if settings.auto_restart {
            "unless-stopped"
        } else {
            "no"
        }
        .into(),
        "--name".into(),
        name.into(),
    ];
    if let Some(mb) = ctx.resources.memory_mb {
        args.push("--memory".into());
        args.push(format!("{mb}m").into());
        // Equal swap limit: the container cannot exceed its memory budget by swapping.
        args.push("--memory-swap".into());
        args.push(format!("{mb}m").into());
    }
    if let Some(cpus) = ctx.resources.cpus {
        args.push("--cpus".into());
        args.push(format!("{cpus}").into());
    }
    // Run as the panel's own uid/gid so server files written into the /data
    // bind mount stay owned by (and deletable by) the panel user. Bedrock's
    // ports are unprivileged, so no container needs root.
    #[cfg(unix)]
    {
        let uid = nix::unistd::getuid();
        let gid = nix::unistd::getgid();
        args.push("--user".into());
        args.push(format!("{}:{}", uid.as_raw(), gid.as_raw()).into());
    }
    if host_network {
        // Linux: the container shares the host network stack, so game ports
        // and ICE ranges behave exactly like a native process and the server
        // binds the instance's own registered port pair.
        args.push("--network".into());
        args.push("host".into());
    } else {
        // Docker Desktop (Windows/macOS): host networking is unavailable, so
        // the container keeps the stock port pair and it is mapped through.
        args.push("-p".into());
        args.push(format!("{}:19132/udp", ctx.port).into());
        args.push("-p".into());
        args.push(format!("{}:19133/udp", ctx.port + 1).into());
    }
    args.push("-v".into());
    args.push(
        format!(
            "{}:/data",
            config
                .bedrock_server_dir
                .to_string_lossy()
                .replace('\\', "/")
        )
        .into(),
    );
    for mount in &ctx.docker.extra_mounts {
        args.push("-v".into());
        args.push(mount.clone().into());
    }
    args.push("-w".into());
    args.push("/data".into());
    args.push(ctx.docker.image_for(settings.backend, config).into());
    // Container paths: the server dir is /data, and stdin comes from the
    // command FIFO the panel also writes from the host.
    let server_argv: Vec<std::ffi::OsString> = match settings.backend {
        Backend::Vanilla => vec!["./bedrock_server".into()],
        Backend::Endstone => vec![
            "python".into(),
            "-m".into(),
            "endstone".into(),
            "--no-confirm".into(),
            "-s".into(),
            "/data".into(),
        ],
    };
    args.extend(docker::command_override(&server_argv));
    args
}

/// Watch one running container via `docker inspect` and report the moment it
/// stops running. The attach client's exit alone is NOT a reliable container
/// exit signal: the daemon can restart the container under its restart policy,
/// which detaches clients while the container keeps going.
fn spawn_docker_monitor(container: String, exit: oneshot::Sender<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut errors = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(750)).await;
            match docker::state_and_exit(&container).await {
                Some((status, _code)) if status == "running" => {
                    errors = 0;
                }
                Some((status, code)) => {
                    let _ = exit.send(match code {
                        Some(c) => format!("container {status} (exit code {c})"),
                        None => format!("container {status}"),
                    });
                    return;
                }
                None => {
                    errors += 1;
                    if errors >= 5 {
                        let _ = exit.send("docker unreachable".into());
                        return;
                    }
                }
            }
        }
    })
}

/// Attach console + command plumbing to an already-running container (fresh
/// start, revive recheck, or adoption after a panel restart): spawn the
/// `docker logs -f` reader, the container state monitor, and point the
/// command sink at the FIFO inside the bind-mounted server directory. The
/// container itself must already be running.
pub(super) async fn attach_docker(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    settings: &ServerSettings,
    container: &str,
) -> anyhow::Result<Running> {
    let fifo = docker::host_stdin_fifo(&config.bedrock_server_dir);
    crate::systemd::ensure_fifo(&fifo).await?;
    let mut running = spawn_process(
        config,
        console,
        ctx,
        settings.backend,
        docker::logs_argv(container, 200),
        Some(container.to_string()),
        false,
        Some(CommandSink::Fifo(fifo)),
    )
    .await?;
    let (exit_tx, exit_rx) = oneshot::channel();
    spawn_docker_monitor(container.to_string(), exit_tx);
    if let RunKind::Process { watch, .. } = &mut running.kind {
        *watch = Some(exit_rx);
    }
    Ok(running)
}

/// Attach to the instance's container when it is running; error otherwise.
/// Never starts or creates anything — revival is exclusively the daemon's
/// restart policy's job, so the panel cannot fight a manual `docker stop`.
pub(super) async fn attach_running_container(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    settings: &ServerSettings,
) -> anyhow::Result<Running> {
    let container = crate::instances::container_name(&ctx.instance_id);
    match docker::state(&container).await.as_deref() {
        Some("running") => attach_docker(config, console, ctx, settings, &container).await,
        _ => anyhow::bail!("container {container} is not running"),
    }
}

#[cfg(test)]
mod docker_args_tests {
    use super::*;
    use crate::instances::{Deployment, DockerConfig, ResourcePolicy};

    fn ctx() -> LaunchContext {
        LaunchContext {
            instance_id: "test-instance".into(),
            deployment: Deployment::Docker,
            port: 20000,
            docker: DockerConfig {
                image: Some("omnibedrock-test:1".into()),
                extra_mounts: vec!["/srv/media:/data/media:ro".into()],
            },
            ..LaunchContext::default()
        }
    }

    fn plain(args: &[std::ffi::OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn detached_run_args_carry_policy_mounts_and_host_network() {
        let args = plain(&docker_run_args(
            &Config::default(),
            &ServerSettings::default(),
            &ctx(),
            true,
        ));
        assert_eq!(args[0], "docker");
        // Detached: the daemon owns the container; the panel only reads its
        // logs and writes its command FIFO.
        assert!(args.contains(&"-d".to_string()));
        assert!(!args.contains(&"-i".to_string()));
        let restart = args.iter().position(|a| a == "--restart").unwrap();
        assert_eq!(args[restart + 1], "unless-stopped");
        let name = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name + 1], "omnibedrock-test-instance");
        let net = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[net + 1], "host");
        assert!(
            !args.iter().any(|a| a == "-p"),
            "host network maps no ports"
        );
        let mount = args.iter().position(|a| a == "-v").unwrap();
        assert!(args[mount + 1].ends_with(":/data"));
        assert_eq!(
            args.iter()
                .filter(|a| a.as_str() == "/srv/media:/data/media:ro")
                .count(),
            1
        );
        let workdir = args.iter().position(|a| a == "-w").unwrap();
        assert_eq!(args[workdir + 1], "/data");
        let image = args.iter().position(|a| a == "omnibedrock-test:1").unwrap();
        // The command override follows the image and redirects stdin to the
        // FIFO both sides of the bind mount share.
        assert_eq!(
            args[image + 1..],
            [
                "sh",
                "-c",
                "exec python -m endstone --no-confirm -s /data <> /data/stdin.fifo"
            ]
        );
    }

    #[test]
    fn mapped_ports_replace_host_network_and_policy_follows_auto_restart() {
        let settings = ServerSettings {
            auto_restart: false,
            ..Default::default()
        };
        let args = plain(&docker_run_args(
            &Config::default(),
            &settings,
            &ctx(),
            false,
        ));
        assert!(!args.iter().any(|a| a == "--network"));
        let p1 = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p1 + 1], "20000:19132/udp");
        assert!(args.contains(&"20001:19133/udp".to_string()));
        let restart = args.iter().position(|a| a == "--restart").unwrap();
        assert_eq!(args[restart + 1], "no");
    }

    #[test]
    fn image_falls_back_to_the_config_default_per_runtime() {
        let config = Config::default();
        let mut c = ctx();
        c.docker.image = None;
        let args = plain(&docker_run_args(
            &config,
            &ServerSettings::default(),
            &c,
            true,
        ));
        assert!(args.contains(&config.docker_image_endstone));
    }

    #[test]
    fn limits_translate_to_container_flags() {
        let mut c = ctx();
        c.resources = ResourcePolicy {
            memory_mb: Some(512),
            cpus: Some(1.5),
        };
        let args = plain(&docker_run_args(
            &Config::default(),
            &ServerSettings::default(),
            &c,
            true,
        ));
        let memory = args.iter().position(|a| a == "--memory").unwrap();
        assert_eq!(args[memory + 1], "512m");
        assert!(args.contains(&"--memory-swap".to_string()));
        let cpus = args.iter().position(|a| a == "--cpus").unwrap();
        assert_eq!(args[cpus + 1], "1.5");
    }
}
