//! Vanilla Bedrock Dedicated Server version management.
//!
//! The version registry and SHA256 checksums come from the
//! EndstoneMC/bedrock-server-data repository (MIT); archives themselves are
//! downloaded from the official Minecraft CDN URLs the registry provides.
use crate::server::Console;
use anyhow::Context;
use omnibedrock_core::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub const REGISTRY_BASE: &str =
    "https://raw.githubusercontent.com/EndstoneMC/bedrock-server-data/v2";
const MANIFEST_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Registry {
    pub release: Channel,
    #[serde(default)]
    pub preview: Channel,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Channel {
    #[serde(default)]
    pub latest: String,
    #[serde(default)]
    pub versions: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Metadata {
    pub version: String,
    pub binary: PlatformBinaries,
}
#[derive(Clone, Debug, Deserialize)]
pub struct PlatformBinaries {
    pub linux: Binary,
    pub windows: Binary,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Binary {
    pub url: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct InstallState {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub progress: Option<f64>,
}
impl InstallState {
    pub fn idle() -> Self {
        Self {
            phase: "idle".into(),
            version: None,
            detail: None,
            progress: None,
        }
    }
}

enum Control {
    Install {
        version: String,
        reply: tokio::sync::oneshot::Sender<anyhow::Result<String>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct Bds {
    control: mpsc::Sender<Control>,
    pub state: watch::Receiver<InstallState>,
    http: reqwest::Client,
    registry: Arc<tokio::sync::RwLock<Option<(std::time::Instant, Registry)>>>,
    config: Arc<Config>,
}

impl Bds {
    pub fn new(
        config: Arc<Config>,
        console: Console,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Self {
        let (state_tx, state) = watch::channel(InstallState::idle());
        let (control, control_rx) = mpsc::channel(4);
        let http = reqwest::Client::builder()
            .user_agent(concat!("OmniBedrock/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(600))
            .build()
            .expect("reqwest client");
        let manager = Self {
            control,
            state,
            http: http.clone(),
            registry: Arc::new(tokio::sync::RwLock::new(None)),
            config,
        };
        tokio::spawn(worker(
            manager.config.clone(),
            control_rx,
            state_tx,
            http,
            console,
            shutdown,
        ));
        manager
    }
    pub fn installed_marker(config: &Config) -> PathBuf {
        config.data_dir.join("bds_version.json")
    }
    pub async fn installed_version(&self) -> Option<Value> {
        let text = tokio::fs::read_to_string(Self::installed_marker(&self.config))
            .await
            .ok()?;
        serde_json::from_str(&text).ok()
    }
    fn channel_of(version: &str) -> &'static str {
        if version.contains("-preview") {
            "preview"
        } else {
            "release"
        }
    }
    /// Resolve "latest" to a concrete registry version for the channel.
    pub async fn resolve(&self, version: &str) -> anyhow::Result<String> {
        if version != "latest" {
            return Ok(version.to_string());
        }
        let registry = self.registry().await?;
        Ok(registry.release.latest.clone())
    }
    pub async fn registry(&self) -> anyhow::Result<Registry> {
        if let Some((at, cached)) = self.registry.read().await.clone()
            && at.elapsed() < MANIFEST_TTL
        {
            return Ok(cached);
        }
        let url = format!("{REGISTRY_BASE}/versions.json");
        let text = self
            .http
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| anyhow::anyhow!("fetch version registry: {e}"))?
            .text()
            .await?;
        let registry: Registry = serde_json::from_str(&text).context("parse version registry")?;
        *self.registry.write().await = Some((std::time::Instant::now(), registry.clone()));
        Ok(registry)
    }
    /// Queue a version installation; the reply resolves when the install finishes.
    pub async fn install(
        &self,
        version: &str,
    ) -> anyhow::Result<tokio::sync::oneshot::Receiver<anyhow::Result<String>>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.control
            .send(Control::Install {
                version: version.into(),
                reply: tx,
            })
            .await
            .context("installer stopped")?;
        Ok(rx)
    }
    pub async fn shutdown(&self) {
        let _ = self.control.send(Control::Shutdown).await;
    }
}

async fn worker(
    config: Arc<Config>,
    mut control: mpsc::Receiver<Control>,
    state: watch::Sender<InstallState>,
    http: reqwest::Client,
    console: Console,
    shutdown: tokio_util::sync::CancellationToken,
) {
    loop {
        let message = tokio::select! {
            _ = shutdown.cancelled() => break,
            m = control.recv() => match m {
                Some(m) => m,
                None => break,
            },
        };
        match message {
            Control::Shutdown => break,
            Control::Install { version, reply } => {
                if state.borrow().phase != "idle" {
                    let _ = reply.send(Err(anyhow::anyhow!(
                        "another BDS install is already running"
                    )));
                    continue;
                }
                let result = run_install(&config, &state, &http, &console, &version).await;
                state.send_modify(|s| {
                    s.phase = "idle".into();
                    s.progress = None;
                });
                let _ = reply.send(result);
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn run_install(
    config: &Config,
    state: &watch::Sender<InstallState>,
    http: &reqwest::Client,
    console: &Console,
    requested: &str,
) -> anyhow::Result<String> {
    let version = if requested == "latest" {
        let registry_text = http
            .get(format!("{REGISTRY_BASE}/versions.json"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| anyhow::anyhow!("fetch version registry: {e}"))?
            .text()
            .await?;
        let registry: Registry = serde_json::from_str(&registry_text)?;
        registry.release.latest
    } else {
        requested.to_string()
    };
    let channel = Bds::channel_of(&version);
    state.send_modify(|s| {
        s.phase = "resolving".into();
        s.version = Some(version.clone());
        s.detail = None;
        s.progress = None;
    });
    let metadata_text = http
        .get(format!("{REGISTRY_BASE}/{channel}/{version}/metadata.json"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow::anyhow!("fetch metadata for {version}: {e}"))?
        .text()
        .await?;
    let metadata: Metadata = serde_json::from_str(&metadata_text)?;
    let platform = if cfg!(windows) {
        &metadata.binary.windows
    } else {
        &metadata.binary.linux
    };
    console
        .publish(
            format!("[LOCAL] BDS {version}: downloading from {}", platform.url),
            "panel",
            false,
        )
        .await;
    state.send_modify(|s| s.phase = "downloading".into());
    let archive = download_verified(http, &platform.url, &platform.sha256, &version, state).await?;
    state.send_modify(|s| {
        s.phase = "installing".into();
        s.progress = None;
    });
    console
        .publish(
            format!("[LOCAL] BDS {version}: applying to server directory"),
            "panel",
            false,
        )
        .await;
    apply_archive(&archive, &config.bedrock_server_dir).await?;
    let marker = json!({"version": metadata.version, "requested": requested, "installed_at": crate::server::now()});
    omnibedrock_core::files::atomic_write(
        &Bds::installed_marker(config),
        serde_json::to_vec(&marker)?.as_slice(),
    )
    .await?;
    let _ = tokio::fs::remove_file(&archive).await;
    console
        .publish(
            format!("[LOCAL] BDS {} installed", metadata.version),
            "panel",
            false,
        )
        .await;
    Ok(format!("BDS {} installed", metadata.version))
}

/// Stream the archive to a temp file, verifying its SHA256 while downloading.
async fn download_verified(
    http: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
    version: &str,
    state: &watch::Sender<InstallState>,
) -> anyhow::Result<PathBuf> {
    let expected = hex::decode(expected_sha256.to_ascii_lowercase())
        .map_err(|_| anyhow::anyhow!("registry SHA256 is not valid hex"))?;
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow::anyhow!("download BDS {version}: {e}"))?;
    let total = response.content_length();
    let temp = std::env::temp_dir().join(format!(
        "omnibedrock-bds-{}-{}.zip",
        version,
        Uuid::new_v4()
    ));
    let mut file = tokio::fs::File::create(&temp).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_report = std::time::Instant::now();
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("download interrupted: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if last_report.elapsed() >= Duration::from_millis(250) {
            last_report = std::time::Instant::now();
            let progress = total.map(|t| (received as f64 / t as f64).clamp(0.0, 1.0));
            state.send_modify(|s| {
                s.progress = progress;
                s.detail = Some(format!("{:.1} MiB", received as f64 / 1_048_576.0));
            });
        }
    }
    file.flush().await?;
    drop(file);
    let digest = hasher.finalize();
    if !digest.as_slice().eq_ignore_ascii_case(expected.as_slice()) {
        let _ = tokio::fs::remove_file(&temp).await;
        anyhow::bail!(
            "SHA256 mismatch for {version}: expected {expected_sha256}, got {}",
            hex::encode(digest)
        );
    }
    state.send_modify(|s| {
        s.progress = Some(1.0);
        s.detail = Some(format!("{:.1} MiB verified", received as f64 / 1_048_576.0));
    });
    Ok(temp)
}

/// Files a BDS upgrade must never clobber when they already exist in the target.
fn is_user_data(entry_name: &str) -> bool {
    let top = entry_name.split(['/', '\\']).next().unwrap_or("");
    matches!(
        top,
        "worlds" | "server.properties" | "allowlist.json" | "permissions.json"
    ) || entry_name.starts_with("worlds/")
}

/// Extract a verified BDS archive over the server directory, preserving user
/// data (worlds, properties, allowlist). Existing upstream files are replaced.
pub async fn apply_archive(archive: &Path, target: &Path) -> anyhow::Result<()> {
    let staging = target.join(format!(".bds-staging-{}", Uuid::new_v4()));
    let result = extract_zip(archive, &staging).await;
    if let Err(e) = &result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(anyhow::anyhow!("extract BDS archive: {e:#}"));
    }
    let apply = copy_preserving(&staging, target).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    apply?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = target.join("bedrock_server");
        if binary.exists() {
            let mut perms = std::fs::metadata(&binary)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&binary, perms)?;
        }
    }
    Ok(())
}

async fn extract_zip(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive)?;
        let mut zip = zip::ZipArchive::new(file)?;
        zip.extract(&dest)?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

async fn copy_preserving(from: &Path, to: &Path) -> anyhow::Result<()> {
    use tokio::fs;
    // zip entries are sanitized by name; walk the extracted staging tree instead.
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let source = entry.path();
            let relative = source
                .strip_prefix(from)
                .expect("walked path under staging root");
            let name = relative.to_string_lossy().replace('\\', "/");
            let destination = to.join(relative);
            if entry.file_type().await?.is_dir() {
                fs::create_dir_all(&destination).await?;
                stack.push(source);
                continue;
            }
            if is_user_data(&name) && tokio::fs::try_exists(&destination).await.unwrap_or(false) {
                continue;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).await?;
            }
            if is_symlink(&source).await {
                anyhow::bail!("BDS archive contains a symlink entry: {name}");
            }
            fs::copy(&source, &destination).await?;
        }
    }
    Ok(())
}

async fn is_symlink(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// Validate that a requested version exists in the registry (offline check
/// against a fetched registry value).
pub fn version_exists(registry: &Registry, version: &str) -> bool {
    if registry.release.versions.contains(&version.to_string()) {
        return true;
    }
    registry.preview.versions.contains(&version.to_string())
}
