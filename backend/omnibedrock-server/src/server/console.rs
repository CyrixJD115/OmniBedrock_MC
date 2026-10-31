//! The bounded console ring: per-instance console state with event
//! publishing, level detection, error-signature and TPS statistics, memory
//! replay with gap reporting, and the disk-backed journal flush.

use crate::journal::Journal;
use omnibedrock_core::config::Config;
use regex::Regex;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};
use tokio::sync::watch;
use uuid::Uuid;

use super::helpers::{detect_level, now};
use super::types::{ConsoleEvent, ErrorStat};

static TPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(tps|mspt)[:\s]*([0-9]+(?:\.[0-9]+)?)").unwrap());
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)+").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+\b").unwrap());

struct ConsoleState {
    lines: VecDeque<Arc<ConsoleEvent>>,
    bytes: usize,
    sequence: u64,
    errors: BTreeMap<String, ErrorStat>,
    tps: Option<f64>,
    /// The last panel advisory published via [`Console::publish_note`], so a
    /// boot retry loop does not re-print an unchanged NOTE on every attempt.
    last_note: Option<String>,
}
#[derive(Clone)]
pub struct Console {
    inner: Arc<Mutex<ConsoleState>>,
    pub stream_id: Uuid,
    pub changed: watch::Sender<u64>,
    pub errors_changed: watch::Sender<u64>,
    max_lines: usize,
    max_bytes: usize,
    journal: Arc<tokio::sync::Mutex<Journal>>,
}
pub struct Replay {
    pub lines: Vec<Arc<ConsoleEvent>>,
    pub gap: Option<(u64, u64)>,
    pub latest: u64,
}
impl Console {
    pub fn new(config: &Config) -> Self {
        Self::with_journal_root(
            config.logs_dir.join("console"),
            config.console_max_lines,
            config.console_max_bytes,
            config.console_journal_bytes,
        )
    }
    /// Console ring for any managed process; the journal root names its spool directory.
    pub fn with_journal_root(
        journal_root: std::path::PathBuf,
        max_lines: usize,
        max_bytes: usize,
        journal_bytes: u64,
    ) -> Self {
        let stream_id = Uuid::new_v4();
        Self {
            journal: Arc::new(tokio::sync::Mutex::new(Journal::new(
                journal_root,
                stream_id,
                journal_bytes,
            ))),
            inner: Arc::new(Mutex::new(ConsoleState {
                lines: VecDeque::new(),
                bytes: 0,
                sequence: 0,
                errors: BTreeMap::new(),
                tps: None,
                last_note: None,
            })),
            stream_id,
            changed: watch::channel(0).0,
            errors_changed: watch::channel(0).0,
            max_lines,
            max_bytes,
        }
    }
    pub async fn publish(&self, line: String, source: &'static str, continued: bool) {
        let mut journal = self.journal.lock().await;
        let (event, sequence, level) = self.insert(line, source, continued);
        if let Err(e) = journal.append(&event).await {
            tracing::error!(%e, "console disk replay unavailable; memory replay remains active");
        }
        self.changed.send_replace(sequence);
        if level == "error" {
            self.errors_changed.send_replace(sequence);
        }
    }
    /// Publish a panel advisory that repeats every boot (the transport NOTE):
    /// identical consecutive notes are skipped, so an auto-restart or retry
    /// loop does not reprint the same line four times in a row. A changed
    /// note — different transport, port, or mapping — always publishes.
    pub async fn publish_note(&self, line: String) {
        {
            let mut s = self.inner.lock().expect("console mutex poisoned");
            if s.last_note.as_deref() == Some(line.as_str()) {
                return;
            }
            s.last_note = Some(line.clone());
        }
        self.publish(line, "panel", false).await;
    }
    fn insert(
        &self,
        line: String,
        source: &'static str,
        continued: bool,
    ) -> (Arc<ConsoleEvent>, u64, &'static str) {
        let level = detect_level(&line);
        let timestamp = now();
        let mut s = self.inner.lock().expect("console mutex poisoned");
        if let Some(c) = TPS.captures(&line)
            && let Ok(value) = c[2].parse::<f64>()
        {
            // MSPT is milliseconds per tick, not TPS.
            s.tps = Some(if c[1].eq_ignore_ascii_case("mspt") {
                if value > 0.0 {
                    (1000.0 / value).min(20.0)
                } else {
                    20.0
                }
            } else {
                value
            });
        }
        if level == "error" {
            let lower = line.to_lowercase();
            let sig = DECIMAL.replace_all(&lower, "@");
            let sig = NUMBER.replace_all(&sig, "#");
            let signature: String = sig
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(120)
                .collect();
            // Bound unique signatures as well as console lines.
            if s.errors.len() >= 1024
                && !s.errors.contains_key(&signature)
                && let Some(key) = s
                    .errors
                    .iter()
                    .min_by(|a, b| a.1.last_seen.total_cmp(&b.1.last_seen))
                    .map(|(k, _)| k.clone())
            {
                s.errors.remove(&key);
            }
            let entry = s.errors.entry(signature.clone()).or_insert(ErrorStat {
                signature,
                count: 0,
                first_seen: timestamp,
                last_seen: timestamp,
            });
            entry.count += 1;
            entry.last_seen = timestamp;
        }
        s.sequence += 1;
        let sequence = s.sequence;
        s.bytes += line.len();
        let event = Arc::new(ConsoleEvent {
            r#type: "console".into(),
            sequence,
            stream_id: self.stream_id,
            line,
            level: level.into(),
            timestamp,
            source: source.into(),
            continued,
        });
        s.lines.push_back(event.clone());
        while s.lines.len() > self.max_lines || s.bytes > self.max_bytes {
            if let Some(line) = s.lines.pop_front() {
                s.bytes -= line.line.len();
            }
        }
        (event, sequence, level)
    }
    pub fn replay(&self, after: Option<u64>, limit: usize) -> Replay {
        let s = self.inner.lock().expect("console mutex poisoned");
        let first = s.lines.front().map_or(s.sequence + 1, |l| l.sequence);
        let gap = after
            .filter(|n| n.saturating_add(1) < first)
            .map(|n| (n + 1, first - 1));
        Replay {
            lines: s
                .lines
                .iter()
                .filter(|l| after.is_none_or(|n| l.sequence > n))
                .take(limit)
                .cloned()
                .collect(),
            gap,
            latest: s.sequence,
        }
    }
    pub async fn replay_reliable(
        &self,
        after: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Replay> {
        let batch = self.replay(after, limit);
        if batch.gap.is_some()
            && let Some(after) = after
        {
            match self.journal.lock().await.replay(after, limit).await {
                Ok(Some(replay)) => return Ok(replay),
                Ok(None) => {}
                Err(e) => tracing::error!(%e, "disk replay failed; reporting memory retention gap"),
            }
        }
        Ok(batch)
    }
    /// The last `limit` events from the in-memory ring, oldest first. The
    /// REST counterpart of `replay` for one-shot readers (scripts, health
    /// checks); deeper history stays on the WebSocket + disk journal.
    pub fn tail(&self, limit: usize) -> Vec<Arc<ConsoleEvent>> {
        let s = self.inner.lock().expect("console mutex poisoned");
        let skip = s.lines.len().saturating_sub(limit);
        s.lines.iter().skip(skip).cloned().collect()
    }
    pub async fn flush(&self) -> anyhow::Result<()> {
        self.journal.lock().await.flush().await
    }
    pub fn errors(&self) -> Vec<ErrorStat> {
        let mut errors: Vec<_> = self
            .inner
            .lock()
            .expect("console mutex poisoned")
            .errors
            .values()
            .cloned()
            .collect();
        errors.sort_by_key(|s| std::cmp::Reverse(s.count));
        errors
    }
    pub fn tps(&self) -> Option<f64> {
        self.inner.lock().expect("console mutex poisoned").tps
    }
    pub fn reset_stats(&self) {
        let mut s = self.inner.lock().expect("console mutex poisoned");
        s.errors.clear();
        s.tps = None;
        self.errors_changed.send_replace(s.sequence);
    }
}
