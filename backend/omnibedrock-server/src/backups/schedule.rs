//! Scheduled backups and retention.
//!
//! The cadence is anchored on the last scheduled run (or, after a panel
//! restart, the newest `auto_backup_*` archive), so neither editing settings
//! nor restarting the panel pushes the next backup out by a full interval.
//! After each pass: count- and age-based retention for scheduled archives
//! (manual ones are never aged out) and a purge of old trash.

use super::service::{Backups, meta_path};
use crate::server::Status;
use chrono::{DateTime, Utc};
use omnibedrock_core::error::{ApiError, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

/// Delay before retrying a pass that another server operation blocked.
const DEFER_RETRY: Duration = Duration::from_secs(60);
/// First run after enabling scheduled backups with no previous archive.
const FIRST_RUN_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, Serialize)]
pub struct ScheduleState {
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub last_result: Option<String>,
    pub deferred: bool,
}

fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()
}

impl Backups {
    /// Newest scheduled archive across all worlds (anchors the cadence after
    /// a panel restart).
    fn newest_auto_archive(&self) -> Option<DateTime<Utc>> {
        let root = self.config.backups_dir.join("worlds");
        std::fs::read_dir(root)
            .ok()?
            .flatten()
            .flat_map(|w| std::fs::read_dir(w.path()).into_iter().flatten().flatten())
            .filter(|e| e.file_name().to_string_lossy().starts_with("auto_backup_"))
            .filter_map(|e| e.metadata().ok()?.modified().ok())
            .max()
            .map(DateTime::<Utc>::from)
    }

    pub fn scheduler_status(&self) -> Value {
        let state = self
            .schedule
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut auto = self.settings()["auto"].clone();
        auto["status"] = json!(state);
        auto
    }

