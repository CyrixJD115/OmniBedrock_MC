//! Endstone toolchain management through a panel-managed uv.
//!
//! The panel bundles no Python. Instead it provisions, all under the panel
//! data dir and without admin rights or a system Python:
//!
//! 1. a pinned standalone `uv` binary (SHA256-verified download from the
//!    official astral-sh/uv release CDN),
//! 2. a uv-managed CPython (`uv python install`),
//! 3. an Endstone virtualenv (`uv venv` + `uv pip install endstone`).
//!
//! The server supervisor launches the venv's `endstone` executable, so
//! switching to the Endstone backend works on a machine with no Python at all.
use crate::server::Console;
use anyhow::Context;
use omnibedrock_core::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

/// Pinned uv standalone release. Checksums below come from the
/// `<asset>.sha256` files published next to each release artifact.
const UV_VERSION: &str = "0.13.5";
const UV_RELEASE_BASE: &str = "https://releases.astral.sh/github/uv/releases/download";
/// Endstone requires Python >=3.11; 3.12 is the pinned managed interpreter.
const PYTHON_VERSION: &str = "3.12";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(600);
/// Endstone's own release registry: the PyPI JSON API. `info.version` is the
/// latest stable release; `releases` keys enumerate every published version.
const PYPI_JSON: &str = "https://pypi.org/pypi/endstone/json";
const REGISTRY_TTL: Duration = Duration::from_secs(600);

/// The available Endstone versions, mirroring `bds::Registry`'s shape.
#[derive(Clone, Debug, Serialize)]
pub struct EndstoneRegistry {
    pub latest: String,
    /// Every published version, newest first.
    pub versions: Vec<String>,
}

#[derive(Deserialize)]
struct PypiProject {
    info: PypiInfo,
    #[serde(default)]
    releases: HashMap<String, Value>,
}
#[derive(Deserialize)]
struct PypiInfo {
    version: String,
}

/// Newest first, semver-flavored: numeric components compare numerically
/// (`1.2.10` > `1.2.9`), a tagged component (`4rc1`, `4.post1`) sorts just
/// below its plain release and above lower numbers (`0.6.4rc1` > `0.6.3`),
/// and non-numeric components sort below all numeric ones. `latest` always
/// comes straight from PyPI, so this only orders the display list.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(version: &str) -> Vec<(u8, u64, u8, String)> {
        version
            .split(['.', '-', '+'])
            .map(|part| {
                let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
                let tag = &part[digits.len()..];
                match (digits.is_empty(), tag.is_empty()) {
                    // purely non-numeric ("dev"): below every numeric component
                    (true, _) => (0, 0, 1, (*tag).to_string()),
                    // "12": plain numeric
                    (false, true) => (1, digits.parse().unwrap_or(0), 1, String::new()),
                    // "4rc1" / "4.post1": numeric with a pre/post-release tag,
                    // which sorts just below the plain release
                    (false, false) => (1, digits.parse().unwrap_or(0), 0, (*tag).to_string()),
                }
            })
            .collect()
    }
    key(b).cmp(&key(a))
}

fn uv_asset() -> (&'static str, &'static str) {
    if cfg!(windows) {
        if cfg!(target_arch = "aarch64") {
            (
                "uv-aarch64-pc-windows-msvc.zip",
                "09f47edf90726084a70f9426f6e3ca4f46e9bf7cbf23b1716ec64d60b055fae3",
            )
        } else {
            (
                "uv-x86_64-pc-windows-msvc.zip",
                "a38b8d1902b7cfbc76b423ba1bb456f623f42064cdc72628b6075b89afd1019a",
            )
        }
    } else if cfg!(target_arch = "aarch64") {
        (
            "uv-aarch64-unknown-linux-musl.tar.gz",
            "7906c79adb21c81abd559841ba76bb1d9af9d791acac7a041b2972e03ae16538",
        )
    } else {
        (
            "uv-x86_64-unknown-linux-musl.tar.gz",
            "57ef9d9ccb01fb1bd998df995b0dd221b52f7d7841fbea93f2d4463bf61fb7a9",
        )
    }
}

