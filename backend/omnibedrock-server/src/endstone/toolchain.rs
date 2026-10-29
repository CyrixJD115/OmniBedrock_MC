//! Provisioning of the pinned standalone uv binary (SHA256-verified download
//! and extraction) and the uv-driven process steps it enables.
use crate::server::Console;
use anyhow::Context;
use omnibedrock_core::config::Config;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

use super::paths::{python_install_dir, uv_binary, uv_cache_dir, uv_dir};

/// Pinned uv standalone release. Checksums below come from the
/// `<asset>.sha256` files published next to each release artifact.
pub(super) const UV_VERSION: &str = "0.13.5";
const UV_RELEASE_BASE: &str = "https://releases.astral.sh/github/uv/releases/download";
/// Endstone requires Python >=3.11; 3.12 is the pinned managed interpreter.
pub(super) const PYTHON_VERSION: &str = "3.12";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(600);

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

/// Download and verify the pinned uv binary unless it is already present.
pub(super) async fn ensure_uv(
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
pub(super) async fn run_uv(
    config: &Config,
    uv: &Path,
    args: &[&str],
    step: &str,
) -> anyhow::Result<String> {
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
pub(super) async fn installed_endstone_version(
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
