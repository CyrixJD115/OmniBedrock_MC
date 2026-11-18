//! Optional "pack CDN": let clients download the world's resource packs over
//! plain HTTP(S) instead of through the game connection.
//!
//! BDS (1.21.90+, verified in the 1.26.65 binary) reads an optional
//! `cdn_config.json` next to `server.properties` that maps
//! `"<pack uuid>_<version>"` to a URL of that pack as a zip. When EVERY
//! resource pack the world uses is covered, joining clients fetch the packs
//! from those URLs; if one is missing or stale BDS logs "Mismatch detected
//! between CDN config and required resource packs." and falls back to sending
//! them in-game. The format is not documented by Mojang (community-sourced),
//! so this is strictly opt-in.
//!
//! The panel builds deterministic zips (manifest at the zip root, as BDS and
//! the client expect), keeps them in `<cache>/pack-cdn/<bucket>/`, serves
//! them itself at `<base-url>/packs/<bucket>/<uuid>_<version>.zip` with an
//! exact `Content-Length` and `Content-Type: application/zip`, and writes
//! `cdn_config.json`. The bucket is a hash of the server directory, so the
//! public URL never names the instance or a filesystem path.
//!
//! This makes the download fast for consoles (an HTTP fetch instead of
//! chunked in-game transfer); it does not change how long a client keeps
//! the packs afterwards.

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Per-instance settings, stored beside server.properties.
pub const SETTINGS_FILE: &str = "omnibedrock_pack_cdn.json";
/// What BDS reads.
pub const CDN_CONFIG: &str = "cdn_config.json";
/// Marker holding the hash of the cdn_config.json the panel wrote, so the
/// panel never deletes a hand-made file.
const OWNED_MARKER: &str = ".omnibedrock_cdn_config.sha256";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CdnSettings {
    pub enabled: bool,
    /// Public origin (and optional path prefix) clients reach the panel at,
    /// e.g. `https://packs.example.com` or `http://203.0.113.7:17754`.
    #[serde(default)]
    pub base_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CdnFile {
    pub key: String,
    pub name: String,
    pub file: String,
    pub url: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Default)]
pub struct CdnBuild {
    pub world: String,
    pub files: Vec<CdnFile>,
    /// Order-file uuids with no pack folder (BDS would reject the config).
    pub missing: Vec<String>,
    pub total_bytes: u64,
    /// cdn_config.json written (only when every pack is covered).
    pub written: bool,
    /// Zips rebuilt this run (others were reused).
    pub rebuilt: usize,
}

pub fn load_settings(server_dir: &Path) -> CdnSettings {
    std::fs::read_to_string(server_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_settings(server_dir: &Path, settings: &CdnSettings) -> anyhow::Result<()> {
    write_atomic(
        &server_dir.join(SETTINGS_FILE),
        serde_json::to_string_pretty(settings)?.as_bytes(),
    )
}

/// Normalize and validate a base URL: http(s), a host, no query/fragment.
pub fn normalize_base_url(raw: &str) -> anyhow::Result<String> {
    let url = raw.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .context("the base URL must start with http:// or https://")?;
    if rest.is_empty() || rest.starts_with('/') {
        bail!("the base URL needs a host");
    }
    if url.contains(['?', '#', ' ', '"', '\\']) {
        bail!("the base URL cannot contain a query, fragment, spaces or quotes");
    }
    Ok(url.to_owned())
}

/// Public, opaque directory name for one server dir.
pub fn bucket(server_dir: &Path) -> String {
    let canonical = std::fs::canonicalize(server_dir).unwrap_or_else(|_| server_dir.to_path_buf());
    hex::encode(&Sha256::digest(canonical.to_string_lossy().as_bytes())[..8])
}

pub fn bucket_dir(cache_dir: &Path, bucket: &str) -> PathBuf {
    cache_dir.join("pack-cdn").join(bucket)
}

/// True for names the public route may serve: `<uuid>_<n.n.n>.zip`.
pub fn is_servable_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".zip") else {
        return false;
    };
    let Some((uuid, version)) = stem.split_once('_') else {
        return false;
    };
    uuid.len() == 36
        && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && !version.is_empty()
        && version.len() <= 32
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// True for a 16-hex-char bucket name.
pub fn is_bucket_name(name: &str) -> bool {
    name.len() == 16 && name.chars().all(|c| c.is_ascii_hexdigit())
}

/// `level-name` from server.properties.
pub fn active_world(server_dir: &Path) -> String {
    std::fs::read_to_string(server_dir.join("server.properties"))
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("level-name=")
                    .map(|v| v.trim().to_owned())
            })
        })
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| "Bedrock level".into())
}