// ─── layout under the panel data dir ────────────────────────────────────────
pub fn uv_dir(config: &Config) -> PathBuf {
    config.data_dir.join("tools").join("uv")
}
pub fn uv_binary(config: &Config) -> PathBuf {
    uv_dir(config).join(if cfg!(windows) { "uv.exe" } else { "uv" })
}
/// Root for the toolchain pieces whose paths leak into the launched server's
/// environment. On Linux, Endstone's launcher injects its runtime through
/// LD_PRELOAD and LD_LIBRARY_PATH built from its own install location and the
/// interpreter's sysconfig — and ld.so splits both variables on spaces, so a
/// data dir with spaces (e.g. "Minecraft Server Panel") silently disables the
/// Endstone runtime. The managed CPython, the Endstone venv, and the uv cache
/// therefore live at a space-free location keyed by a digest of the data dir;
/// temp is the last resort (a reboot there just triggers a quick automatic
/// reinstall on next start). The uv binary itself is only exec'd, so it stays
/// under the data dir.
fn runtime_dir(config: &Config) -> PathBuf {
    let preferred = config.data_dir.join("tools");
    if cfg!(windows) || !contains_whitespace(&preferred) {
        return preferred;
    }
    let digest = Sha256::digest(config.data_dir.to_string_lossy().as_bytes());
    let name = format!("omb-{}", hex::encode(&digest[..4]));
    for base in [
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")),
    ]
    .into_iter()
    .flatten()
    {
        if !base.as_os_str().is_empty() && !contains_whitespace(&base) {
            return base.join("omnibedrock").join(name);
        }
    }
    std::env::temp_dir().join(name)
}
pub fn python_install_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("python")
}
pub fn uv_cache_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("uv-cache")
}
pub fn venv_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("endstone")
}
fn contains_whitespace(path: &Path) -> bool {
    path.to_string_lossy().chars().any(char::is_whitespace)
}
/// The venv's `endstone` console script, used by the server supervisor.
pub fn endstone_binary(config: &Config) -> PathBuf {
    venv_dir(config)
        .join(if cfg!(windows) { "Scripts" } else { "bin" })
        .join(if cfg!(windows) {
            "endstone.exe"
        } else {
            "endstone"
        })
}

// ─── launch candidate resolution ────────────────────────────────────────────
/// How the supervisor would launch Endstone right now.
#[derive(Debug)]
pub enum EndstoneLaunch {
    /// An `endstone` executable (explicit override, the managed venv, or PATH).
    Binary(PathBuf),
    /// A Python interpreter with the `endstone` module importable.
    PythonModule(PathBuf),
}

/// Resolve the Endstone launch command without starting anything. Order:
/// explicit `ENDSTONE_BIN`, the panel-managed uv toolchain venv, `endstone`
/// on PATH, then a legacy `ENDSTONE_PYTHON` whose interpreter can actually
/// import the endstone module (probed, so a bare `python3` never crashes the
/// server into a restart loop).
pub fn resolve_launch(config: &Config) -> Option<EndstoneLaunch> {
    if let Some(bin) = &config.endstone_bin {
        return Some(EndstoneLaunch::Binary(bin.clone()));
    }
    let managed = endstone_binary(config);
    if managed.exists() {
        return Some(EndstoneLaunch::Binary(managed));
    }
    if let Some(found) = find_on_path("endstone") {
        return Some(EndstoneLaunch::Binary(found));
    }
    let python = config.endstone_python.as_ref()?;
    python_can_import_endstone(python).then(|| EndstoneLaunch::PythonModule(python.clone()))
}

/// Look `name` up in PATH, returning the first executable regular file.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe_names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{name}.{ext}"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path) {
        for exe in &exe_names {
            let candidate = dir.join(exe);
            let Ok(meta) = std::fs::metadata(&candidate) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Some(candidate);
        }
    }
    None
}

