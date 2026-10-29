//! The CurseForge HTTP client: authenticated Core API access with a TTL
//! cache and single-flight coalescing, paginated search/detail lookups, CDN
//! file downloads, and the proxied-image endpoint.
use super::consts::{
    API_BASE, FILES_TTL, GAME_ID_BEDROCK, IMAGE_CACHE_MAX_BYTES, IMAGE_FETCH_TIMEOUT,
    MAX_DOWNLOAD_BYTES, MAX_IMAGE_BYTES, META_TTL, PROJECT_TTL, SEARCH_TTL, VERSIONS_TTL,
};
use super::image_cache::{PruneGuard, image_ext, read_cached_image};
use super::models::{CfCategory, CfFile, CfPagination, CfProject, Envelope};
use super::query::{class_id, sort_field};
use anyhow::Context as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct CurseForge {
    http: reqwest::Client,
    download_http: reqwest::Client,
    /// Runtime-swappable so the panel Settings page can set or clear the key
    /// without a restart.
    api_key: Arc<std::sync::RwLock<String>>,
    cache: Arc<tokio::sync::Mutex<HashMap<String, (Instant, Value)>>>,
    /// Coalesces concurrent identical lookups: the first caller fetches
    /// upstream while the rest wait on the same key and reuse its result
    /// instead of stampeding CurseForge.
    inflight: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Directory for proxied CDN images (survives restarts); None disables caching.
    image_cache_dir: Option<PathBuf>,
    /// Approximate total size of the image cache, kept honest by prune rescans.
    image_cache_bytes: Arc<AtomicU64>,
    image_prune_running: Arc<AtomicBool>,
}

