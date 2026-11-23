//! Which script-module versions a server build supports, learned from the
//! npm registry. Mojang publishes every module beta as
//! `X.Y.Z-beta.<engine>-stable`, so for an engine E the beta in that build
//! is X.Y.Z and every stable version below it is available. Registry
//! answers are cached on disk for a day; an offline table covers
//! `@minecraft/server` when the registry is unreachable.

use super::types::ModuleSupport;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TTL_SECS: i64 = 24 * 3600;

pub type Ver = [u64; 3];

pub fn parse_ver(s: &str) -> Option<Ver> {
    let mut it = s.split(['.', '-']).map(|p| p.parse::<u64>());
    Some([it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?])
}

pub fn fmt_ver(v: Ver) -> String {
    format!("{}.{}.{}", v[0], v[1], v[2])
}

/// Engine (first three parts of `1.26.65.1`).
pub fn engine_of(version: &str) -> Option<Ver> {
    parse_ver(version)
}

/// `@minecraft/server`: first engine where each beta appeared (npm-derived).
const SERVER_FALLBACK: &[(Ver, Ver)] = &[
    ([1, 19, 60], [1, 1, 0]),
    ([1, 19, 80], [1, 2, 0]),
    ([1, 20, 0], [1, 3, 0]),
    ([1, 20, 10], [1, 4, 0]),
    ([1, 20, 30], [1, 6, 0]),
    ([1, 20, 40], [1, 7, 0]),
    ([1, 20, 50], [1, 8, 0]),
    ([1, 20, 60], [1, 9, 0]),
    ([1, 20, 70], [1, 10, 0]),
    ([1, 20, 80], [1, 11, 0]),
    ([1, 21, 0], [1, 12, 0]),
    ([1, 21, 20], [1, 14, 0]),
    ([1, 21, 30], [1, 15, 0]),
    ([1, 21, 40], [1, 16, 0]),
    ([1, 21, 50], [1, 17, 0]),
    ([1, 21, 60], [1, 18, 0]),
    ([1, 21, 70], [2, 0, 0]),
    ([1, 21, 90], [2, 1, 0]),
    ([1, 21, 100], [2, 2, 0]),
    ([1, 21, 111], [2, 3, 0]),
    ([1, 21, 120], [2, 4, 0]),
    ([1, 21, 130], [2, 5, 0]),
    ([1, 26, 0], [2, 6, 0]),
    ([1, 26, 10], [2, 7, 0]),
    ([1, 26, 20], [2, 8, 0]),
    ([1, 26, 30], [2, 9, 0]),
    ([1, 26, 40], [2, 10, 0]),
    ([1, 26, 50], [2, 11, 0]),
    ([1, 26, 60], [2, 12, 0]),
];

#[derive(Serialize, Deserialize, Default)]
struct Cached {
    fetched_at: i64,
    versions: Vec<String>,
}

/// Compute support for `engine` from a module's published version list.
pub fn support_from_versions(module: &str, versions: &[String], engine: Ver) -> ModuleSupport {
    let mut stable: Vec<Ver> = Vec::new();
    let mut betas: Vec<(Ver, Ver, String)> = Vec::new();
    for v in versions {
        if let Some(base) = parse_ver(v)
            && fmt_ver(base) == *v
        {
            stable.push(base);
        } else if let Some((base, rest)) = v.split_once("-beta.")
            && let Some(eng) = rest.strip_suffix("-stable").and_then(parse_ver)
            && let Some(base) = parse_ver(base)
            && eng[0] == 1
            && eng[1] >= 19
        {
            betas.push((eng, base, v.clone()));
        }
    }
    // The newest engine with a published beta that is not newer than ours.
    let best_engine = betas.iter().map(|b| b.0).filter(|e| *e <= engine).max();
    let newest_known = betas.iter().map(|b| b.0).max();
    let mut out = ModuleSupport {
        module: module.to_owned(),
        source: "npm",
        ..ModuleSupport::default()
    };
    match best_engine {
        Some(e) => {
            let here: Vec<&(Ver, Ver, String)> = betas.iter().filter(|b| b.0 == e).collect();
            let floor = here.iter().map(|b| b.1).min().unwrap_or([u64::MAX; 3]);
            out.max_stable = stable
                .iter()
                .filter(|s| **s < floor)
                .max()
                .map(|v| fmt_ver(*v));
            // Betas only match their exact build.
            if e == engine || newest_known == Some(e) {
                out.beta = here.iter().map(|b| b.2.clone()).collect();
                out.beta.sort();
                out.beta.dedup();
            }
        }
        None if !stable.is_empty() && betas.is_empty() => {
            out.max_stable = stable.iter().max().map(|v| fmt_ver(*v));
        }
        None => out.source = "unknown",
    }
    out
}