/// Probe whether `<python> -c "import endstone"` succeeds.
fn python_can_import_endstone(python: &Path) -> bool {
    std::process::Command::new(python)
        .arg("-c")
        .arg("import endstone")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

enum Control {
    Install {
        version: Option<String>,
        reply: tokio::sync::oneshot::Sender<anyhow::Result<String>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct Endstone {
    control: mpsc::Sender<Control>,
    pub state: watch::Receiver<crate::bds::InstallState>,
    http: reqwest::Client,
    registry: Arc<tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>>,
    config: Arc<Config>,
}

impl Endstone {
    pub fn new(
        config: Arc<Config>,
        console: Console,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Self {
        let (state_tx, state) = watch::channel(crate::bds::InstallState::idle());
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
            manager.registry.clone(),
            console,
            shutdown,
        ));
        manager
    }
    pub fn installed_marker(config: &Config) -> PathBuf {
        config.data_dir.join("endstone_toolchain.json")
    }
    pub async fn installed(&self) -> Option<Value> {
        let text = tokio::fs::read_to_string(Self::installed_marker(&self.config))
            .await
            .ok()?;
        serde_json::from_str(&text).ok()
    }
    /// The Endstone version recorded in the install marker, if any.
    pub async fn installed_version(&self) -> Option<String> {
        let installed = self.installed().await?;
        installed["endstone_version"].as_str().map(str::to_string)
    }
    /// Fetch the Endstone release registry (cached for `REGISTRY_TTL`).
    pub async fn registry(&self) -> anyhow::Result<EndstoneRegistry> {
        fetch_registry_cached(&self.http, &self.registry).await
    }
    /// The cached registry without touching the network; `None` until the
    /// first successful fetch. Keeps hot paths (status polling) offline.
    pub fn cached_registry(&self) -> Option<EndstoneRegistry> {
        self.registry
            .try_read()
            .ok()?
            .as_ref()
            .filter(|(at, _)| at.elapsed() < REGISTRY_TTL)
            .map(|(_, r)| r.clone())
    }
    /// Queue a toolchain installation; the reply resolves when it finishes.
    pub async fn install(
        &self,
        version: Option<String>,
    ) -> anyhow::Result<tokio::sync::oneshot::Receiver<anyhow::Result<String>>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.control
            .send(Control::Install { version, reply: tx })
            .await
            .context("installer stopped")?;
        Ok(rx)
    }
    pub async fn shutdown(&self) {
        let _ = self.control.send(Control::Shutdown).await;
    }
}

/// Validate a requested version against the registry.
pub fn version_exists(registry: &EndstoneRegistry, version: &str) -> bool {
    registry.versions.iter().any(|v| v == version)
}

async fn worker(
    config: Arc<Config>,
    mut control: mpsc::Receiver<Control>,
    state: watch::Sender<crate::bds::InstallState>,
    http: reqwest::Client,
    registry: Arc<tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>>,
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
                        "another Endstone install is already running"
                    )));
                    continue;
                }
                // Every instance shares one toolchain under <data_dir>/tools;
                // serialize installs panel-wide, then re-check in case another
                // instance's install just finished the same work.
                // run_install resolves "latest" through the shared registry
                // cache and no-ops when the toolchain is already current, so
                // update requests and start-bootstraps are safe to queue here.
                static TOOLCHAIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
                let _guard = TOOLCHAIN.lock().await;
                let result =
                    run_install(&config, &state, &http, &registry, &console, version).await;
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
    state: &watch::Sender<crate::bds::InstallState>,
    http: &reqwest::Client,
    registry_cache: &tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>,
    console: &Console,
    version: Option<String>,
) -> anyhow::Result<String> {
    // Resolve the target release up front: a versionless install means
    // "latest", resolved through the PyPI registry exactly like the BDS
    // updater resolves "latest" through its version registry. When the
    // registry is unreachable, fall back to letting pip pick the newest
    // release it can see rather than failing the whole install.
    let resolved: Option<String> = match &version {
        Some(v) => Some(v.clone()),
        None => match fetch_registry_cached(http, registry_cache).await {
            Ok(registry) => Some(registry.latest),
            Err(e) => {
                console
                    .publish(
                        format!("[LOCAL] Endstone: {e:#} — pip will pick the newest release"),
                        "panel",
                        false,
                    )
                    .await;
                None
            }
        },
    };
    let spec = match &resolved {
        Some(v) => format!("endstone=={v}"),
        None => "endstone".to_string(),
    };

    let installed_version = read_marker_version(config).await;
    let marker_present = tokio::fs::try_exists(Endstone::installed_marker(config))
        .await
        .unwrap_or(false);
    // A versionless install that is already at the resolved latest is a
    // no-op — this is the update check. Without a resolved version (registry
    // down) keep the old "existing toolchain" skip so routine starts never
    // reinstall just because PyPI was briefly unreachable.
    if version.is_none()
        && match (&resolved, &installed_version) {
            (Some(latest), Some(installed)) => installed == latest,
            (None, _) => {
                marker_present
                    && tokio::fs::try_exists(endstone_binary(config))
                        .await
                        .unwrap_or(false)
            }
            (Some(_), None) => false,
        }
    {
        let summary = tokio::fs::read_to_string(Endstone::installed_marker(config))
            .await
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|marker| {
                format!(
                    "endstone {}, CPython {}, uv {}",
                    marker["endstone_version"].as_str().unwrap_or("?"),
                    marker["python_version"].as_str().unwrap_or(PYTHON_VERSION),
                    marker["uv_version"].as_str().unwrap_or("?")
                )
            })
            .unwrap_or_else(|| "existing toolchain".to_string());
        let already_latest = matches!((&resolved, &installed_version),
            (Some(l), Some(i)) if i == l);
        let message = if already_latest {
            format!("Endstone toolchain already at latest ({summary}) — nothing to update")
        } else {
            format!("Endstone toolchain already installed ({summary}) — skipping reinstall")
        };
        console
            .publish(format!("[LOCAL] {message}"), "panel", false)
            .await;
        return Ok(message);
    }

    // The versionless "update to latest" upgrades the package inside the
    // existing venv when it still holds a working `endstone` executable;
    // explicit version installs keep the old full-recreate semantics (that is
    // the repair path after a broken venv). uv and the managed CPython only
    // need provisioning on the bootstrap path.
    let venv = venv_dir(config);
    let venv_python = venv.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });
    let in_place = version.is_none()
        && tokio::fs::try_exists(endstone_binary(config))
            .await
            .unwrap_or(false);

    state.send_modify(|s| {
        s.phase = "uv".into();
        s.version = resolved.clone();
        s.detail = None;
        s.progress = None;
    });
    let uv = ensure_uv(config, http, state, console).await?;

    if in_place {
        state.send_modify(|s| {
            s.phase = "endstone".into();
            s.progress = None;
        });
        console
            .publish(
                format!(
                    "[LOCAL] Endstone: upgrading venv at {} ({})",
                    venv.display(),
                    spec
                ),
                "panel",
                false,
            )
            .await;
    } else {
        state.send_modify(|s| {
            s.phase = "python".into();
            s.progress = None;
        });
        console
            .publish(
                format!("[LOCAL] Endstone: installing managed CPython {PYTHON_VERSION}"),
                "panel",
                false,
            )
            .await;
        run_uv(
            config,
            &uv,
            &["python", "install", PYTHON_VERSION],
            "python install",
        )
        .await?;

        state.send_modify(|s| {
            s.phase = "venv".into();
            s.progress = None;
        });
        console
            .publish(
                format!(
                    "[LOCAL] Endstone: creating virtualenv at {}",
                    venv.display()
                ),
                "panel",
                false,
            )
            .await;
        run_uv(
            config,
            &uv,
            &[
                "venv",
                venv.to_string_lossy().as_ref(),
                "--python",
                PYTHON_VERSION,
                "--clear",
            ],
            "create venv",
        )
        .await?;

        state.send_modify(|s| {
            s.phase = "endstone".into();
            s.progress = None;
        });
        console
            .publish(
                format!("[LOCAL] Endstone: installing {spec} (prebuilt wheel)"),
                "panel",
                false,
            )
            .await;
    }
    run_uv(
        config,
        &uv,
        &[
            "pip",
            "install",
            "--python",
            venv_python.to_string_lossy().as_ref(),
            "--upgrade",
            &spec,
        ],
        "pip install endstone",
    )
    .await?;
    let installed = installed_endstone_version(config, &uv, &venv_python).await;

    // Drop pre-relocation toolchain pieces if the runtime moved to a
    // space-free location; they can never launch correctly there.
    let preferred = config.data_dir.join("tools");
    if runtime_dir(config) != preferred {
        for legacy in ["endstone", "python", "uv-cache"] {
            let dir = preferred.join(legacy);
            if tokio::fs::try_exists(&dir).await.unwrap_or(false) {
                let _ = tokio::fs::remove_dir_all(&dir).await;
            }
        }
    }

    let mut marker = json!({
        "uv_version": UV_VERSION,
        "python_version": PYTHON_VERSION,
        "endstone_version": installed,
        "binary": endstone_binary(config),
        "installed_at": crate::server::now(),
    });
    if let Some(previous) = installed_version
        && Some(&previous) != installed.as_ref()
    {
        marker["previous_version"] = json!(previous);
    }
    if let Some(resolved) = &resolved {
        marker["requested"] = json!(resolved);
    }
    omnibedrock_core::files::atomic_write(
        &Endstone::installed_marker(config),
        serde_json::to_vec(&marker)?.as_slice(),
    )
    .await?;
    let message = format!(
        "Endstone toolchain ready (endstone {}, CPython {PYTHON_VERSION}, uv {UV_VERSION})",
        installed.as_deref().unwrap_or("unknown")
    );
    console
        .publish(format!("[LOCAL] {message}"), "panel", false)
        .await;
    Ok(message)
}

