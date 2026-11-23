//! Follows the server console and keeps a persistent player history:
//! who joined, their xuid, first/last seen, session count and playtime.
//! Online state is derived from join/leave lines and reconciled against the
//! `list` command by the API, so adopted systemd/docker servers that were
//! already running when the panel started converge too.

use crate::server::Console;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

static CONNECTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Player connected: (.+?), xuid: (\d*)").expect("regex"));
static SPAWNED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Player Spawned: (.+?) xuid: (\d*), pfid: (\w*)").expect("regex"));
static DISCONNECTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Player disconnected: (.+?), xuid: (\d*)(?:, pfid: (\w*))?").expect("regex")
});

const SAVE_EVERY: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayerRecord {
    pub name: String,
    #[serde(default)]
    pub xuid: Option<String>,
    #[serde(default)]
    pub pfid: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    #[serde(default)]
    pub sessions: u64,
    #[serde(default)]
    pub playtime_secs: u64,
    /// Unix time the current session started; `None` while offline.
    #[serde(default)]
    pub online_since: Option<i64>,
}

#[derive(Default)]
struct State {
    records: BTreeMap<String, PlayerRecord>,
    dirty: bool,
}

/// Lines that end the game process: everyone is offline afterwards.
fn is_shutdown(line: &str) -> bool {
    line.contains("Quit correctly")
        || line.contains("Server stop requested")
        || line.contains("Stopping server...")
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl State {
    fn join(&mut self, name: &str, xuid: &str, pfid: Option<&str>, now: i64) {
        let rec = self
            .records
            .entry(key(name))
            .or_insert_with(|| PlayerRecord {
                name: name.trim().to_owned(),
                first_seen: now,
                ..PlayerRecord::default()
            });
        rec.name = name.trim().to_owned();
        if !xuid.is_empty() {
            rec.xuid = Some(xuid.to_owned());
        }
        if let Some(p) = pfid.filter(|p| !p.is_empty()) {
            rec.pfid = Some(p.to_owned());
        }
        if rec.online_since.is_none() {
            rec.online_since = Some(now);
            rec.sessions += 1;
        }
        rec.last_seen = now;
        self.dirty = true;
    }

    fn leave(&mut self, name: &str, now: i64) {
        if let Some(rec) = self.records.get_mut(&key(name)) {
            if let Some(since) = rec.online_since.take() {
                rec.playtime_secs += (now - since).max(0) as u64;
            }
            rec.last_seen = now;
            self.dirty = true;
        }
    }

    fn all_offline(&mut self, now: i64) {
        let names: Vec<String> = self
            .records
            .values()
            .filter(|r| r.online_since.is_some())
            .map(|r| r.name.clone())
            .collect();
        for n in names {
            self.leave(&n, now);
        }
    }

    fn line(&mut self, line: &str, now: i64) {
        if let Some(c) = SPAWNED.captures(line) {
            self.join(&c[1], &c[2], c.get(3).map(|m| m.as_str()), now);
        } else if let Some(c) = CONNECTED.captures(line) {
            self.join(&c[1], &c[2], None, now);
        } else if let Some(c) = DISCONNECTED.captures(line) {
            if let Some(rec) = self.records.get_mut(&key(&c[1]))
                && let Some(p) = c.get(3).filter(|p| !p.as_str().is_empty())
            {
                rec.pfid = Some(p.as_str().to_owned());
            }
            self.leave(&c[1], now);
        } else if is_shutdown(line) {
            self.all_offline(now);
        }
    }
}

#[derive(Clone)]
pub struct Players {
    state: Arc<Mutex<State>>,
    path: PathBuf,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Players {
    /// Load `<state_dir>/players.json` (sessions left open by a previous
    /// panel run are closed at their last-seen time) and follow `console`.
    pub fn spawn(console: Console, state_dir: &Path, token: CancellationToken) -> Self {
        let path = state_dir.join("players.json");
        let mut state = State::default();
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(list) = serde_json::from_slice::<Vec<PlayerRecord>>(&bytes)
        {
            for mut r in list {
                if let Some(since) = r.online_since.take() {
                    r.playtime_secs += (r.last_seen - since).max(0) as u64;
                }
                state.records.insert(key(&r.name), r);
            }
        }
        let players = Self {
            state: Arc::new(Mutex::new(state)),
            path,
        };
        tokio::spawn(players.clone().follow(console, token));
        players
    }

    async fn follow(self, console: Console, token: CancellationToken) {
        let mut changes = console.changed.subscribe();
        let mut cursor = console.replay(None, 0).latest;
        let mut save = tokio::time::interval(SAVE_EVERY);
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = save.tick() => { self.save().await; continue; }
                r = changes.changed() => if r.is_err() { break },
            }
            loop {
                let batch = console.replay(Some(cursor), 512);
                if batch.lines.is_empty() {
                    cursor = cursor.max(batch.latest);
                    break;
                }
                let t = now();
                let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                for line in &batch.lines {
                    cursor = line.sequence;
                    s.line(&line.line, t);
                }
            }
        }
        self.save().await;
    }

    async fn save(&self) {
        let bytes = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !s.dirty {
                return;
            }
            s.dirty = false;
            serde_json::to_vec_pretty(&s.records.values().collect::<Vec<_>>()).unwrap_or_default()
        };
        if let Err(e) = omnibedrock_core::files::atomic_write(&self.path, &bytes).await {
            tracing::warn!(%e, path = %self.path.display(), "saving player history failed");
        }
    }

    pub fn list(&self) -> Vec<PlayerRecord> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<PlayerRecord> = s.records.values().cloned().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.last_seen));
        out
    }

    pub fn find(&self, name: &str) -> Option<PlayerRecord> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(&key(name))
            .cloned()
    }

    /// Make the online set exactly `names` (from the `list` command, or empty
    /// when the server is not running).
    pub fn reconcile(&self, names: &[String]) {
        let t = now();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let wanted: Vec<String> = names.iter().map(|n| key(n)).collect();
        let stale: Vec<String> = s
            .records
            .values()
            .filter(|r| r.online_since.is_some() && !wanted.contains(&key(&r.name)))
            .map(|r| r.name.clone())
            .collect();
        for n in stale {
            s.leave(&n, t);
        }
        for n in names {
            let missing = s
                .records
                .get(&key(n))
                .is_none_or(|r| r.online_since.is_none());
            if missing {
                let xuid = s
                    .records
                    .get(&key(n))
                    .and_then(|r| r.xuid.clone())
                    .unwrap_or_default();
                s.join(n, &xuid, None, t);
            }
        }
    }

    /// Forget a player's history. Returns whether a record existed.
    pub fn forget(&self, name: &str) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let removed = s.records.remove(&key(name)).is_some();
        s.dirty |= removed;
        removed
    }

    #[cfg(test)]
    pub(super) fn feed(&self, line: &str, at: i64) {
        self.state.lock().unwrap().line(line, at);
    }

    #[cfg(test)]
    pub(super) fn detached(path: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            path,
        }
    }
}