fn version_string(value: &Value) -> Option<String> {
    match value {
        Value::Array(parts) => {
            let nums: Vec<String> = parts
                .iter()
                .filter_map(Value::as_u64)
                .map(|n| n.to_string())
                .collect();
            (!nums.is_empty()).then(|| nums.join("."))
        }
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        _ => None,
    }
}

struct Pack {
    dir: PathBuf,
    name: String,
    version: String,
}

fn resource_packs(world: &Path) -> HashMap<String, Pack> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(world.join("resource_packs")) else {
        return out;
    };
    for entry in entries.flatten().take(512) {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path().join("manifest.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let header = &manifest["header"];
        let (Some(uuid), Some(version)) =
            (header["uuid"].as_str(), version_string(&header["version"]))
        else {
            continue;
        };
        out.insert(
            uuid.to_ascii_lowercase(),
            Pack {
                dir: entry.path(),
                name: header["name"].as_str().unwrap_or_default().to_owned(),
                version,
            },
        );
    }
    out
}

fn active_order(world: &Path) -> Vec<String> {
    std::fs::read_to_string(world.join("world_resource_packs.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e["pack_id"].as_str().map(str::to_ascii_lowercase))
        .collect()
}

/// Cheap change fingerprint of a pack folder: file count, total size, newest mtime.
fn fingerprint(dir: &Path) -> String {
    let (mut count, mut size, mut newest) = (0u64, 0u64, 0u64);
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file()
            && let Ok(meta) = entry.metadata()
        {
            count += 1;
            size += meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            newest = newest.max(mtime);
        }
    }
    format!("{count}:{size}:{newest}")
}

/// Deterministic zip of a pack folder with manifest.json at the zip root.
fn zip_pack(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut files: Vec<(String, PathBuf)> = walkdir::WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e
                .path()
                .strip_prefix(src)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, e.path().to_path_buf()))
        })
        .collect();
    files.sort();
    if !files.iter().any(|(rel, _)| rel == "manifest.json") {
        bail!("{} has no top-level manifest.json", src.display());
    }
    let tmp = dest.with_extension("zip.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        for (rel, path) in files {
            // Already-compressed media gains nothing from deflate.
            let stored = matches!(
                rel.rsplit('.')
                    .next()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "ogg" | "fsb" | "zip")
            );
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(if stored {
                    zip::CompressionMethod::Stored
                } else {
                    zip::CompressionMethod::Deflated
                })
                .last_modified_time(zip::DateTime::default())
                .large_file(std::fs::metadata(&path)?.len() >= u32::MAX as u64);
            zip.start_file(rel, options)?;
            let mut input = std::fs::File::open(&path)?;
            std::io::copy(&mut input, &mut zip)?;
        }
        zip.finish()?.flush()?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp-omnibedrock");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Build (or refresh) the zips for the active world and write
