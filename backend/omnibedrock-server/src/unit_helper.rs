//! The hidden `__unit-helper` subcommand: the process systemd runs as the
//! instance service. It is deliberately tiny and synchronous.
//!
//! Responsibilities:
//! - spawn the actual game server with its stdin wired to the instance's
//!   command FIFO and stdout/stderr inherited (so the journal captures
//!   them),
//! - forward a graceful shutdown when systemd (or the panel) asks the
//!   service to stop: first write `stop` into the server's stdin, then
//!   SIGTERM, then SIGKILL — mirroring the panel's own stop ladder,
//! - exit with the server's status so `systemctl` reports it faithfully.

use anyhow::{Context, bail};
use std::{
    ffi::OsString,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Graceful stop budget before SIGTERM, then SIGKILL.
const STOP_TIMEOUT: Duration = Duration::from_secs(8);
const KILL_TIMEOUT: Duration = Duration::from_secs(4);
const POLL: Duration = Duration::from_millis(150);

static SIGNALED: AtomicBool = AtomicBool::new(false);

/// Entry point: `omnibedrock __unit-helper --stdin-fifo <path> -- <argv...>`.
/// Returns the exit code the process should terminate with.
#[cfg(unix)]
pub fn run(args: &[OsString]) -> anyhow::Result<i32> {
    use std::process::Stdio;

    let fifo = parse_fifo(args)?;
    let argv = parse_argv(args)?;
    // Open the FIFO read+write: the write half keeps the read end from ever
    // seeing EOF (the panel may hold no writer at all between commands).
    let fifo_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fifo)
        .with_context(|| format!("open command FIFO {}", fifo.display()))?;
    let child_stdin = fifo_file
        .try_clone()
        .context("clone FIFO handle for the server stdin")?;
    install_signal_handlers()?;
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::from(child_stdin))
        .spawn()
        .with_context(|| format!("spawn {}", argv[0].to_string_lossy()))?;
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let mut stop_sent_at: Option<Instant> = None;
    let mut term_sent_at: Option<Instant> = None;
    loop {
        if SIGNALED.load(Ordering::SeqCst) && stop_sent_at.is_none() {
            stop_sent_at = Some(Instant::now());
            // Same graceful request the panel uses; failures fall through to
            // the signal ladder below.
            let _ = (&fifo_file).write_all(b"stop\n");
            let _ = (&fifo_file).flush();
        }
        if let Some(at) = stop_sent_at
            && at.elapsed() > STOP_TIMEOUT
            && term_sent_at.is_none()
        {
            term_sent_at = Some(Instant::now());
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        }
        if let Some(at) = term_sent_at
            && at.elapsed() > KILL_TIMEOUT
        {
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
        match child.try_wait()? {
            Some(status) => {
                if let Some(code) = status.code() {
                    return Ok(code);
                }
                #[cfg(unix)]
                if let Some(sig) = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                } {
                    return Ok(128 + sig);
                }
                return Ok(1);
            }
            None => std::thread::sleep(POLL),
        }
    }
}

#[cfg(unix)]
fn install_signal_handlers() -> anyhow::Result<()> {
    extern "C" fn handler(_: i32) {
        SIGNALED.store(true, Ordering::SeqCst);
    }
    let action = nix::sys::signal::SigAction::new(
        nix::sys::signal::SigHandler::Handler(handler),
        nix::sys::signal::SaFlags::SA_RESTART,
        nix::sys::signal::SigSet::empty(),
    );
    unsafe {
        nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGTERM, &action)?;
        nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGINT, &action)?;
    }
    Ok(())
}

fn parse_fifo(args: &[OsString]) -> anyhow::Result<std::path::PathBuf> {
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--stdin-fifo" {
            return Ok(Path::new(&args[i + 1]).to_path_buf());
        }
        i += 1;
    }
    bail!("__unit-helper requires --stdin-fifo <path>")
}

fn parse_argv(args: &[OsString]) -> anyhow::Result<Vec<OsString>> {
    let Some(index) = args.iter().position(|a| a == "--") else {
        bail!("__unit-helper requires '-- <server argv>'");
    };
    let argv: Vec<OsString> = args[index + 1..].to_vec();
    if argv.is_empty() {
        bail!("__unit-helper requires a server command after '--'");
    }
    Ok(argv)
}

#[cfg(not(unix))]
pub fn run(_args: &[OsString]) -> anyhow::Result<i32> {
    bail!("systemd instance services are only supported on Linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fifo_and_argv() {
        let args: Vec<OsString> = [
            "--stdin-fifo",
            "/run/omni.fifo",
            "--",
            "/srv/server/bedrock_server",
            "--no-confirm",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert_eq!(parse_fifo(&args).unwrap(), Path::new("/run/omni.fifo"));
        let argv = parse_argv(&args).unwrap();
        assert_eq!(argv[0].to_string_lossy(), "/srv/server/bedrock_server");
        assert_eq!(argv[1].to_string_lossy(), "--no-confirm");
    }

    #[test]
    fn rejects_missing_pieces() {
        assert!(parse_fifo(&[OsString::from("--")]).is_err());
        assert!(parse_argv(&[OsString::from("--stdin-fifo")]).is_err());
        let only_dash: Vec<OsString> = vec![OsString::from("--")];
        assert!(parse_argv(&only_dash).is_err());
    }
}
