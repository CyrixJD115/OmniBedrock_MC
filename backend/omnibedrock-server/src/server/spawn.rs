//! Spawning the panel-owned child processes (a native server or a docker
//! logs reader) and wiring their stdio into the console: command
//! configuration, the spawn itself, and process-group signalling.

use anyhow::Context;
use omnibedrock_core::config::Config;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::commands::{CommandSink, WriteCommand};
use super::console::Console;
use super::pipes::{PipeAdvice, read_pipe_advised};
use super::runtime::{RunKind, Running};
use super::types::{Backend, LaunchContext};
use crate::instances::Deployment;

/// Spawn one process the panel owns (a native server, or the `docker logs`
/// reader of a container) and wire its stdio into the console. `with_stdin`
/// is false for log readers; their sink comes from `sink_override`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn spawn_process(
    config: &Config,
    console: &Console,
    ctx: &LaunchContext,
    backend: Backend,
    argv: Vec<std::ffi::OsString>,
    container: Option<String>,
    with_stdin: bool,
    sink_override: Option<CommandSink>,
) -> anyhow::Result<Running> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if ctx.deployment != Deployment::Docker {
        // Native children inherit the server directory so vanilla BDS finds
        // its own files; containers run with -w inside the image.
        cmd.current_dir(&config.bedrock_server_dir);
    }
    configure_process(&mut cmd, with_stdin);
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "launch {} instance ({})",
            ctx.deployment.as_str(),
            argv[0].to_string_lossy()
        )
    })?;
    let pid = child.id().context("child has no PID")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let (tx, writer) = if with_stdin {
        let mut stdin = child.stdin.take().context("missing stdin")?;
        let (tx, mut rx) = mpsc::channel::<WriteCommand>(config.command_queue_capacity);
        let writer = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                if cmd.reply.is_closed() {
                    continue;
                }
                let result = async {
                    stdin.write_all(cmd.text.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                let failed = result.is_err();
                let _ = cmd.reply.send(result);
                if failed {
                    break;
                }
            }
        });
        (Some(tx), Some(writer))
    } else {
        (None, None)
    };
    // The advisory port mirrors what launch() pinned into server.properties:
    // port-mapped containers use the stock internal pair, everything else
    // the instance's registered port.
    let advice = PipeAdvice {
        backend,
        fallback_port: match ctx.deployment {
            Deployment::Docker if !cfg!(target_os = "linux") => 19132,
            _ => ctx.port,
        },
    };
    let readers = vec![
        tokio::spawn(read_pipe_advised(
            stdout,
            console.clone(),
            "stdout",
            config.console_max_line_bytes,
            Some(advice),
        )),
        tokio::spawn(read_pipe_advised(
            stderr,
            console.clone(),
            "stderr",
            config.console_max_line_bytes,
            Some(advice),
        )),
    ];
    tracing::info!(
        pid,
        instance = %ctx.instance_id,
        backend = backend.as_str(),
        deployment = ctx.deployment.as_str(),
        container = ?container,
        "game server started"
    );
    let sink = match sink_override {
        Some(sink) => sink,
        None => CommandSink::Pipe(tx.clone().context("pipe sink without stdin child")?),
    };
    Ok(Running {
        pid,
        sink,
        kind: RunKind::Process {
            child,
            stdin: tx,
            writer,
            readers,
            container,
            watch: None,
        },
    })
}
fn configure_process(cmd: &mut Command, with_stdin: bool) {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PYTHONUNBUFFERED", "1");
    if with_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    #[cfg(unix)]
    cmd.process_group(0);
}
pub(super) fn signal_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        if let Err(e) = killpg(
            Pid::from_raw(pid as i32),
            if force {
                Signal::SIGKILL
            } else {
                Signal::SIGTERM
            },
        ) && e != nix::errno::Errno::ESRCH
        {
            tracing::warn!(%e, "process group signal failed");
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}