/// cdn_config.json when every resource pack is covered.
pub fn build(
    server_dir: &Path,
    cache_dir: &Path,
    settings: &CdnSettings,
) -> anyhow::Result<CdnBuild> {
    let base = normalize_base_url(&settings.base_url)?;
    let world_name = active_world(server_dir);
    let world = server_dir.join("worlds").join(&world_name);
    anyhow::ensure!(world.is_dir(), "world '{world_name}' does not exist");
    let bucket = bucket(server_dir);
    let out_dir = bucket_dir(cache_dir, &bucket);
    std::fs::create_dir_all(&out_dir)?;
    let packs = resource_packs(&world);
    let mut report = CdnBuild {
        world: world_name,
        ..CdnBuild::default()
    };
    let mut mapping = BTreeMap::new();
    let mut keep = Vec::new();
    for uuid in active_order(&world) {
        let Some(pack) = packs.get(&uuid) else {
            report.missing.push(uuid);
            continue;
        };
        let key = format!("{uuid}_{}", pack.version);
        let file = format!("{key}.zip");
        let dest = out_dir.join(&file);
        let stamp_path = out_dir.join(format!("{key}.src"));
        let stamp = fingerprint(&pack.dir);
        let fresh = dest.exists() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp);
        if !fresh {
            zip_pack(&pack.dir, &dest)
                .with_context(|| format!("zipping {}", pack.dir.display()))?;
            std::fs::write(&stamp_path, &stamp)?;
            report.rebuilt += 1;
        }
        let bytes = std::fs::metadata(&dest)?.len();
        let url = format!("{base}/packs/{bucket}/{file}");
        mapping.insert(key.clone(), Value::String(url.clone()));
        keep.push(file.clone());
        keep.push(format!("{key}.src"));
        report.total_bytes += bytes;
        report.files.push(CdnFile {
            key,
            name: pack.name.clone(),
            file,
            url,
            bytes,
        });
    }
    // Drop zips for packs no longer in the stack.
    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !keep.contains(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    if report.missing.is_empty() && !report.files.is_empty() {
        let text = serde_json::to_string_pretty(&Value::Object(mapping.into_iter().collect()))?;
        write_atomic(&server_dir.join(CDN_CONFIG), text.as_bytes())?;
        std::fs::write(server_dir.join(OWNED_MARKER), digest_hex(text.as_bytes()))?;
        report.written = true;
    } else {
        // A partial config makes BDS ignore it anyway; do not leave one.
        remove_owned_config(server_dir);
    }
    Ok(report)
}

/// Remove cdn_config.json only if it is exactly what the panel wrote.
pub fn remove_owned_config(server_dir: &Path) -> bool {
    let config = server_dir.join(CDN_CONFIG);
    let marker = server_dir.join(OWNED_MARKER);
    let owned = match (std::fs::read(&config), std::fs::read_to_string(&marker)) {
        (Ok(bytes), Ok(hash)) => digest_hex(&bytes) == hash.trim(),
        _ => false,
    };
    if owned {
        let _ = std::fs::remove_file(&config);
    }
    let _ = std::fs::remove_file(&marker);
    owned
}

/// Turn the feature off: drop the panel's cdn_config.json and the zips.
pub fn disable(server_dir: &Path, cache_dir: &Path) {
    remove_owned_config(server_dir);
    let _ = std::fs::remove_dir_all(bucket_dir(cache_dir, &bucket(server_dir)));
}

/// Read-only view of the current state (no zipping).
#[derive(Debug, Serialize)]
pub struct CdnState {
    pub settings: CdnSettings,
    pub bucket: String,
    pub world: String,
    /// cdn_config.json exists next to server.properties.
    pub config_present: bool,
    /// ...and it is the one the panel wrote.
    pub config_owned: bool,
    /// Active resource packs (uuid_version) the config does NOT cover.
    pub uncovered: Vec<String>,
    pub files: usize,
    pub total_bytes: u64,
}

