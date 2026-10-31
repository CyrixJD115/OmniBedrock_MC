//! Child output pipes: split oversized lines into bounded console events
//! without discarding bytes, and answer "port may be in use" boot blockers
//! with an actionable advisory line.

use regex::Regex;
use std::sync::LazyLock;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use super::console::Console;
use super::types::Backend;

/// Splits oversized lines into bounded chunks without discarding their bytes. Invalid
/// UTF-8 is replaced for the JSON wire protocol. EOF flushes the unterminated last line.
pub(crate) async fn read_pipe<R: AsyncRead + Unpin>(
    pipe: R,
    console: Console,
    source: &'static str,
    max: usize,
) {
    read_pipe_advised(pipe, console, source, max, None).await
}

/// Regex for the boot-blocker line the Endstone launcher (and BDS) prints
/// when something else holds the game port: "Port [19132] may be in use by
/// another process."
static PORT_IN_USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)port\s*\[?\s*(\d{1,5})\s*\]?\s+may be in use").unwrap());

/// Boot-failure context attached to a game server's output pipes: which
/// backend is starting and which port it would have bound, so a "port may
/// be in use" line can be answered with the fix instead of a raw exit code.
#[derive(Clone, Copy)]
pub(super) struct PipeAdvice {
    pub(super) backend: Backend,
    pub(super) fallback_port: u16,
}

pub(super) async fn read_pipe_advised<R: AsyncRead + Unpin>(
    pipe: R,
    console: Console,
    source: &'static str,
    max: usize,
    advice: Option<PipeAdvice>,
) {
    let mut reader = BufReader::with_capacity(32 * 1024, pipe);
    let mut pending = Vec::with_capacity(4096);
    loop {
        let buf = match reader.fill_buf().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(%e, source, "pipe read failed");
                break;
            }
        };
        if buf.is_empty() {
            break;
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let take = newline
            .map_or(buf.len(), |i| i + 1)
            .min(max - pending.len());
        pending.extend_from_slice(&buf[..take]);
        reader.consume(take);
        let complete = pending.last() == Some(&b'\n');
        if complete || pending.len() == max {
            if complete {
                pending.pop();
                if pending.last() == Some(&b'\r') {
                    pending.pop();
                }
            }
            let line = String::from_utf8_lossy(&pending).into_owned();
            console.publish(line.clone(), source, !complete).await;
            // A held game port kills the boot with exit code 1 moments after
            // this line; answer it with the fix while the cause is on screen.
            if complete
                && let Some(advice) = &advice
                && let Some(capture) = PORT_IN_USE.captures(&line)
            {
                let port: u16 = capture[1].parse().unwrap_or(advice.fallback_port);
                let hint = match advice.backend {
                    Backend::Endstone => format!(
                        "Boot blocked: TCP port {port} is held by another process — the \
                         Endstone launcher claims server-port (TCP) BEFORE the server \
                         starts, then exits. The usual holder is the previous server \
                         process still releasing the port: give it a few seconds and \
                         start again (on Linux, `ss -tlnp sport = :{port}` names whoever \
                         holds it). If the broadcaster or another instance was started \
                         first, stop it and start the server first."
                    ),
                    Backend::Vanilla => format!(
                        "Boot blocked: TCP port {port} is held by another process. Free it or \
                         change server-port in Properties, then start again."
                    ),
                };
                console
                    .publish(format!("[LOCAL] {hint}"), "panel", false)
                    .await;
            }
            pending.clear();
        }
    }
    if !pending.is_empty() {
        console
            .publish(
                String::from_utf8_lossy(&pending).into_owned(),
                source,
                false,
            )
            .await;
    }
}

#[cfg(test)]
mod pipe_advice_tests {
    use super::*;

    fn console(dir: &std::path::Path) -> Console {
        Console::with_journal_root(dir.join("journal"), 128, 65536, 1 << 20)
    }

    #[tokio::test]
    async fn endstone_port_in_use_line_gets_an_actionable_advisory() {
        let dir = tempfile::TempDir::new().unwrap();
        let console = console(dir.path());
        let input = "Port [19132] may be in use by another process. Free up port and re-run \
                     program or adjust server.properties file to use alternate ports for \
                     server\nNOISE: nothing to see here\n";
        read_pipe_advised(
            std::io::Cursor::new(input),
            console.clone(),
            "stdout",
            4096,
            Some(PipeAdvice {
                backend: Backend::Endstone,
                fallback_port: 24474,
            }),
        )
        .await;
        let tail = console.tail(10);
        // Launcher line, panel advisory, then the trailing noise line; the
        // port comes from the log line, not the fallback.
        assert_eq!(tail.len(), 3, "{:?}", tail);
        let advisory = &tail[1];
        assert!(advisory.line.contains("Boot blocked"), "{}", advisory.line);
        assert!(advisory.line.contains("19132"), "{}", advisory.line);
        assert!(advisory.line.contains("broadcaster"), "{}", advisory.line);
        assert_eq!(tail[0].source, "stdout");
        assert_eq!(advisory.source, "panel");
    }

    #[tokio::test]
    async fn vanilla_port_in_use_line_gets_the_plain_advisory() {
        let dir = tempfile::TempDir::new().unwrap();
        let console = console(dir.path());
        // Unbracketed port form falls back to the configured port when the
        // regex misses; here it parses and wins over the fallback.
        read_pipe_advised(
            std::io::Cursor::new("Port 20100 may be in use\n"),
            console.clone(),
            "stderr",
            4096,
            Some(PipeAdvice {
                backend: Backend::Vanilla,
                fallback_port: 19132,
            }),
        )
        .await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 2, "{:?}", tail);
        assert!(
            tail[1].line.contains("20100") && tail[1].line.contains("Boot blocked"),
            "{}",
            tail[1].line
        );
    }

    #[tokio::test]
    async fn ordinary_output_stays_advisory_free() {
        let dir = tempfile::TempDir::new().unwrap();
        let console = console(dir.path());
        read_pipe_advised(
            std::io::Cursor::new("Server started\nIPv4 supported, port: 19132\n"),
            console.clone(),
            "stdout",
            4096,
            Some(PipeAdvice {
                backend: Backend::Endstone,
                fallback_port: 19132,
            }),
        )
        .await;
        assert_eq!(console.tail(10).len(), 2, "{:?}", console.tail(10));
    }
}
