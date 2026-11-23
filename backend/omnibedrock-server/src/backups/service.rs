//! The `Backups` service: settings access, backup listing with metadata,
//! job orchestration (built-in save hold / snapshot / resume for running
//! servers), verification, trash handling and restore. Hooks live in
//! `hooks.rs`, the scheduler and retention in `schedule.rs`.

use crate::server::{ServerManager, Status};
use anyhow::bail;
use omnibedrock_core::{
    config::Config,
    error::{ApiError, Result},
    files::{atomic_write, component, dirs, safe_path},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::{Mutex, broadcast, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::archive::{create_zip, restore_archive, verify_archive};
use super::settings::{defaults, merge, validate_settings};
use super::types::{BackupRequest, Hooks};

/// Pre-restore copies of a world kept (hidden) per world.
const PRE_RESTORE_KEEP: usize = 2;

#[derive(Clone)]
pub struct Backups {
    pub(super) config: Arc<Config>,
    /// Where backup settings persist; per-instance backups keep their own file.
    settings_path: PathBuf,
    pub(super) server: ServerManager,
    pub(super) settings: watch::Sender<Value>,
    settings_lock: Arc<Mutex<()>>,
    pub(super) job: Arc<Mutex<()>>,
    active: Arc<AtomicBool>,
    pub events: broadcast::Sender<Value>,
    pub(super) shutdown: CancellationToken,
    /// Scheduler bookkeeping (last / next scheduled run).
    pub(super) schedule: Arc<std::sync::Mutex<super::schedule::ScheduleState>>,
}
struct ActiveGuard(Arc<AtomicBool>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Metadata sidecar next to each archive (`<name>.zip.meta.json`).
pub(super) fn meta_path(zip: &Path) -> PathBuf {
    let mut name = zip.file_name().unwrap_or_default().to_os_string();
    name.push(".meta.json");
    zip.with_file_name(name)
}

/// World folders that are backup targets (shared rule in the core crate).
pub(super) fn is_world_name(name: &str) -> bool {
    omnibedrock_core::files::is_world_dir_name(name)
}

impl Backups {
    pub async fn new(
        config: Arc<Config>,
        server: ServerManager,
        shutdown: CancellationToken,
    ) -> anyhow::Result<Self> {
        let settings_path = config.data_dir.join("backup_settings.yaml");
        Self::with_settings_path(config, server, shutdown, settings_path).await
    }
    /// Instance-scoped constructor with an explicit settings file location.
    pub async fn with_settings_path(
        config: Arc<Config>,
        server: ServerManager,
        shutdown: CancellationToken,
        settings_path: PathBuf,
    ) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(config.backups_dir.join("worlds")).await?;
        let mut settings = defaults();
        match tokio::fs::read_to_string(&settings_path).await {
            Ok(s) => {
                let raw: Value = serde_yaml::from_str(&s)?;
                for key in ["manual", "auto", "pre_post"] {
                    merge(&mut settings[key], &raw[key]);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        validate_settings(&settings)?;
        let result = Self {
            config,
            settings_path,
            server,
            settings: watch::channel(settings).0,
            settings_lock: Arc::new(Mutex::new(())),
            job: Arc::new(Mutex::new(())),
            active: Arc::new(AtomicBool::new(false)),
            events: broadcast::channel(2048).0,
            shutdown,
            schedule: Arc::default(),
        };
        atomic_write(
            &result.settings_path,
            serde_yaml::to_string(&result.settings())?.as_bytes(),
        )
        .await?;
        Ok(result)
    }
    pub fn settings(&self) -> Value {
        self.settings.borrow().clone()
    }
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub fn emit(&self, v: Value) {
        let _ = self.events.send(v);
    }
    pub async fn wait_idle(&self) {
        let _guard = self.job.lock().await;
    }
    pub async fn update_settings(&self, patch: Value) -> Result<Value> {
        let _lock = self.settings_lock.lock().await;
        let mut new = self.settings();
        for key in ["manual", "auto", "pre_post"] {
            if !patch[key].is_null() {
                if !patch[key].is_object() {
                    return Err(ApiError::bad("Settings section must be an object"));
                }
                merge(&mut new[key], &patch[key]);
            }
        }
        validate_settings(&new)?;
        atomic_write(
            &self.settings_path,
            serde_yaml::to_string(&new)
                .map_err(anyhow::Error::from)?
                .as_bytes(),
        )
        .await?;
        self.settings.send_replace(new.clone());
        Ok(new)
    }
    pub async fn worlds(&self) -> Result<Vec<String>> {
        Ok(dirs(&self.config.bedrock_server_dir.join("worlds"))
            .await?
            .into_iter()
            .filter(|n| is_world_name(n))
            .collect())
    }
    pub async fn list(&self, world: Option<String>, trash: bool) -> Result<Vec<Value>> {
        let root = self.config.backups_dir.join("worlds");
        let names = if let Some(w) = world {
            component(&w)?;
            vec![w]
        } else {
            dirs(&root).await?
        };
        let mut result = Vec::new();
        for world in names {
            let relative = if trash {
                PathBuf::from(&world).join(".trash")
            } else {
                PathBuf::from(&world)
            };
            let path = safe_path(&root, &relative).await?;
            let mut entries = match tokio::fs::read_dir(path).await {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                if !entry.file_type().await?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".zip") {
                    continue;
                }
                let m = entry.metadata().await?;
                let modified: chrono::DateTime<chrono::Utc> = m.modified()?.into();
                let meta = tokio::fs::read(meta_path(&entry.path()))
                    .await
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .unwrap_or(Value::Null);
                let kind = if name.starts_with("auto_backup") {
                    "auto"
                } else {
                    "manual"
                };
                result.push(json!({"filename":name,"world":world,"size_bytes":m.len(),"modified":modified.to_rfc3339(),"kind":kind,"meta":meta}));
            }
        }
        result.sort_by(|a, b| b["modified"].as_str().cmp(&a["modified"].as_str()));
        Ok(result)
    }
    pub async fn path(&self, world: &str, filename: &str, trash: bool) -> Result<PathBuf> {
        component(world)?;
        component(filename)?;
        if !filename.ends_with(".zip") {
            return Err(ApiError::bad("Backup must be a ZIP archive"));
        }
        let relative = if trash {
            PathBuf::from(world).join(".trash").join(filename)
        } else {
            PathBuf::from(world).join(filename)
        };
        safe_path(&self.config.backups_dir.join("worlds"), &relative).await
    }
    /// Move a backup (and its metadata) into or out of the world's trash.
    pub async fn move_trash(&self, world: &str, filename: &str, restore: bool) -> Result<()> {
        let _guard = self
            .job
            .clone()
            .try_lock_owned()
            .map_err(|_| ApiError::conflict("A backup job is already running"))?;
        self.move_trash_unlocked(world, filename, restore).await
    }
    pub(super) async fn move_trash_unlocked(
        &self,
        world: &str,
        filename: &str,
        restore: bool,
    ) -> Result<()> {
        let from = self.path(world, filename, restore).await?;
        let to = self.path(world, filename, !restore).await?;
        if tokio::fs::try_exists(&to).await? {
            return Err(ApiError::conflict("Destination already exists"));
        }
        tokio::fs::create_dir_all(to.parent().ok_or_else(|| ApiError::bad("Invalid path"))?)
            .await?;
        tokio::fs::rename(&from, &to).await?;
        let _ = tokio::fs::rename(meta_path(&from), meta_path(&to)).await;
        Ok(())
    }
    pub async fn run(&self, request: BackupRequest) -> Result<Value> {
        component(&request.world)?;
        component(&request.zip_prefix)?;
        if !request.tag.is_empty() {
            component(&request.tag)?;
        }
        if !["deflate", "store"].contains(&request.compression.as_str()) {
            return Err(ApiError::bad("Compression must be deflate or store"));
        }
        if let Some(items) = &request.include_items {
            for item in items {
                safe_path(
                    &self
                        .config
                        .bedrock_server_dir
                        .join("worlds")
                        .join(&request.world),
                    Path::new(item),
                )
                .await?;
            }
        }
        let hooks: Hooks = serde_json::from_value(self.settings()["pre_post"].clone())
            .map_err(|e| ApiError::bad(e.to_string()))?;
        if request.run_hooks {
            hooks.validate(false)?;
        }
        // Job owns its lock and continues even if its HTTP client disconnects.
        let guard = self
            .job
            .clone()
            .try_lock_owned()
            .map_err(|_| ApiError::conflict("A backup job is already running"))?;
        self.active.store(true, Ordering::Release);
        let service = self.clone();
        Ok(tokio::spawn(async move {
            let _guard = guard;
            let _active = ActiveGuard(service.active.clone());
            service.run_inner(request, hooks).await
        })
        .await
        .map_err(anyhow::Error::from)?)
    }
    async fn run_inner(&self, r: BackupRequest, hooks: Hooks) -> Value {
        let started = Instant::now();
        self.emit(json!({"type":"status","phase":"pre","message":"Starting backup"}));
        self.emit(json!({"type":"progress","percent":2,"message":"pre-commands"}));
        // The panel holds saves itself when the server runs; user hooks that
        // already send `save hold` / `save resume` are not doubled.
        let user_holds = hooks
            .before
            .iter()
            .any(|h| h.r#type == "send" && h.text().trim() == "save hold");
        let held = AtomicBool::new(false);
        let live = !r.dry_run && self.server.snapshot().status == Status::Running;
        let result = async {
            if r.run_hooks {
                self.phase("pre", &hooks.before, r.dry_run).await?;
            }
            if self.shutdown.is_cancelled() {
                bail!("Backup cancelled during shutdown");
            }
            if r.dry_run {
                self.emit(json!({"type":"status","phase":"zip","message":"Dry-run: no archive written"}));
                return Ok::<_, anyhow::Error>(None);
            }
            let snapshot = if live {
                if !user_holds {
                    self.emit(json!({"type":"output","stream":"pre","line":"[panel] save hold"}));
                    self.send_capture("save hold").await?;
                    held.store(true, Ordering::Release);
                }
                self.wait_snapshot(&r.world).await?
            } else {
                None
            };
            self.emit(json!({"type":"status","phase":"zip","message":"Creating archive"}));
            let directory = self.export_directory(&r.world, &r.export_folder).await?;
            tokio::fs::create_dir_all(&directory).await?;
            let filename = format!(
                "{}{}_{}_{}_{}.zip",
                r.zip_prefix,
                if r.tag.is_empty() { String::new() } else { format!("_{}", r.tag) },
                r.world,
                chrono::Utc::now().format("%Y-%m-%d-%H%M%S"),
                &Uuid::new_v4().simple().to_string()[..8]
            );
            let destination = directory.join(&filename);
            let source = safe_path(&self.config.bedrock_server_dir.join("worlds"), Path::new(&r.world)).await?;
            let request = r.clone();
            let events = self.events.clone();
            let dest = destination.clone();
            let stats = tokio::task::spawn_blocking(move || {
                let progress = |done: u64, total: u64| {
                    let pct = if total == 0 { 80 } else { 5 + (done * 75 / total.max(1)).min(75) };
                    let _ = events.send(json!({"type":"progress","percent":pct,"message":"zipping","done_bytes":done,"total_bytes":total}));
                };
                create_zip(&source, &dest, &request, snapshot.as_ref(), &progress)
            })
            .await??;
            self.emit(json!({"type":"output","stream":"zip","line":format!(
                "Created {filename}: {} files, {} MB{}{}",
                stats.files,
                stats.bytes / (1024 * 1024),
                if stats.truncated > 0 { format!(", {} truncated to the held length", stats.truncated) } else { String::new() },
                if stats.skipped > 0 { format!(", {} unheld db files skipped", stats.skipped) } else { String::new() },
            )}));
            Ok(Some((destination, filename, stats)))
        }
        .await;
        // Always resume saving if the panel held it, even after a failure.
        if held.load(Ordering::Acquire) {
            self.emit(json!({"type":"output","stream":"post","line":"[panel] save resume"}));
            if let Err(e) = self.send_capture("save resume").await {
                tracing::error!(%e, "save resume failed after backup");
            }
        }
        self.emit(json!({"type":"progress","percent":85,"message":"post-commands"}));
        let post = if r.run_hooks {
            self.phase("post", &hooks.after, r.dry_run).await
        } else {
            Ok(())
        };
        let result = match (result, post) {
            (Ok(file), Ok(())) => Ok(file),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
        let value = match result {
            Ok(Some((destination, filename, stats))) => {
                // Verify after save resume so the server is never held for it.
                let verified = if r.verify {
                    self.emit(json!({"type":"progress","percent":92,"message":"verifying"}));
                    let (path, world) = (destination.clone(), r.world.clone());
                    match tokio::task::spawn_blocking(move || verify_archive(&path, &world)).await {
                        Ok(Ok(_)) => Some(true),
                        Ok(Err(e)) => {
                            self.emit(json!({"type":"error","message":format!("Verification failed: {e}")}));
                            Some(false)
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let size = tokio::fs::metadata(&destination)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                let meta = json!({
                    "world": r.world, "tag": r.tag, "created_at": chrono::Utc::now().to_rfc3339(),
                    "duration_ms": started.elapsed().as_millis() as u64, "live": live, "files": stats.files,
                    "bytes": stats.bytes, "size_bytes": size, "truncated": stats.truncated,
                    "skipped": stats.skipped, "compression": r.compression, "full_backup": r.full_backup,
                    "verified": verified, "server_version": self.server.console.version(),
                });
                let _ = atomic_write(
                    &super::service::meta_path(&destination),
                    meta.to_string().as_bytes(),
                )
                .await;
                self.emit(json!({"type":"progress","percent":100,"message":"done"}));
                if verified == Some(false) {
                    json!({"success":false,"filename":filename,"message":"Backup written but failed verification","meta":meta})
                } else {
                    json!({"success":true,"filename":filename,"message":"Backup complete","meta":meta})
                }
            }
            Ok(None) => {
                self.emit(json!({"type":"progress","percent":100,"message":"done"}));
                json!({"success":true,"filename":null,"message":"Dry run complete"})
            }
            Err(e) => {
                tracing::error!(error=?e,"backup failed");
                self.emit(json!({"type":"error","message":e.to_string()}));
                json!({"success":false,"filename":null,"message":e.to_string()})
            }
        };
        let mut event = value.clone();
        event["type"] = json!("done");
        self.emit(event);
        value
    }
    async fn export_directory(&self, world: &str, folder: &str) -> Result<PathBuf> {
        let root = &self.config.backups_dir;
        let folder_path = Path::new(folder);
        if folder.is_empty() {
            return safe_path(root, &PathBuf::from("worlds").join(world)).await;
        }
        // Permit absolute export paths only within the administrator-configured backup root.
        let relative = if folder_path.is_absolute() {
            let canonical = tokio::fs::canonicalize(root).await?;
            folder_path
                .strip_prefix(&canonical)
                .map_err(|_| ApiError::bad("Export folder must be inside BACKUPS_DIR"))?
                .to_path_buf()
        } else {
            PathBuf::from("worlds").join(folder_path)
        };
        safe_path(root, &relative).await
    }
    /// Re-read an archive end to end (CRC checks) and record the result.
    pub async fn verify(&self, world: &str, filename: &str) -> Result<Value> {
        let path = self.path(world, filename, false).await?;
        let (p, w) = (path.clone(), world.to_owned());
        let started = Instant::now();
        let outcome = tokio::task::spawn_blocking(move || verify_archive(&p, &w))
            .await
            .map_err(anyhow::Error::from)?;
        let meta_file = meta_path(&path);
        let mut meta = tokio::fs::read(&meta_file)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or_else(|| json!({"world": world}));
        let ok = outcome.is_ok();
        meta["verified"] = json!(ok);
        meta["verified_at"] = json!(chrono::Utc::now().to_rfc3339());
        let _ = atomic_write(&meta_file, meta.to_string().as_bytes()).await;
        Ok(match outcome {
            Ok((entries, bytes)) => {
                json!({"ok":true,"entries":entries,"bytes":bytes,"elapsed_ms":started.elapsed().as_millis() as u64})
            }
            Err(e) => json!({"ok":false,"error":e.to_string()}),
        })
    }
    /// Restore an archive over its world. The current world is kept as a
    /// hidden `.<world>.pre-restore-<ts>` folder (newest two per world), so a
    /// bad restore can be undone and never shows up as a world of its own.
    pub async fn restore_world(
        &self,
        world: String,
        filename: String,
        start_after: bool,
    ) -> Result<()> {
        let guard = self
            .job
            .clone()
            .try_lock_owned()
            .map_err(|_| ApiError::conflict("A backup job is already running"))?;
        let path = self.path(&world, &filename, false).await?;
        let worlds_root = self.config.bedrock_server_dir.join("worlds");
        let target = safe_path(&worlds_root, Path::new(&world)).await?;
        // Caller serializes lifecycle actions with restore through AppState.operations.
        self.server.action("stop").await?;
        let w = world.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            restore_archive(&path, &target, &w)?;
            prune_pre_restore(&worlds_root, &w);
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(anyhow::Error::from)??;
        if start_after {
            self.server.action("start").await?;
        }
        Ok(())
    }
}

/// Keep only the newest [`PRE_RESTORE_KEEP`] pre-restore copies of `world`.
fn prune_pre_restore(worlds_root: &Path, world: &str) {
    let prefix = format!(".{world}.pre-restore-");
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(worlds_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in found.into_iter().skip(PRE_RESTORE_KEEP) {
        let _ = std::fs::remove_dir_all(path);
    }
}