pub fn state(server_dir: &Path, cache_dir: &Path) -> CdnState {
    let settings = load_settings(server_dir);
    let world_name = active_world(server_dir);
    let world = server_dir.join("worlds").join(&world_name);
    let bucket = bucket(server_dir);
    let config_path = server_dir.join(CDN_CONFIG);
    let config_text = std::fs::read(&config_path).ok();
    let config_owned = config_text.as_ref().is_some_and(|bytes| {
        std::fs::read_to_string(server_dir.join(OWNED_MARKER))
            .is_ok_and(|h| h.trim() == digest_hex(bytes))
    });
    let covered: Vec<String> = config_text
        .as_ref()
        .and_then(|b| serde_json::from_slice::<serde_json::Map<String, Value>>(b).ok())
        .map(|m| m.keys().map(|k| k.to_ascii_lowercase()).collect())
        .unwrap_or_default();
    let packs = resource_packs(&world);
    let uncovered = active_order(&world)
        .into_iter()
        .map(|uuid| match packs.get(&uuid) {
            Some(p) => format!("{uuid}_{}", p.version),
            None => uuid,
        })
        .filter(|key| !covered.contains(key))
        .collect();
    let (mut files, mut total_bytes) = (0, 0);
    if let Ok(entries) = std::fs::read_dir(bucket_dir(cache_dir, &bucket)) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".zip") {
                files += 1;
                total_bytes += entry.metadata().map_or(0, |m| m.len());
            }
        }
    }
    CdnState {
        settings,
        bucket,
        world: world_name,
        config_present: config_text.is_some(),
        config_owned,
        uncovered,
        files,
        total_bytes,
    }
}

/// Result of fetching one CDN URL the way a client would.
#[derive(Debug, Serialize)]
pub struct SelfTest {
    pub url: String,
    pub ok: bool,
    pub status: Option<u16>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub expected_length: u64,
    pub detail: String,
}

