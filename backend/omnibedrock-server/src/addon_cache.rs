//! Persistent parse cache for the addon listing.
//!
//! Listing every pack re-reads its manifest and walks its files for size and
//! date stats — work that only needs redoing when the pack actually changed.
//! This cache stores the fully built listing item per pack path, invalidated
//! by a fingerprint of the pack's change-sensitive mtimes (manifest, pack
//! directory, icons, texts). Entries survive panel restarts in one JSON file
//! under the cache dir.
use omnibedrock_core::files::atomic_write;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

const CACHE_FILE: &str = "addon-meta.json";
const FORMAT: u64 = 1;

#[derive(Default)]
struct Inner {
    loaded: bool,
    /// pack path -> {"fingerprint": String, "item": listing JSON}
    entries: HashMap<String, Value>,
    dirty: bool,
}

#[derive(Clone)]
pub struct AddonMetaCache {
    path: PathBuf,
    inner: Arc<RwLock<Inner>>,
}

impl AddonMetaCache {
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            path: cache_dir.join(CACHE_FILE),
            inner: Arc::new(RwLock::new(Inner::default())),
        }
    }

    async fn ensure_loaded(&self) {
        let mut inner = self.inner.write().await;
        if inner.loaded {
            return;
        }
        inner.loaded = true;
        let Ok(text) = tokio::fs::read_to_string(&self.path).await else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            tracing::warn!(path = %self.path.display(), "addon metadata cache is corrupt; rebuilding");
            return;
        };
        if value["format"].as_u64() != Some(FORMAT) {
            return;
        }
        if let Some(entries) = value["entries"].as_object() {
            for (path, entry) in entries {
                if entry.get("fingerprint").is_some_and(Value::is_string)
                    && entry.get("item").is_some_and(Value::is_object)
                {
                    inner.entries.insert(path.clone(), entry.clone());
                }
            }
        }
    }

    /// Cached listing item for `path`, valid only while `fingerprint` matches.
    pub async fn lookup(&self, path: &str, fingerprint: &str) -> Option<Value> {
        self.ensure_loaded().await;
        let inner = self.inner.read().await;
        inner
            .entries
            .get(path)
            .filter(|entry| entry["fingerprint"].as_str() == Some(fingerprint))
            .map(|entry| entry["item"].clone())
    }

    pub async fn store(&self, path: String, fingerprint: String, item: Value) {
        self.ensure_loaded().await;
        let mut inner = self.inner.write().await;
        inner
            .entries
            .insert(path, json!({"fingerprint": fingerprint, "item": item}));
        inner.dirty = true;
    }

    /// Drop entries for packs that no longer exist on disk.
    pub async fn retain_paths(&self, live: &HashSet<String>) {
        self.ensure_loaded().await;
        let mut inner = self.inner.write().await;
        let before = inner.entries.len();
        inner.entries.retain(|path, _| live.contains(path));
        inner.dirty |= inner.entries.len() != before;
    }

    /// Persist pending changes; a no-op when nothing changed since load.
    pub async fn flush(&self) {
        let value = {
            let mut inner = self.inner.write().await;
            if !inner.dirty {
                return;
            }
            inner.dirty = false;
            json!({"format": FORMAT, "entries": inner.entries})
        };
        if let Err(e) = atomic_write(
            &self.path,
            serde_json::to_vec(&value).unwrap_or_default().as_slice(),
        )
        .await
        {
            tracing::warn!(error = %e, "failed to persist addon metadata cache");
        }
    }
}

/// Cheap change detector for a pack: nanosecond mtimes of the manifest, the
/// pack directory, both icon spellings, and the texts directory, joined.
/// Deep file edits that touch none of these leave a stale size/date — the
/// force-refresh listing param covers that case.
pub async fn pack_fingerprint(pack_dir: &Path) -> String {
    let mut parts = Vec::with_capacity(5);
    for target in [
        pack_dir.join("manifest.json"),
        pack_dir.to_path_buf(),
        pack_dir.join("pack_icon.png"),
        pack_dir.join("pack_icon.jpg"),
        pack_dir.join("texts"),
    ] {
        let nanos = tokio::fs::metadata(&target)
            .await
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        parts.push(nanos.to_string());
    }
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn item(name: &str) -> Value {
        json!({"name": name})
    }

    #[tokio::test]
    async fn roundtrip_survives_restart_and_fingerprint_change() {
        let dir = TempDir::new().unwrap();
        let cache = AddonMetaCache::new(dir.path());
        cache
            .store("/packs/a".into(), "fp1".into(), item("Alpha"))
            .await;
        cache.flush().await;

        // A fresh instance simulates a panel restart.
        let reloaded = AddonMetaCache::new(dir.path());
        assert_eq!(
            reloaded.lookup("/packs/a", "fp1").await.unwrap()["name"],
            "Alpha"
        );
        assert!(reloaded.lookup("/packs/a", "fp2").await.is_none());

        reloaded
            .store("/packs/a".into(), "fp2".into(), item("Alpha v2"))
            .await;
        reloaded.flush().await;
        let reloaded = AddonMetaCache::new(dir.path());
        assert!(reloaded.lookup("/packs/a", "fp1").await.is_none());
        assert_eq!(
            reloaded.lookup("/packs/a", "fp2").await.unwrap()["name"],
            "Alpha v2"
        );
    }

    #[tokio::test]
    async fn retain_drops_missing_packs_and_corrupt_file_tolerated() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(CACHE_FILE), "not json at all").unwrap();
        let cache = AddonMetaCache::new(dir.path());
        cache.store("/packs/a".into(), "fp".into(), item("A")).await;
        cache
            .store("/packs/gone".into(), "fp".into(), item("G"))
            .await;
        cache
            .retain_paths(&HashSet::from(["/packs/a".to_string()]))
            .await;
        cache.flush().await;

        let reloaded = AddonMetaCache::new(dir.path());
        assert!(reloaded.lookup("/packs/a", "fp").await.is_some());
        assert!(reloaded.lookup("/packs/gone", "fp").await.is_none());
    }
}