fn fallback(module: &str, engine: Ver) -> ModuleSupport {
    let mut out = ModuleSupport {
        module: module.to_owned(),
        source: "unknown",
        ..ModuleSupport::default()
    };
    if module != "@minecraft/server" {
        return out;
    }
    if let Some((_, beta)) = SERVER_FALLBACK.iter().rev().find(|(e, _)| *e <= engine) {
        let prev = if beta[1] > 0 {
            [beta[0], beta[1] - 1, 0]
        } else {
            // First minor of a new major: the previous major's last minor.
            SERVER_FALLBACK
                .iter()
                .map(|(_, b)| *b)
                .filter(|b| b[0] < beta[0])
                .max()
                .unwrap_or([1, 0, 0])
        };
        out.max_stable = Some(fmt_ver(prev));
        out.source = "builtin";
    }
    out
}

fn cache_path(dir: &Path, module: &str) -> PathBuf {
    dir.join(format!(
        "{}.json",
        module.trim_start_matches('@').replace('/', "__")
    ))
}

async fn fetch(http: &reqwest::Client, module: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("https://registry.npmjs.org/{}", module.replace('/', "%2F"));
    let bytes = http
        .get(url)
        .header("Accept", "application/vnd.npm.install-v1+json")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(value["versions"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default())
}

/// Resolve support for each `@minecraft/*` module the packs depend on.
pub async fn resolve(
    cache_dir: &Path,
    modules: &[String],
    engine: Option<Ver>,
    refresh: bool,
) -> Vec<ModuleSupport> {
    let Some(engine) = engine else {
        return modules
            .iter()
            .map(|m| ModuleSupport {
                module: m.clone(),
                source: "unknown",
                ..ModuleSupport::default()
            })
            .collect();
    };
    let dir = cache_dir.join("script_api");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .user_agent("omnibedrock-panel")
        .build();
    let now = chrono::Utc::now().timestamp();
    let mut out = Vec::new();
    for module in modules {
        let path = cache_path(&dir, module);
        let cached: Option<Cached> = tokio::fs::read(&path)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let fresh = cached
            .as_ref()
            .is_some_and(|c| now - c.fetched_at < TTL_SECS);
        let versions = if fresh && !refresh {
            cached.map(|c| c.versions)
        } else {
            match http.as_ref().ok() {
                Some(client) => match fetch(client, module).await {
                    Ok(v) if !v.is_empty() => {
                        let body = Cached {
                            fetched_at: now,
                            versions: v.clone(),
                        };
                        if let Ok(bytes) = serde_json::to_vec(&body) {
                            let _ = omnibedrock_core::files::atomic_write(&path, &bytes).await;
                        }
                        Some(v)
                    }
                    _ => cached.map(|c| c.versions),
                },
                None => cached.map(|c| c.versions),
            }
        };
        out.push(match versions {
            Some(v) => {
                let s = support_from_versions(module, &v, engine);
                if s.source == "unknown" {
                    fallback(module, engine)
                } else {
                    s
                }
            }
            None => fallback(module, engine),
        });
    }
    out
}
