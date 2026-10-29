//! Endstone release registry: the PyPI JSON API, its newest-first version
//! ordering, and the shared TTL-cached fetch.
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Endstone's own release registry: the PyPI JSON API. `info.version` is the
/// latest stable release; `releases` keys enumerate every published version.
const PYPI_JSON: &str = "https://pypi.org/pypi/endstone/json";
pub(super) const REGISTRY_TTL: Duration = Duration::from_secs(600);

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
pub(crate) fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
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

/// Validate a requested version against the registry.
pub fn version_exists(registry: &EndstoneRegistry, version: &str) -> bool {
    registry.versions.iter().any(|v| v == version)
}

/// Fetch the Endstone release registry with a bare client, honoring the
/// shared TTL cache so installs, status reads, and the versions endpoint all
/// see the same snapshot.
pub(super) async fn fetch_registry_cached(
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
pub(super) async fn fetch_registry(http: &reqwest::Client) -> anyhow::Result<EndstoneRegistry> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