/// Fetch the Endstone release registry with a bare client, honoring the
/// shared TTL cache so installs, status reads, and the versions endpoint all
/// see the same snapshot.
async fn fetch_registry_cached(
    http: &reqwest::Client,
    cache: &tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>,
) -> anyhow::Result<EndstoneRegistry> {
    if let Some((at, cached)) = cache.read().await.clone()
        && at.elapsed() < REGISTRY_TTL
    {
        return Ok(cached);
    }
    let registry = fetch_registry(http).await?;
    *cache.write().await = Some((std::time::Instant::now(), registry.clone()));
    Ok(registry)
}

/// Fetch the Endstone release registry with a bare client.
async fn fetch_registry(http: &reqwest::Client) -> anyhow::Result<EndstoneRegistry> {
    let text = http
        .get(PYPI_JSON)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow::anyhow!("fetch Endstone release registry: {e}"))?
        .text()
        .await?;
    let project: PypiProject =
        serde_json::from_str(&text).context("parse Endstone release registry")?;
    let mut versions: Vec<String> = project.releases.into_keys().collect();
    versions.sort_by(|a, b| version_cmp(a, b));
    Ok(EndstoneRegistry {
        latest: project.info.version,
        versions,
    })
}

/// The `endstone_version` recorded in the toolchain marker, if any.
async fn read_marker_version(config: &Config) -> Option<String> {
    let text = tokio::fs::read_to_string(Endstone::installed_marker(config))
        .await
        .ok()?;
    let marker: Value = serde_json::from_str(&text).ok()?;
    marker["endstone_version"].as_str().map(str::to_string)
}

