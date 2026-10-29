//! Command plumbing between the manager, the supervisor, and a running
//! server: control messages on the supervisor channel, stdin write requests,
//! and the pipe/FIFO command sinks they are dispatched through.

use anyhow::Context;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::types::ServerSettings;

type Reply = oneshot::Sender<anyhow::Result<String>>;
pub(super) enum Control {
    Action(String, Reply),
    Settings(ServerSettings, Reply),
    /// Adopt a systemd unit that is already active (panel restart).
    Adopt(Reply),
    Shutdown(Reply),
}
pub(super) struct WriteCommand {
    pub(super) text: String,
    pub(super) reply: oneshot::Sender<anyhow::Result<()>>,
}

/// How console commands reach a running server: an owned child's stdin pipe
/// or the FIFO of a systemd unit service.
#[derive(Clone)]
pub(super) enum CommandSink {
    Pipe(mpsc::Sender<WriteCommand>),
    Fifo(std::path::PathBuf),
}

pub(super) async fn dispatch_command(sink: &CommandSink, text: &str) -> anyhow::Result<()> {
    match sink {
        CommandSink::Pipe(tx) => write_command(tx, text).await,
        CommandSink::Fifo(path) => tokio::time::timeout(Duration::from_secs(5), async {
            use tokio::io::AsyncWriteExt;
            // Open read+write: a write-only open blocks until a reader
            // appears, and the panel must never block on the server.
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
        .context("FIFO command timed out")?,
    }
}

pub(super) async fn write_command(
    tx: &mpsc::Sender<WriteCommand>,
    command: &str,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (reply, rx) = oneshot::channel();
        tx.send(WriteCommand {
            text: command.into(),
            reply,
        })
        .await
        .context("stdin closed")?;
        rx.await.context("stdin closed")?
    })
    .await
    .context("stdin command timed out")?
}