/// HEAD-equivalent check of one served zip from the panel host: status 200,
/// `application/zip`, and a `Content-Length` that matches the file. A public
/// IP may fail here only because the router refuses hairpin NAT — the detail
/// says so instead of declaring the setup broken.
pub async fn self_test(url: &str, expected_length: u64) -> SelfTest {
    static HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("static HTTP client")
    });
    let mut result = SelfTest {
        url: url.to_owned(),
        ok: false,
        status: None,
        content_type: None,
        content_length: None,
        expected_length,
        detail: String::new(),
    };
    match HTTP.head(url).send().await {
        Ok(response) => {
            result.status = Some(response.status().as_u16());
            result.content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            result.content_length = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            let type_ok = result
                .content_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("application/zip"));
            let length_ok = result.content_length == Some(expected_length);
            result.ok = response.status().is_success() && type_ok && length_ok;
            result.detail = if result.ok {
                "reachable from the server with the headers BDS clients need".into()
            } else if !response.status().is_success() {
                format!("the URL answered HTTP {}", response.status().as_u16())
            } else if !type_ok {
                "wrong Content-Type (must be application/zip) — a proxy in front may be rewriting it".into()
            } else {
                "Content-Length missing or wrong — a proxy in front may be compressing or chunking it".into()
            };
        }
        Err(e) => {
            result.detail = if e.is_connect() || e.is_timeout() {
                "not reachable FROM THIS SERVER. If the base URL is your public IP, the router may simply refuse hairpin NAT — test it from a phone on mobile data before assuming it is broken".into()
            } else {
                format!("request failed: {e}")
            };
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn rp(world: &Path, folder: &str, uuid: &str, version: Value) {
        let dir = world.join("resource_packs").join(folder);
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({"format_version": 2, "header": {"name": folder, "uuid": uuid, "version": version}}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("textures/a.png"), vec![7u8; 4096]).unwrap();
    }

    #[test]
    fn builds_zips_and_a_complete_cdn_config() {
        let tmp = tempfile::tempdir().unwrap();
        let server = tmp.path().join("server");
        let cache = tmp.path().join("cache");
        let world = server.join("worlds").join("W");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(server.join("server.properties"), "level-name=W\n").unwrap();
        let a = "aaaaaaaa-1111-2222-3333-444444444444";
        let b = "bbbbbbbb-1111-2222-3333-444444444444";
        rp(&world, "pa", a, serde_json::json!([1, 0, 86]));
        rp(&world, "pb", b, serde_json::json!("2.1.0"));
        std::fs::write(
            world.join("world_resource_packs.json"),
            format!(
                r#"[{{"pack_id":"{a}","version":[1,0,86]}},{{"pack_id":"{b}","version":[2,1,0]}}]"#
            ),
        )
        .unwrap();
        let settings = CdnSettings {
            enabled: true,
            base_url: "https://packs.example.com/".into(),
        };
        let report = build(&server, &cache, &settings).unwrap();
        assert!(report.written);
        assert_eq!(report.rebuilt, 2);
        let config: serde_json::Map<String, Value> =
            serde_json::from_str(&std::fs::read_to_string(server.join(CDN_CONFIG)).unwrap())
                .unwrap();
        let bucket = bucket(&server);
        assert_eq!(
            config[&format!("{a}_1.0.86")],
            format!("https://packs.example.com/packs/{bucket}/{a}_1.0.86.zip")
        );
        assert!(config.contains_key(&format!("{b}_2.1.0")));
        // manifest.json sits at the zip root.
        let zip_path = bucket_dir(&cache, &bucket).join(format!("{a}_1.0.86.zip"));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let mut manifest = String::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        assert!(manifest.contains(a));
        assert!(archive.by_name("textures/a.png").is_ok());
        // Unchanged packs are reused, and the build is byte-identical.
        let before = std::fs::read(&zip_path).unwrap();
        assert_eq!(build(&server, &cache, &settings).unwrap().rebuilt, 0);
        assert_eq!(before, std::fs::read(&zip_path).unwrap());
        let st = state(&server, &cache);
        assert!(st.config_present && st.config_owned && st.uncovered.is_empty());
        assert_eq!(st.files, 2);
        // Disabling removes only what the panel wrote.
        disable(&server, &cache);
        assert!(!server.join(CDN_CONFIG).exists());
        assert!(!bucket_dir(&cache, &bucket).exists());
    }

    #[test]
    fn missing_pack_means_no_config_and_hand_written_files_survive() {
        let tmp = tempfile::tempdir().unwrap();
        let server = tmp.path().join("server");
        let world = server.join("worlds").join("W");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(server.join("server.properties"), "level-name=W\n").unwrap();
        std::fs::write(
            world.join("world_resource_packs.json"),
            r#"[{"pack_id":"cccccccc-1111-2222-3333-444444444444","version":[1,0,0]}]"#,
        )
        .unwrap();
        std::fs::write(server.join(CDN_CONFIG), r#"{"hand":"made"}"#).unwrap();
        let settings = CdnSettings {
            enabled: true,
            base_url: "http://203.0.113.7:17754".into(),
        };
        let report = build(&server, &tmp.path().join("cache"), &settings).unwrap();
        assert!(!report.written);
        assert_eq!(report.missing.len(), 1);
        assert!(
            server.join(CDN_CONFIG).exists(),
            "a hand-made config is never deleted"
        );
    }

    #[test]
    fn names_and_urls_are_validated() {
        assert!(is_servable_name(
            "aaaaaaaa-1111-2222-3333-444444444444_1.0.86.zip"
        ));
        assert!(!is_servable_name("../etc/passwd"));
        assert!(!is_servable_name(
            "aaaaaaaa-1111-2222-3333-444444444444_1.0.86.zip.src"
        ));
        assert!(is_bucket_name("0123456789abcdef"));
        assert!(!is_bucket_name("../../x"));
        assert_eq!(
            normalize_base_url("https://x.example/").unwrap(),
            "https://x.example"
        );
        assert!(normalize_base_url("ftp://x").is_err());
        assert!(normalize_base_url("https://x?y=1").is_err());
        assert!(normalize_base_url("http://").is_err());
    }
}