    pub fn start_scheduler(&self, operations: Arc<Mutex<()>>) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut settings = service.settings.subscribe();
            let mut anchor = service.newest_auto_archive();
            let mut retry: Option<DateTime<Utc>> = None;
            loop {
                let current = settings.borrow_and_update().clone();
                let enabled = current["auto"]["enabled"].as_bool().unwrap_or(false);
                let interval = chrono::Duration::minutes(
                    current["auto"]["interval_minutes"]
                        .as_i64()
                        .unwrap_or(30)
                        .max(1),
                );
                let now = Utc::now();
                let next = retry.unwrap_or_else(|| match anchor {
                    Some(last) => (last + interval).max(now),
                    None => now + chrono::Duration::from_std(FIRST_RUN_DELAY).unwrap_or_default(),
                });
                {
                    let mut s = service.schedule.lock().unwrap_or_else(|e| e.into_inner());
                    s.next_run = enabled.then_some(next);
                    s.last_run = s.last_run.or(anchor);
                }
                let wait = (next - now).to_std().unwrap_or_default();
                tokio::select! {
                    _ = service.shutdown.cancelled() => break,
                    changed = settings.changed() => { if changed.is_err() { break; } continue; }
                    _ = tokio::time::sleep(wait), if enabled => {}
                }
                let Ok(_operation) = operations.try_lock() else {
                    tracing::info!("Scheduled backup deferred: server operation in progress");
                    retry = Some(
                        Utc::now() + chrono::Duration::from_std(DEFER_RETRY).unwrap_or_default(),
                    );
                    service
                        .schedule
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .deferred = true;
                    continue;
                };
                retry = None;
                let outcome = service.tick_once().await;
                let ran_at = Utc::now();
                anchor = Some(ran_at);
                let mut s = service.schedule.lock().unwrap_or_else(|e| e.into_inner());
                s.last_run = Some(ran_at);
                s.deferred = false;
                s.last_result = Some(match outcome {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::error!(error=?e, "scheduled backup failed");
                        format!("failed: {e}")
                    }
                });
            }
        });
    }

    /// Execute one scheduled pass using current settings; returns a summary.
    pub async fn tick_once(&self) -> Result<String> {
        let settings = self.settings();
        let auto = &settings["auto"];
        let worlds: Vec<String> = match auto["worlds"].as_array().filter(|a| !a.is_empty()) {
            Some(a) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            None => self.worlds().await?,
        };
        let (mut made, mut skipped, mut failed) = (0, 0, 0);
        for world in worlds {
            if self.shutdown.is_cancelled() {
                break;
            }
            if auto["skip_if_idle"] == true && self.idle_since_last_backup(&world).await {
                skipped += 1;
                self.emit(json!({"type":"output","stream":"pre","line":format!("Skipped {world}: server stopped and nothing changed since the last backup")}));
                continue;
            }
            let mut data = auto.clone();
            data["world"] = json!(world);
            data["tag"] = json!("auto");
            data["zip_prefix"] = json!("auto_backup");
            let request = serde_json::from_value(data).map_err(|e| ApiError::bad(e.to_string()))?;
            let result = self.run(request).await?;
            if result["success"] != true {
                failed += 1;
                continue;
            }
            made += 1;
            self.apply_retention(&world, auto).await?;
        }
        let days = auto["trash_days"].as_u64().unwrap_or(7);
        if days > 0 {
            self.purge_trash(None, Some(Duration::from_secs(days * 86_400)))
                .await?;
        }
        Ok(format!(
            "{made} created, {skipped} skipped (idle), {failed} failed"
        ))
    }

    /// Server stopped and no world file newer than the newest backup.
    async fn idle_since_last_backup(&self, world: &str) -> bool {
        if self.server.snapshot().status == Status::Running {
            return false;
        }
        let Ok(list) = self.list(Some(world.to_owned()), false).await else {
            return false;
        };
        let Some(newest) = list
            .first()
            .and_then(|b| b["modified"].as_str())
            .and_then(|m| DateTime::parse_from_rfc3339(m).ok())
        else {
            return false;
        };
        let dir = self.config.bedrock_server_dir.join("worlds").join(world);
        let changed = tokio::task::spawn_blocking(move || newest_mtime(&dir))
            .await
            .ok()
            .flatten();
        changed.is_some_and(|m| DateTime::<Utc>::from(m) <= newest.with_timezone(&Utc))
    }

    /// Trash scheduled archives beyond `keep_count` or older than `keep_days`
    /// (always keeping the newest one). Manual backups are never aged out.
    async fn apply_retention(&self, world: &str, auto: &Value) -> Result<()> {
        let keep = auto["keep_count"].as_u64().unwrap_or(10) as usize;
        let max_age = auto["keep_days"].as_u64().unwrap_or(0);
        let cutoff = (max_age > 0).then(|| Utc::now() - chrono::Duration::days(max_age as i64));
        let scheduled: Vec<Value> = self
            .list(Some(world.to_owned()), false)
            .await?
            .into_iter()
            .filter(|v| {
                v["filename"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("auto_backup_"))
            })
            .collect();
        for (i, backup) in scheduled.iter().enumerate() {
            let too_many = i >= keep;
            let too_old = i > 0
                && cutoff.is_some_and(|c| {
                    backup["modified"]
                        .as_str()
                        .and_then(|m| DateTime::parse_from_rfc3339(m).ok())
                        .is_some_and(|m| m.with_timezone(&Utc) < c)
                });
            if (too_many || too_old)
                && let Some(name) = backup["filename"].as_str()
            {
                // The pass already holds the job lock through run(); use the
                // unlocked mover to avoid self-deadlock.
                self.move_trash_unlocked(world, name, false).await?;
            }
        }
        Ok(())
    }

    /// Permanently delete trashed archives (optionally only those older than
    /// `older_than`). Returns (files, bytes) removed.
    pub async fn purge_trash(
        &self,
        world: Option<&str>,
        older_than: Option<Duration>,
    ) -> Result<(u64, u64)> {
        let trash = self.list(world.map(str::to_owned), true).await?;
        let now = SystemTime::now();
        let (mut files, mut bytes) = (0u64, 0u64);
        for item in trash {
            let (Some(w), Some(name)) = (item["world"].as_str(), item["filename"].as_str()) else {
                continue;
            };
            let path: PathBuf = self.path(w, name, true).await?;
            if let Some(age) = older_than {
                let modified = tokio::fs::metadata(&path)
                    .await
                    .and_then(|m| m.modified())
                    .unwrap_or(now);
                if now.duration_since(modified).unwrap_or_default() < age {
                    continue;
                }
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                files += 1;
                bytes += item["size_bytes"].as_u64().unwrap_or(0);
                let _ = tokio::fs::remove_file(meta_path(&path)).await;
            }
        }
        Ok((files, bytes))
    }
}
