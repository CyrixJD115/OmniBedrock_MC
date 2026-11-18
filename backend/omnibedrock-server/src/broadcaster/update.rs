//! Keeping MCXboxBroadcast current: look up the newest Standalone build on
//! GitHub, and install it over the jar this instance resolves — verified
//! against the SHA-256 digest GitHub publishes for the release asset, staged
//! next to the target and swapped in with a rename (the previous jar is kept
//! as `.bak`). New Bedrock releases routinely need a new broadcaster build
//! (the jar pins the client protocol), so a stale jar is a common silent
//! cause of "friends can see the world but cannot join".

use anyhow::{Context, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::consts::JAR_NAME;

const RELEASES_API: &str =
    "https://api.github.com/repos/MCXboxBroadcast/Broadcaster/releases/latest";
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
/// Sidecar recording which release the panel installed (`<jar>.build`).
const BUILD_SIDECAR: &str = "build";

#[derive(Clone, Debug, Serialize)]
pub struct JarRelease {
    pub tag: String,
    pub published_at: String,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub sha256: Option<String>,
    pub size: u64,
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("OmniBedrock/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()
        .expect("static HTTP client")
});

static LATEST: LazyLock<Mutex<Option<(Instant, JarRelease)>>> = LazyLock::new(|| Mutex::new(None));

/// The newest Standalone release (cached for half an hour; `force` refetches).
pub async fn latest_release(force: bool) -> anyhow::Result<JarRelease> {
    if !force
        && let Some((at, release)) = LATEST.lock().expect("cache lock").as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return Ok(release.clone());
    }
    let bytes = HTTP
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub is unreachable")?
        .error_for_status()
        .context("GitHub refused the release lookup")?
        .bytes()
        .await
        .context("GitHub response interrupted")?;
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).context("unexpected GitHub response")?;
    let asset = body["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"].as_str() == Some(JAR_NAME)))
        .context("the latest release has no MCXboxBroadcastStandalone.jar")?;
    let release = JarRelease {
        tag: body["tag_name"].as_str().unwrap_or_default().to_owned(),
        published_at: body["published_at"].as_str().unwrap_or_default().to_owned(),
        url: asset["browser_download_url"]
            .as_str()
            .context("asset has no download URL")?
            .to_owned(),
        sha256: asset["digest"]
            .as_str()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase),
        size: asset["size"].as_u64().unwrap_or(0),
    };
    if !release
        .url
        .starts_with("https://github.com/MCXboxBroadcast/Broadcaster/releases/download/")
    {
        bail!("refusing an unexpected download URL: {}", release.url);
    }
    *LATEST.lock().expect("cache lock") = Some((Instant::now(), release.clone()));
    Ok(release)
}

fn sidecar(jar: &Path) -> PathBuf {
    let mut name = jar.as_os_str().to_owned();
    name.push(".");
    name.push(BUILD_SIDECAR);
    PathBuf::from(name)
}

/// The release tag the panel last installed at `jar`, if it did.
pub async fn installed_build(jar: &Path) -> Option<String> {
    tokio::fs::read_to_string(sidecar(jar))
        .await
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Download `release` over `jar` (verified, staged, atomically swapped).
pub async fn install_release(release: &JarRelease, jar: &Path) -> anyhow::Result<()> {
    let dir = jar.parent().context("jar path has no directory")?;
    tokio::fs::create_dir_all(dir).await?;
    let bytes = HTTP
        .get(&release.url)
        .send()
        .await
        .context("download failed")?
        .error_for_status()
        .context("download refused")?
        .bytes()
        .await
        .context("download interrupted")?;
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(b"PK\x03\x04") {
        bail!("the download is not a jar ({} bytes)", bytes.len());
    }
    if release.size > 0 && bytes.len() as u64 != release.size {
        bail!(
            "size mismatch: expected {} bytes, got {}",
            release.size,
            bytes.len()
        );
    }
    if let Some(expected) = &release.sha256 {
        let actual = hex::encode(Sha256::digest(&bytes));
        if &actual != expected {
            bail!("SHA-256 mismatch: expected {expected}, got {actual}");
        }
    }
    let staged = dir.join(format!(".{JAR_NAME}.download"));
    tokio::fs::write(&staged, &bytes).await?;
    if tokio::fs::try_exists(jar).await.unwrap_or(false) {
        let backup = dir.join(format!("{JAR_NAME}.bak"));
        let _ = tokio::fs::remove_file(&backup).await;
        tokio::fs::copy(jar, &backup)
            .await
            .context("could not back up the current jar")?;
    }
    tokio::fs::rename(&staged, jar)
        .await
        .context("could not swap in the new jar")?;
    let _ = tokio::fs::write(sidecar(jar), format!("{}\n", release.tag)).await;
    Ok(())
}

/// Parse the build number the jar prints at boot
/// (`Starting MCXboxBroadcast Standalone build 160 (git-…) for Bedrock 1.26.60`).
pub fn parse_boot_banner(line: &str) -> Option<(String, Option<String>)> {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"Standalone build (\d+)(?:.*?for Bedrock ([0-9.]+))?").unwrap()
    });
    let c = RE.captures(line)?;
    Some((c[1].to_owned(), c.get(2).map(|m| m.as_str().to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_banner_is_parsed() {
        let line = "[01:41:41.540 main/INFO] Starting MCXboxBroadcast Standalone build 160 (git-master-798d8b3) for Bedrock 1.26.60 (2225)";
        assert_eq!(
            parse_boot_banner(line),
            Some(("160".into(), Some("1.26.60".into())))
        );
        assert_eq!(parse_boot_banner("unrelated"), None);
    }
}
