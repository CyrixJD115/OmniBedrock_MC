//! The `Backups` service: settings access, backup listing and paths, job
//! orchestration, hook phases with console capture, restore, and scheduling.

use crate::server::{ServerManager, Status};
use anyhow::{Context, bail, ensure};
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
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{Mutex, broadcast, watch},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::archive::{create_zip, restore_archive};
use super::settings::{defaults, merge, validate_settings};
use super::types::{BackupRequest, Hook, Hooks};

#[derive(Clone)]
pub struct Backups {
    config: Arc<Config>,
    /// Where backup settings persist; per-instance backups keep their own file.
    settings_path: PathBuf,
    server: ServerManager,
    settings: watch::Sender<Value>,
    settings_lock: Arc<Mutex<()>>,
    job: Arc<Mutex<()>>,
    active: Arc<AtomicBool>,
    pub events: broadcast::Sender<Value>,
    shutdown: CancellationToken,
}
struct ActiveGuard(Arc<AtomicBool>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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
        dirs(&self.config.bedrock_server_dir.join("worlds")).await
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
                result.push(json!({"filename":name,"world":world,"size_bytes":m.len(),"modified":modified.to_rfc3339()}));
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
    pub async fn move_trash(&self, world: &str, filename: &str, restore: bool) -> Result<()> {
        let _guard = self
            .job
            .clone()
            .try_lock_owned()
            .map_err(|_| ApiError::conflict("A backup job is already running"))?;
        let from = self.path(world, filename, restore).await?;
        let to = self.path(world, filename, !restore).await?;
        if tokio::fs::try_exists(&to).await? {
            return Err(ApiError::conflict("Destination already exists"));
        }
        tokio::fs::create_dir_all(to.parent().ok_or_else(|| ApiError::bad("Invalid path"))?)
            .await?;
        tokio::fs::rename(from, to).await?;
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
            hooks.validate(true)?;
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
        self.emit(json!({"type":"status","phase":"pre","message":"Starting backup"}));
        self.emit(json!({"type":"progress","percent":5,"message":"pre-commands"}));
        let result=async{
            if r.run_hooks{self.phase("pre",&hooks.before,r.dry_run).await?;}
            if self.shutdown.is_cancelled(){bail!("Backup cancelled during shutdown");}
            if !r.dry_run && self.server.snapshot().status==Status::Running {
                ensure!(r.run_hooks,"Live backups require save hold/resume hooks; stop the server to run without hooks");
                self.wait_save_ready().await?;
            }
            if r.dry_run{self.emit(json!({"type":"status","phase":"zip","message":"Dry-run: no archive written"}));return Ok::<_,anyhow::Error>(None);}
            self.emit(json!({"type":"status","phase":"zip","message":"Creating archive"}));self.emit(json!({"type":"progress","percent":40,"message":"zipping"}));
            let directory=self.export_directory(&r.world,&r.export_folder).await?;tokio::fs::create_dir_all(&directory).await?;
            let filename=format!("{}{}_{}_{}_{}.zip",r.zip_prefix,if r.tag.is_empty(){String::new()}else{format!("_{}",r.tag)},r.world,chrono::Utc::now().format("%Y-%m-%d-%H%M%S"),&Uuid::new_v4().simple().to_string()[..8]);
            let destination=directory.join(&filename);let source=safe_path(&self.config.bedrock_server_dir.join("worlds"),Path::new(&r.world)).await?;
            let r_clone=r.clone();tokio::task::spawn_blocking(move||create_zip(&source,&destination,&r_clone)).await??;
            self.emit(json!({"type":"output","stream":"zip","line":format!("Created {filename}")}));Ok(Some(filename))
        }.await;
        // Resume saving even when compression or a pre-hook fails. Dry-run has no side effects.
        self.emit(json!({"type":"progress","percent":85,"message":"post-commands"}));
        let post = if r.run_hooks {
            self.phase("post", &hooks.after, r.dry_run).await
        } else {
            Ok(())
        };
        let result = match (result, post) {
            (Ok(file), Ok(())) => Ok(file),
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(e),
        };
        let value = match result {
            Ok(filename) => {
                self.emit(json!({"type":"progress","percent":100,"message":"done"}));
                json!({"success":true,"filename":filename,"message":"Backup complete"})
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
    async fn phase(&self, phase: &str, entries: &[Hook], dry_run: bool) -> anyhow::Result<()> {
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
                self.emit(json!({"type":"output","stream":"server","line":line.line}));
                output.push(line.line.clone());
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
    async fn wait_save_ready(&self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let lines = self.send_capture("save query").await?;
            if lines.iter().any(|l| {
                let l = l.to_lowercase();
                l.contains("files are now ready") || l.contains("ready to be copied")
            }) {
                return Ok(());
            }
            if self.shutdown.is_cancelled() {
                bail!("Shutdown requested");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!("Server did not confirm save query readiness within 30 seconds")
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
    pub async fn restore_world(&self, world: String, filename: String) -> Result<()> {
        let guard = self
            .job
            .clone()
            .try_lock_owned()
            .map_err(|_| ApiError::conflict("A backup job is already running"))?;
        let path = self.path(&world, &filename, false).await?;
        let target = safe_path(
            &self.config.bedrock_server_dir.join("worlds"),
            Path::new(&world),
        )
        .await?;
        // Caller serializes lifecycle actions with restore through AppState.operations.
        self.server.action("stop").await?;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            restore_archive(&path, &target, &world)
        })
        .await
        .map_err(anyhow::Error::from)??;
        Ok(())
    }
    pub fn start_scheduler(&self, operations: Arc<Mutex<()>>) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut settings = service.settings.subscribe();
            loop {
                let current = settings.borrow_and_update().clone();
                let enabled = current["auto"]["enabled"].as_bool().unwrap_or(false);
                let interval = current["auto"]["interval_minutes"].as_u64().unwrap_or(30) * 60;
                tokio::select! {_=service.shutdown.cancelled()=>break,changed=settings.changed()=>{if changed.is_err(){break;}continue;},_=tokio::time::sleep(Duration::from_secs(interval)),if enabled=>{}}
                let Ok(_operation) = operations.try_lock() else {
                    tracing::info!("Scheduled backup deferred: server operation in progress");
                    continue;
                };
                if let Err(e) = service.tick_once().await {
                    tracing::error!(error=?e,"scheduled backup failed");
                }
            }
        });
    }
    /// Execute one scheduled backup pass using current persisted settings.
    pub async fn tick_once(&self) -> Result<()> {
        let settings = self.settings();
        let auto = &settings["auto"];
        let worlds = auto["worlds"]
            .as_array()
            .filter(|a| !a.is_empty())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });
        let worlds = match worlds {
            Some(w) => w,
            None => self.worlds().await?,
        };
        for world in worlds {
            if self.shutdown.is_cancelled() {
                break;
            }
            let mut data = auto.clone();
            data["world"] = json!(world);
            data["tag"] = json!("auto");
            data["zip_prefix"] = json!("auto_backup");
            let request = serde_json::from_value(data).map_err(|e| ApiError::bad(e.to_string()))?;
            let result = self.run(request).await?;
            if result["success"] != true {
                continue;
            }
            // Retain manual backups. Only age out successful scheduled archives.
            let list = self.list(Some(world.clone()), false).await?;
            let keep = auto["keep_count"].as_u64().unwrap_or(10) as usize;
            for backup in list
                .into_iter()
                .filter(|v| {
                    v["filename"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("auto_backup_"))
                })
                .skip(keep)
            {
                if let Some(name) = backup["filename"].as_str() {
                    self.move_trash(&world, name, false).await?;
                }
            }
        }
        Ok(())
    }
}