impl CurseForge {
    pub fn new(api_key: &str, image_cache_dir: Option<PathBuf>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("OmniBedrock/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        // File downloads stream hundreds of MiB off the CDN; no overall
        // deadline, only a connect timeout.
        let download_http = reqwest::Client::builder()
            .user_agent(concat!("OmniBedrock/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("reqwest download client");
        Self {
            http,
            download_http,
            api_key: Arc::new(std::sync::RwLock::new(api_key.trim().to_string())),
            cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            inflight: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            image_cache_dir,
            image_cache_bytes: Arc::new(AtomicU64::new(0)),
            image_prune_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn configured(&self) -> bool {
        !self.api_key.read().unwrap().trim().is_empty()
    }

    /// Replace the API key at runtime (panel Settings page). Empty clears it.
    pub fn set_api_key(&self, key: &str) {
        *self.api_key.write().unwrap() = key.trim().to_string();
    }

    fn current_key(&self) -> String {
        self.api_key.read().unwrap().trim().to_string()
    }

    /// Authenticated GET against the Core API with a TTL cache keyed on the
    /// full URL, so paged browsing and repeated detail views stay polite.
    async fn get_json(
        &self,
        path: &str,
        query: &[(&str, String)],
        ttl: Duration,
    ) -> anyhow::Result<Value> {
        if !self.configured() {
            anyhow::bail!(
                "CURSEFORGE_API_KEY is not configured; set it in the panel .env to enable the Store"
            );
        }
        let mut url = reqwest::Url::parse(&format!("{API_BASE}{path}"))?;
        for (key, value) in query {
            if !value.is_empty() {
                url.query_pairs_mut().append_pair(key, value);
            }
        }
        let key = url.as_str().to_string();
        {
            let cache = self.cache.lock().await;
            if let Some((expires, value)) = cache.get(&key)
                && *expires > Instant::now()
            {
                return Ok(value.clone());
            }
        }
        // Single-flight: when another caller is already fetching this exact
        // URL, wait for it and reuse the entry it installs instead of
        // stampeding CurseForge with duplicate requests.
        let flight = self
            .inflight
            .lock()
            .await
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _flight_guard = flight.lock().await;
        {
            let cache = self.cache.lock().await;
            if let Some((expires, value)) = cache.get(&key)
                && *expires > Instant::now()
            {
                return Ok(value.clone());
            }
        }
        let response = self
            .http
            .get(url)
            .header("x-api-key", self.current_key())
            .send()
            .await
            .context("CurseForge API request failed")?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // The search endpoint is gated separately from the rest of the API:
            // fresh keys read catalog data fine but get 403 on /mods/search
            // until CurseForge approves developer search access.
            if path == "/mods/search" {
                anyhow::bail!(
                    "CurseForge denied search for this API key (HTTP {status}). Lookup endpoints work; \
                     /mods/search unlocks once CurseForge approves developer access for the key \
                     (see console.curseforge.com)."
                );
            }
            anyhow::bail!(
                "CurseForge rejected the API key (HTTP {status}); check CURSEFORGE_API_KEY"
            );
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!("CurseForge rate limit reached; retry in a moment");
        }
        let value: Value = serde_json::from_slice(
            &response
                .error_for_status()
                .context("CurseForge API returned an error")?
                .bytes()
                .await
                .context("CurseForge API response read failed")?,
        )
        .context("CurseForge API returned invalid JSON")?;
        if value.get("errors").is_some() {
            anyhow::bail!(
                "CurseForge API error: {}",
                value["errors"].as_array().map_or_else(
                    || value.to_string(),
                    |e| e
                        .iter()
                        .filter_map(|v| v.to_string().into())
                        .collect::<Vec<String>>()
                        .join("; ")
                )
            );
        }
        {
            let mut cache = self.cache.lock().await;
            if cache.len() > 512 {
                cache.retain(|_, (expires, _)| *expires > Instant::now());
            }
            cache.insert(key.clone(), (Instant::now() + ttl, value.clone()));
        }
        // Hand the flight slot back only once nobody is queued behind us.
        if Arc::strong_count(&flight) == 1 {
            self.inflight.lock().await.remove(&key);
        }
        Ok(value)
    }

    fn parse_projects(value: Value) -> anyhow::Result<(Vec<CfProject>, CfPagination)> {
        let envelope: Envelope<Vec<CfProject>> = serde_json::from_value(value)?;
        Ok((
            envelope.data,
            envelope.pagination.unwrap_or(CfPagination {
                index: 0,
                page_size: 0,
                total_count: 0,
            }),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        class: Option<&str>,
        query: &str,
        page: u64,
        page_size: u64,
        sort: &str,
        version: Option<&str>,
        category: Option<u64>,
    ) -> anyhow::Result<(Vec<CfProject>, CfPagination)> {
        let mut params: Vec<(&str, String)> = vec![
            ("gameId", GAME_ID_BEDROCK.to_string()),
            ("index", ((page.saturating_sub(1)) * page_size).to_string()),
            ("pageSize", page_size.to_string()),
            ("sortOrder", "desc".into()),
        ];
        if let Some(id) = class.and_then(class_id) {
            params.push(("classId", id.to_string()));
        }
        if !query.trim().is_empty() {
            params.push(("searchFilter", query.trim().to_string()));
        }
        if let Some(field) = sort_field(sort) {
            params.push(("sortField", field.to_string()));
        }
        if let Some(v) = version {
            params.push(("gameVersion", v.to_string()));
        }
        if let Some(id) = category {
            params.push(("categoryIds", format!("[{id}]")));
        }
        let value = self.get_json("/mods/search", &params, SEARCH_TTL).await?;
        Self::parse_projects(value)
    }

    pub async fn categories(&self) -> anyhow::Result<Vec<CfCategory>> {
        let value = self
            .get_json(
                "/categories",
                &[("gameId", GAME_ID_BEDROCK.to_string())],
                META_TTL,
            )
            .await?;
        let envelope: Envelope<Vec<CfCategory>> = serde_json::from_value(value)?;
        Ok(envelope.data)
    }

    /// Bedrock game versions, newest first (numeric-aware).
    pub async fn versions(&self) -> anyhow::Result<Vec<String>> {
        let value = self
            .get_json(
                &format!("/games/{GAME_ID_BEDROCK}/versions"),
                &[],
                VERSIONS_TTL,
            )
            .await?;
        let envelope: Envelope<Vec<serde_json::Value>> = serde_json::from_value(value)?;
        let mut versions = envelope
            .data
            .first()
            .and_then(|v| v.get("versions"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        versions.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
        Ok(versions)
    }

    pub async fn project(&self, id: u64) -> anyhow::Result<CfProject> {
        let value = self
            .get_json(&format!("/mods/{id}"), &[], PROJECT_TTL)
            .await?;
        let envelope: Envelope<CfProject> = serde_json::from_value(value)?;
        Ok(envelope.data)
    }

    /// Rich HTML description (untrusted content — sanitize client-side).
    pub async fn description(&self, id: u64) -> anyhow::Result<String> {
        let value = self
            .get_json(&format!("/mods/{id}/description"), &[], META_TTL)
            .await?;
        Ok(value
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub async fn files(
        &self,
        id: u64,
        page: u64,
        page_size: u64,
        version: Option<&str>,
    ) -> anyhow::Result<(Vec<CfFile>, u64)> {
        let mut params: Vec<(&str, String)> = vec![
            ("pageIndex", (page.saturating_sub(1)).to_string()),
            ("pageSize", page_size.to_string()),
        ];
        if let Some(v) = version {
            params.push(("gameVersion", v.to_string()));
        }
        let value = self
            .get_json(&format!("/mods/{id}/files"), &params, FILES_TTL)
            .await?;
        let envelope: Envelope<Vec<CfFile>> = serde_json::from_value(value)?;
        let total = envelope.pagination.as_ref().map_or(0, |p| p.total_count);
        Ok((envelope.data, total))
    }

    pub async fn file(&self, mod_id: u64, file_id: u64) -> anyhow::Result<CfFile> {
        let value = self
            .get_json(&format!("/mods/{mod_id}/files/{file_id}"), &[], FILES_TTL)
            .await?;
        let envelope: Envelope<CfFile> = serde_json::from_value(value)?;
        Ok(envelope.data)
    }

    /// Stream a CDN file to `dest`, enforcing the byte cap. Returns bytes written.
    pub async fn download_to(&self, url: &str, dest: &Path) -> anyhow::Result<u64> {
        use futures_util::StreamExt;
        let response = self
            .download_http
            .get(url)
            .header("x-api-key", self.current_key())
            .send()
            .await
            .context("CurseForge CDN request failed")?;
        let response = response
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("CurseForge CDN download failed: {e}"))?;
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = response.bytes_stream();
        let mut written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("CurseForge CDN download interrupted")?;
            written += chunk.len() as u64;
            if written > MAX_DOWNLOAD_BYTES {
                anyhow::bail!(
                    "download exceeds the {} MiB limit",
                    MAX_DOWNLOAD_BYTES / 1048576
                );
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(written)
    }

    /// Proxy a forgecdn image (logos, screenshots) for the Store UI.
    ///
    /// Thumbnails are small but numerous, and every page view used to
    /// re-download all of them; results are cached on disk under the panel
    /// data dir (survives restarts) and coalesced per URL, so repeat views
    /// are served straight from cache.
    pub async fn fetch_image(&self, url: &str) -> anyhow::Result<(Vec<u8>, String)> {
        let parsed = reqwest::Url::parse(url).context("invalid image URL")?;
        if !is_allowed_image_url(&parsed) {
            anyhow::bail!("only forgecdn.net images may be proxied");
        }
        let cache_path = self
            .image_cache_dir
            .as_ref()
            .map(|dir| dir.join(hex::encode(Sha256::digest(url.as_bytes()))));
        if let Some(path) = &cache_path
            && let Some((bytes, content_type)) = read_cached_image(path).await
        {
            return Ok((bytes, content_type.to_owned()));
        }
        // Coalesce concurrent fetches of the same image (grid + detail modal).
        let flight = self
            .inflight
            .lock()
            .await
            .entry(url.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _flight_guard = flight.lock().await;
        if let Some(path) = &cache_path
            && let Some((bytes, content_type)) = read_cached_image(path).await
        {
            return Ok((bytes, content_type.to_owned()));
        }
        let response = self
            .http
            .get(parsed)
            .timeout(IMAGE_FETCH_TIMEOUT)
            .send()
            .await
            .context("CurseForge CDN image request failed")?;
        let response = response
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("CurseForge CDN image fetch failed: {e}"))?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/png")
            .split(';')
            .next()
            .unwrap_or("image/png")
            .trim()
            .to_owned();
        if !content_type.starts_with("image/") {
            anyhow::bail!("URL is not an image");
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_IMAGE_BYTES)
        {
            anyhow::bail!(
                "image exceeds the {} MiB proxy limit",
                MAX_IMAGE_BYTES / 1048576
            );
        }
        let bytes = response
            .bytes()
            .await
            .context("CurseForge CDN image read failed")?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            anyhow::bail!(
                "image exceeds the {} MiB proxy limit",
                MAX_IMAGE_BYTES / 1048576
            );
        }
        if let Some(path) = cache_path {
            self.store_cached_image(&path, &content_type, &bytes).await;
        }
        Ok((bytes.to_vec(), content_type))
    }

    /// Persist a proxied image to the disk cache and keep the cache within
    /// its byte budget. Best-effort: cache failures never fail the request.
    pub(super) async fn store_cached_image(&self, path: &Path, content_type: &str, bytes: &[u8]) {
        let Some(ext) = image_ext(content_type) else {
            return;
        };
        let final_path = path.with_extension(ext);
        let tmp_path = path.with_extension(format!("{ext}.tmp"));
        if tokio::fs::write(&tmp_path, bytes).await.is_err() {
            // The cache dir may not exist yet (first use after a fresh data dir).
            if let Some(parent) = path.parent()
                && tokio::fs::create_dir_all(parent).await.is_ok()
                && tokio::fs::write(&tmp_path, bytes).await.is_ok()
            {
                let _ = tokio::fs::rename(&tmp_path, &final_path).await;
                self.account_cached_bytes(bytes.len() as u64).await;
            }
            return;
        }
        if tokio::fs::rename(&tmp_path, &final_path).await.is_ok() {
            self.account_cached_bytes(bytes.len() as u64).await;
        }
    }

    async fn account_cached_bytes(&self, added: u64) {
        let total = self.image_cache_bytes.fetch_add(added, Ordering::Relaxed) + added;
        if total > IMAGE_CACHE_MAX_BYTES {
            self.prune_image_cache().await;
        }
    }

    /// LRU-by-mtime eviction once the cache outgrows its budget. A single
    /// prune scans the whole dir, so concurrent requests trigger at most one.
    async fn prune_image_cache(&self) {
        let Some(dir) = self.image_cache_dir.as_ref() else {
            return;
        };
        if self
            .image_prune_running
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        let _guard = PruneGuard(self.image_prune_running.as_ref());
        let mut entries: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
        let mut total: u64 = 0;
        let mut reader = match tokio::fs::read_dir(dir).await {
            Ok(reader) => reader,
            Err(_) => {
                self.image_cache_bytes.store(0, Ordering::Relaxed);
                return;
            }
        };
        while let Ok(Some(entry)) = reader.next_entry().await {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            total += meta.len();
            entries.push((
                entry.path(),
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                meta.len(),
            ));
        }
        entries.sort_by_key(|(_, modified, _)| *modified);
        let target = IMAGE_CACHE_MAX_BYTES * 3 / 4;
        for (path, _, size) in &entries {
            if total <= target {
                break;
            }
            if tokio::fs::remove_file(path).await.is_ok() {
                total -= size;
            }
        }
        self.image_cache_bytes.store(total, Ordering::Relaxed);
    }
}

/// Numeric-aware sort key: "1.21.10" > "1.21.9", "26.50" > "1.21.132".
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// Exact-host allowlist for the image proxy. A bare suffix match would admit
/// attacker-registered lookalikes (`evil-forgecdn.net`), so only the CDN
/// apex and its direct subdomains pass.
fn is_allowed_image_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_proxy_host_allowlist_is_exact() {
        let ok = |u: &str| is_allowed_image_url(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://media.forgecdn.net/files/x.png"));
        assert!(ok("https://forgecdn.net/a.png"));
        // Lookalike hosts must not slip through a suffix match.
        assert!(!ok("https://evil-forgecdn.net/a.png"));
        assert!(!ok("https://notforgecdn.net/a.png"));
        assert!(!ok("http://media.forgecdn.net/a.png"));
        assert!(!ok("https://example.com/a.png"));
    }

    #[test]
    fn versions_sort_newest_first() {
        let mut versions = vec![
            "1.21.9".to_string(),
            "26.50".into(),
            "1.21.132".into(),
            "1.20".into(),
        ];
        versions.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
        assert_eq!(versions, vec!["26.50", "1.21.132", "1.21.9", "1.20"]);
    }

    #[test]
    fn project_parses_search_result() {
        let raw = serde_json::json!({
            "data": [{
                "id": 1575941u64,
                "name": "Verity - Bedrock Edition",
                "slug": "verity-bedrock-edition",
                "summary": "A little helper friend",
                "downloadCount": 14802919u64,
                "dateModified": "2026-09-17T09:13:39.243Z",
                "dateCreated": "2026-06-15T14:09:24.197Z",
                "authors": [{"id": 1u64, "name": "PnTMC", "url": "https://x"}],
                "logo": {"thumbnailUrl": "https://thumb", "url": "https://full"},
                "categories": [{"id": 8833u64, "name": "Horror"}],
                "links": {"websiteUrl": "https://www.curseforge.com/x"},
                "classId": 4984u64,
                "latestFilesIndexes": [{"gameVersion": "26.50", "fileId": 8902437u64, "filename": "Verity.mcaddon"}]
            }],
            "pagination": {"index": 0, "pageSize": 20, "totalCount": 10000}
        });
        let (projects, pagination) = CurseForge::parse_projects(raw).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].authors[0].name, "PnTMC");
        assert_eq!(
            projects[0].logo.as_ref().unwrap().thumbnail_url.as_deref(),
            Some("https://thumb")
        );
        assert_eq!(projects[0].latest_files_indexes[0].file_id, 8902437);
        assert_eq!(pagination.total_count, 10000);
    }

    #[test]
    fn file_parses_download_fields() {
        let raw = serde_json::json!({
            "data": {
                "id": 8902437u64,
                "modId": 1575941u64,
                "displayName": "Verity V26.50.mcaddon",
                "fileName": "Verity.mcaddon",
                "releaseType": 1u8,
                "fileDate": "2026-09-17T09:05:26.977Z",
                "fileLength": 37688517u64,
                "downloadCount": 15345u64,
                "downloadUrl": "https://edge.forgecdn.net/files/8902/437/Verity.mcaddon",
                "gameVersions": ["26.50"],
                "isAvailable": true,
                "dependencies": []
            }
        });
        let envelope: Envelope<CfFile> = serde_json::from_value(raw).unwrap();
        let file = envelope.data;
        assert_eq!(file.release_label(), "release");
        assert!(file.download_url.as_deref().unwrap().contains("forgecdn"));
        assert_eq!(file.game_versions, vec!["26.50"]);
    }

    #[tokio::test]
    async fn unconfigured_client_reports_missing_key() {
        let cf = CurseForge::new("", None);
        assert!(!cf.configured());
        let err = cf
            .search(Some("addons"), "", 1, 20, "relevancy", None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("CURSEFORGE_API_KEY"));
    }
}