/// Download and verify the pinned uv binary unless it is already present.
async fn ensure_uv(
    config: &Config,
    http: &reqwest::Client,
    state: &watch::Sender<crate::bds::InstallState>,
    console: &Console,
) -> anyhow::Result<PathBuf> {
    let target = uv_binary(config);
    if tokio::fs::try_exists(&target).await.unwrap_or(false) {
        return Ok(target);
    }
    let (asset, expected) = uv_asset();
    let url = format!("{UV_RELEASE_BASE}/{UV_VERSION}/{asset}");
    console
        .publish(
            format!("[LOCAL] Endstone: downloading uv {UV_VERSION} ({asset})"),
            "panel",
            false,
        )
        .await;
    let archive = download_verified(http, &url, expected, state).await?;
    state.send_modify(|s| {
        s.phase = "uv-extract".into();
        s.progress = None;
    });
    let dir = uv_dir(config);
    let extracted = extract_first_binary(&archive, &dir).await;
    let _ = tokio::fs::remove_file(&archive).await;
    let binary = extracted?;
    if binary != target {
        tokio::fs::rename(&binary, &target).await?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)?;
    }
    console
        .publish(
            format!(
                "[LOCAL] Endstone: uv {UV_VERSION} installed at {}",
                target.display()
            ),
            "panel",
            false,
        )
        .await;
    Ok(target)
}

/// Stream a file to a temp path, verifying its SHA256 while downloading.
async fn download_verified(
    http: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
    state: &watch::Sender<crate::bds::InstallState>,
) -> anyhow::Result<PathBuf> {
    let expected = hex::decode(expected_sha256.to_ascii_lowercase())
        .map_err(|_| anyhow::anyhow!("pinned SHA256 is not valid hex"))?;
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| anyhow::anyhow!("download uv {UV_VERSION}: {e}"))?;
    let total = response.content_length();
    let temp = std::env::temp_dir().join(format!("omnibedrock-uv-{}", uuid::Uuid::new_v4()));
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
            "SHA256 mismatch for uv {UV_VERSION}: expected {expected_sha256}, got {}",
            hex::encode(digest)
        );
    }
    state.send_modify(|s| {
        s.progress = Some(1.0);
        s.detail = Some(format!("{:.1} MiB verified", received as f64 / 1_048_576.0));
    });
    Ok(temp)
}

