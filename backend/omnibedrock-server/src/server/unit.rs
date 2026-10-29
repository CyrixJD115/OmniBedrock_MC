//! systemd user-unit hosting plumbing: the per-instance command FIFO path,
//! the `journalctl -f` console reader, the unit state monitor, and attaching
//! console + monitoring to an already-active unit.

use anyhow::{Context, bail};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::commands::CommandSink;
use super::console::Console;
use super::pipes::read_pipe;
use super::runtime::{RunKind, Running};
use super::types::ServerPaths;

/// FIFO path for an instance's unit service commands.
pub(super) fn unit_fifo(paths: &ServerPaths) -> std::path::PathBuf {
    paths.state_dir.join("stdin.fifo")
}

/// Spawn `journalctl --user -f` for one unit and pipe its output into the
/// console ring, so the panel's Console shows the service journal exactly
/// like a child's stdout.
fn spawn_journal_reader(
    unit: &str,
    console: &Console,
    max_line: usize,
) -> anyhow::Result<JoinHandle<()>> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new("journalctl")
        .args(["--user", "-u", unit, "-n", "200", "-o", "cat", "-f"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("attach journalctl for {unit}"))?;
    let stdout = child.stdout.take().context("journalctl stdout")?;
    Ok(tokio::spawn(read_pipe(
        stdout,
        console.clone(),
        "journal",
        max_line,
    )))
}

/// Watch a unit's state until it stops being active, then report how it
/// ended through the exit channel. systemd itself does not restart the unit
/// (`Restart=no`); the panel's supervisor owns restart policy uniformly.
fn spawn_unit_monitor(id: String, exit: oneshot::Sender<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut errors = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(750)).await;
            match crate::systemd::active_state(&id).await {
                Some(state) if ["active", "activating", "reloading"].contains(&state.as_str()) => {
                    errors = 0;
                }
                Some(state) => {
                    let status = crate::systemd::exit_status(&id)
                        .await
                        .map(|s| format!("exit status {s}"))
                        .unwrap_or_else(|| "no exit status reported".into());
                    let _ = exit.send(format!("unit {state} ({status})"));
                    return;
                }
                None => {
                    // systemctl failing repeatedly means the user manager is
                    // gone; the service is gone with it.
                    errors += 1;
                    if errors >= 5 {
                        let _ = exit.send("user manager unreachable".into());
                        return;
                    }
                }
            }
        }
    })
}

/// Attach console + monitoring to an already-active unit without (re)starting
/// it. Used right after `systemctl start` and to adopt units that kept
/// running across a panel restart. Refuses anything that is not actually
/// `active` — `systemctl show` happily reports `inactive` for units that do
/// not exist, which must never look like adoption success.
pub(super) async fn attach_unit(
    id: &str,
    fifo: &std::path::Path,
    console: &Console,
    max_line: usize,
) -> anyhow::Result<Running> {
    let unit = crate::systemd::unit_name(id);
    if crate::systemd::active_state(id).await.as_deref() != Some("active") {
        bail!("unit {unit} is not active");
    }
    let pid = crate::systemd::main_pid(id)
        .await
        .context("unit is active but has no MainPID yet")?;
    let (exit_tx, exit_rx) = oneshot::channel();
    let monitor = spawn_unit_monitor(id.to_string(), exit_tx);
    let reader = spawn_journal_reader(&unit, console, max_line)?;
    Ok(Running {
        pid,
        sink: CommandSink::Fifo(fifo.to_path_buf()),
        kind: RunKind::Unit {
            id: id.to_string(),
            exit: exit_rx,
            monitor,
            readers: vec![reader],
        },
    })
}
