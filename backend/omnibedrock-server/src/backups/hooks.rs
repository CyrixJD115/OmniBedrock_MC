//! Hook phases (send / wait / command / comment), console capture, the
//! shell runner for `command` hooks, and the live-backup snapshot handshake.

use super::live::{Snapshot, parse_save_query};
use super::service::Backups;
use super::types::Hook;
use crate::server::Status;
use anyhow::{Context, bail, ensure};
use serde_json::json;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

impl Backups {
    pub(super) async fn phase(
        &self,
        phase: &str,
        entries: &[Hook],
        dry_run: bool,
    ) -> anyhow::Result<()> {
        let mut first_error = None;
        for h in entries {
            self.emit(
                json!({"type":"output","stream":phase,"line":format!("[{}] {}",h.r#type,h.text())}),
            );
            if dry_run {
                continue;
            }
            let result = match h.r#type.as_str() {
                "comment" => Ok(()),
                "wait" => {
                    let secs = h
                        .value
                        .as_u64()
                        .or_else(|| h.value.as_str()?.parse().ok())
                        .context("invalid wait")?;
                    tokio::select! {_=self.shutdown.cancelled()=>Err(anyhow::anyhow!("Shutdown requested")),_=tokio::time::sleep(Duration::from_secs(secs))=>Ok(())}
                }
                "send" => {
                    if self.server.snapshot().status == Status::Running {
                        self.send_capture(&h.text()).await.map(|_| ())
                    } else {
                        Ok(())
                    }
                }
                "command" => self
                    .shell(&h.text(), phase, 300)
                    .await
                    .and_then(|(code, _)| {
                        ensure!(code == 0, "Hook command exited with {code}");
                        Ok(())
                    }),
                _ => Err(anyhow::anyhow!("Invalid hook type")),
            };
            if let Err(e) = result {
                if phase == "pre" {
                    return Err(e);
                }
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        if let Some(e) = first_error {
            Err(e)
        } else {
            Ok(())
        }
    }
    pub async fn send_capture(&self, command: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .send_capture_events(command)
            .await?
            .into_iter()
            .map(|(line, _)| line)
            .collect())
    }

    /// Send a console command and collect the response as (line, continued)
    /// pairs, stopping once output goes quiet for 500 ms (5 s max wait for
    /// the first line).
    pub(super) async fn send_capture_events(
        &self,
        command: &str,
    ) -> anyhow::Result<Vec<(String, bool)>> {
        let console = &self.server.console;
        let mut changes = console.changed.subscribe();
        let mut cursor = Some(console.replay(None, 0).latest);
        self.server.send_command(command).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        loop {
            let batch = console.replay(cursor, 256);
            if batch.gap.is_some() {
                bail!("Server response exceeded retention window");
            }
            for line in batch.lines {
                cursor = Some(line.sequence);
                // The save-query file list can be thousands of entries; keep
                // it out of the progress stream.
                let shown: String = line.line.chars().take(300).collect();
                self.emit(json!({"type":"output","stream":"server","line":shown}));
                output.push((line.line.clone(), line.continued));
                if output.len() >= 1000 {
                    return Ok(output);
                }
            }
            let wait = if output.is_empty() {
                deadline.saturating_duration_since(tokio::time::Instant::now())
            } else {
                Duration::from_millis(500)
            };
            if tokio::time::timeout(wait, changes.changed()).await.is_err() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
        }
        Ok(output)
    }

    /// Poll `save query` until the server reports the held files are ready,
    /// and return the `path:length` snapshot for `world` (None when the list
    /// could not be read: the archive then copies whole files).
    pub(super) async fn wait_snapshot(&self, world: &str) -> anyhow::Result<Option<Snapshot>> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            let lines = self.send_capture_events("save query").await?;
            let ready = lines.iter().any(|(l, _)| {
                let l = l.to_lowercase();
                l.contains("files are now ready") || l.contains("ready to be copied")
            });
            if ready {
                let snapshot = parse_save_query(&lines, world);
                match &snapshot {
                    Some(s) => self.emit(json!({"type":"output","stream":"zip","line":format!("Snapshot: {} files held by the server", s.len())})),
                    None => self.emit(json!({"type":"output","stream":"zip","line":"Snapshot list not readable; copying whole files"})),
                }
                return Ok(snapshot);
            }
            if self.shutdown.is_cancelled() {
                bail!("Shutdown requested");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        bail!("Server did not confirm save query readiness within 60 seconds")
    }

    pub async fn shell(
        &self,
        command: &str,
        phase: &str,
        timeout_secs: u64,
    ) -> anyhow::Result<(i32, String)> {
        // `command` hooks run arbitrary shell as the panel user, so they sit
        // behind an explicit operator opt-in; send/wait/comment hooks are
        // unaffected. This gates both scheduled runs and the test endpoint,
        // because both funnel through here.
        anyhow::ensure!(
            self.config.backup_hooks_enabled,
            "Backup command hooks are disabled on this host (BACKUP_HOOKS_ENABLED=false); \
             send/wait/comment hooks keep working"
        );
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        };
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let out = child.stdout.take().context("missing stdout")?;
        let err = child.stderr.take().context("missing stderr")?;
        let service = self.clone();
        let phase = phase.to_owned();
        let mut readers = tokio::spawn(async move {
            async fn read<R: tokio::io::AsyncRead + Unpin>(
                r: R,
                service: &Backups,
                phase: &str,
            ) -> anyhow::Result<String> {
                let mut reader = BufReader::new(r);
                let mut output = String::new();
                let mut chunk = Vec::with_capacity(4096);
                loop {
                    let buf = reader.fill_buf().await?;
                    if buf.is_empty() {
                        break;
                    }
                    let n = buf
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(buf.len(), |i| i + 1)
                        .min(65536 - chunk.len());
                    chunk.extend_from_slice(&buf[..n]);
                    reader.consume(n);
                    if chunk.last() == Some(&b'\n') || chunk.len() == 65536 {
                        let text = String::from_utf8_lossy(&chunk);
                        service.emit(json!({"type":"output","stream":phase,"line":text.trim_end_matches(['\r','\n'])}));
                        if output.len() < 2000 {
                            output.extend(text.chars().take(2000 - output.len()));
                        }
                        chunk.clear();
                    }
                }
                if !chunk.is_empty() {
                    let text = String::from_utf8_lossy(&chunk);
                    service.emit(json!({"type":"output","stream":phase,"line":text}));
                    if output.len() < 2000 {
                        output.extend(text.chars().take(2000 - output.len()));
                    }
                }
                Ok(output)
            }
            let (a, b) =
                tokio::try_join!(read(out, &service, &phase), read(err, &service, &phase))?;
            Ok::<_, anyhow::Error>(format!("{a}{b}"))
        });
        let result = tokio::select! {
            _=self.shutdown.cancelled()=>Err(anyhow::anyhow!("Shutdown requested")),
            result=tokio::time::timeout(Duration::from_secs(timeout_secs),async {let status=child.wait().await?;let output=(&mut readers).await??;Ok::<_,anyhow::Error>((status.code().unwrap_or(-1),output))})=>match result {Ok(result)=>result,Err(_)=>Err(anyhow::anyhow!("Hook command timed out"))},
        };
        #[cfg(unix)]
        if let Some(pid) = pid {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        if result.is_err() {
            let _ = child.kill().await;
            readers.abort();
            let _ = readers.await;
        }
        result
    }
}