/// Extract the `uv` executable from a release archive (tar.gz or zip) into
/// `dir`, returning its path. Archives hold `uv-<target>/uv` (+ `uvx`); only
/// the single uv binary is kept.
async fn extract_first_binary(archive: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let archive = archive.to_path_buf();
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        if cfg!(windows) {
            let file = std::fs::File::open(&archive)?;
            let mut zip = zip::ZipArchive::new(file)?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index)?;
                let name = entry.name().to_string();
                let base = name.rsplit(['/']).next().unwrap_or("");
                if base == "uv.exe" {
                    let target = dir.join("uv.exe");
                    let mut out = std::fs::File::create(&target)?;
                    std::io::copy(&mut entry, &mut out)?;
                    return Ok(target);
                }
            }
            anyhow::bail!("uv.exe not found in archive");
        } else {
            let file = std::fs::File::open(&archive)?;
            let gz = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(gz);
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.to_path_buf();
                if path.file_name().map(|n| n == "uv").unwrap_or(false) {
                    let target = dir.join("uv");
                    let mut out = std::fs::File::create(&target)?;
                    std::io::copy(&mut entry, &mut out)?;
                    return Ok(target);
                }
            }
            anyhow::bail!("uv binary not found in archive");
        }
    })
    .await?
}

/// Run uv with the panel-scoped environment (managed Python + cache under the
/// data dir) and return stdout. The tail of stderr is attached to errors so
/// the console shows why a step failed.
async fn run_uv(config: &Config, uv: &Path, args: &[&str], step: &str) -> anyhow::Result<String> {
    let mut cmd = tokio::process::Command::new(uv);
    cmd.args(args)
        .env("UV_PYTHON_INSTALL_DIR", python_install_dir(config))
        .env("UV_CACHE_DIR", uv_cache_dir(config))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(PROCESS_TIMEOUT, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("uv {step} timed out"))?
        .with_context(|| format!("run uv {step} (is the downloaded binary executable?)"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut lines: Vec<&str> = stderr.lines().collect();
        if lines.len() > 5 {
            lines.drain(..lines.len() - 5);
        }
        let tail = lines.join("\n");
        anyhow::bail!("uv {step} failed ({}):\n{}", output.status, tail);
    }
    Ok(stdout)
}

/// Ask uv which endstone version ended up installed, for the state marker.
async fn installed_endstone_version(
    config: &Config,
    uv: &Path,
    venv_python: &Path,
) -> Option<String> {
    let stdout = run_uv(
        config,
        uv,
        &[
            "pip",
            "show",
            "--python",
            venv_python.to_string_lossy().as_ref(),
            "endstone",
        ],
        "pip show endstone",
    )
    .await
    .ok()?;
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_relocates_only_for_spaced_data_dirs() {
        let mut config = Config {
            data_dir: PathBuf::from("/tmp/omb-clean/data"),
            ..Default::default()
        };
        assert_eq!(
            venv_dir(&config),
            PathBuf::from("/tmp/omb-clean/data/tools/endstone")
        );

        config.data_dir = PathBuf::from("/tmp/omb space test/data");
        for dir in [
            venv_dir(&config),
            python_install_dir(&config),
            uv_cache_dir(&config),
        ] {
            assert!(
                !contains_whitespace(&dir),
                "relocated runtime dir must be space-free, got {dir:?}"
            );
            assert_eq!(dir, dir, "paths must be deterministic");
        }
        assert_eq!(venv_dir(&config), venv_dir(&config));
    }

    #[test]
    fn versions_sort_newest_first() {
        let mut versions = vec![
            "0.5.10",
            "0.5.9",
            "1.0.0",
            "0.10.0",
            "0.9.1",
            "1.0.0rc1",
            "0.5.9.post1",
        ];
        versions.sort_by(|a, b| version_cmp(a, b));
        // rc1 loses to its own release but beats older versions; a `.post1`
        // release sorts just above its base (PEP 440).
        assert_eq!(
            versions,
            vec![
                "1.0.0",
                "1.0.0rc1",
                "0.10.0",
                "0.9.1",
                "0.5.10",
                "0.5.9.post1",
                "0.5.9"
            ]
        );
    }

    #[test]
    fn registry_parses_pypi_shape() {
        let project: PypiProject = serde_json::from_str(
            r#"{
                "info": {"version": "0.6.4"},
                "releases": {
                    "0.6.4": [], "0.6.3": [], "0.6.10": [], "0.6.4rc1": []
                }
            }"#,
        )
        .expect("parse PyPI JSON");
        let mut versions: Vec<String> = project.releases.into_keys().collect();
        versions.sort_by(|a, b| version_cmp(a, b));
        assert_eq!(project.info.version, "0.6.4");
        assert_eq!(versions, vec!["0.6.10", "0.6.4", "0.6.4rc1", "0.6.3"]);
    }
}
