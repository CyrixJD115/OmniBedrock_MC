//! Addon archive installation and metadata.
//!
//! Handles `.mcpack` (single pack), `.mcaddon` (multiple packs), and plain
//! `.zip` uploads. Manifests are detected at the archive root or one folder
//! deep; behavior vs resource is decided from module types. Extraction is
//! staged per pack and moved into place with a rename, refusing symlink and
//! traversal entries.
use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Parse a Bedrock `.lang` file: `key=value` lines, `#` comment lines, and
/// blank lines. Later duplicate keys win, matching the game's behavior.
/// Marketplace packs end values with a whitespace-then-`#` comment marker
/// (`…Pack\t#`); a `#` preceded by whitespace starts a trailing comment and
/// is stripped, while a glued `#` stays part of the value.
pub fn parse_lang(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = match value.find('#') {
                Some(idx) if idx > 0 && value[..idx].ends_with(char::is_whitespace) => {
                    &value[..idx]
                }
                _ => value,
            };
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// Read a pack's localization table from `texts/en_US.lang`, falling back to
/// the first `*.lang` in sorted order when en_US is absent. Marketplace-style
/// packs put localization keys like `pack.name` in the manifest and the real
/// strings here. Empty map when the pack ships no texts.
pub async fn read_texts(pack_dir: &Path) -> HashMap<String, String> {
    let texts = pack_dir.join("texts");
    let mut candidate = texts.join("en_US.lang");
    if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&texts).await {
            while let Ok(entry) = entries.next_entry().await {
                let Some(entry) = entry else { break };
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "lang") {
                    files.push(path);
                }
            }
        }
        files.sort();
        match files.first() {
            Some(path) => candidate = path.clone(),
            None => return HashMap::new(),
        }
    }
    match tokio::fs::read_to_string(&candidate).await {
        Ok(content) => parse_lang(&content),
        Err(_) => HashMap::new(),
    }
}

/// Resolve a manifest string against the pack's lang table: keys like
/// `pack.name` become their translation, plain strings pass through.
pub fn resolve_lang(texts: &HashMap<String, String>, raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    Some(texts.get(raw).cloned().unwrap_or_else(|| raw.to_string()))
}

const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_PACKS: usize = 64;

#[derive(Debug, serde::Serialize)]
pub struct InstalledPack {
    pub name: String,
    #[serde(rename = "pack_type")]
    pub pack_type: &'static str,
    pub uuid: String,
    pub version: Vec<u64>,
}

/// Map manifest module types to a pack directory. Behavior wins when mixed.
pub fn detect_pack_type(manifest: &Value) -> Option<&'static str> {
    let modules = manifest.get("modules")?.as_array()?;
    let mut behavior = false;
    let mut resource = false;
    for module in modules {
        match module.get("type").and_then(Value::as_str) {
            Some("resources") => resource = true,
            Some("data" | "script" | "javascript") => behavior = true,
            _ => {}
        }
    }
    if behavior {
        Some("behavior_packs")
    } else if resource {
        Some("resource_packs")
    } else {
        None
    }
}

fn header_string(manifest: &Value, key: &str) -> Option<String> {
    manifest
        .get("header")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}
fn header_uuid(manifest: &Value) -> String {
    header_string(manifest, "uuid").unwrap_or_default()
}
fn header_version(manifest: &Value) -> Vec<u64> {
    manifest
        .get("header")
        .and_then(|h| h.get("version"))
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

/// Sanitize a user-controlled pack folder name into a single safe component.
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._-'()[]".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.']).trim();
    let name: String = trimmed.chars().take(64).collect();
    if name.is_empty() {
        "pack".to_string()
    } else {
        name
    }
}

/// Pick a free directory name next to `target`, appending -1, -2, … on conflicts.
fn free_name(parent: &Path, base: &str) -> PathBuf {
    let mut candidate = parent.join(base);
    if !candidate.exists() {
        return candidate;
    }
    for index in 1..1000u32 {
        candidate = parent.join(format!("{base}-{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{}-{}", base, Uuid::new_v4().simple()))
}

struct SourcePack {
    /// Archive entry prefix holding the pack ("", "SomePack/", …).
    prefix: String,
    manifest: Value,
}

/// Locate manifests at the archive root or exactly one folder deep.
fn find_source_packs(
    archive: &mut ZipArchive<impl Read + Seek>,
) -> anyhow::Result<Vec<SourcePack>> {
    if archive.len() > MAX_ENTRIES {
        anyhow::bail!("archive has too many entries");
    }
    let mut root: Option<Value> = None;
    let mut nested: Vec<(String, Value)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if entry.is_dir() || !name.ends_with("manifest.json") {
            continue;
        }
        let depth = name.split('/').count();
        if depth > 2 {
            continue;
        }
        let text = read_limited(entry, MAX_ENTRY_BYTES)?;
        let manifest: Value = match serde_json::from_slice::<Value>(&text) {
            Ok(v) if v.get("header").is_some() => v,
            _ => continue,
        };
        match name.split_once('/') {
            None => root = Some(manifest),
            Some((folder, _)) => {
                if !nested.iter().any(|(f, _)| f == folder) {
                    nested.push((format!("{folder}/"), manifest));
                }
            }
        }
    }
    if nested.len() > MAX_PACKS {
        anyhow::bail!("archive contains too many packs");
    }
    let mut packs: Vec<SourcePack> = nested
        .into_iter()
        .map(|(prefix, manifest)| SourcePack { prefix, manifest })
        .collect();
    match root {
        Some(manifest) => {
            // A root manifest makes this a single-pack archive; nested folders
            // (if any) would not be loaded by BDS in that layout, so ignore them.
            packs.clear();
            packs.push(SourcePack {
                prefix: String::new(),
                manifest,
            });
        }
        None if packs.is_empty() => anyhow::bail!("no manifest.json found in archive"),
        None => {}
    }
    Ok(packs)
}

fn read_limited(reader: impl Read, cap: u64) -> anyhow::Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(cap).read_to_end(&mut data)?;
    Ok(data)
}

/// Copy every archive entry under `prefix` into `dest`, enforcing safety caps.
fn extract_prefix(
    archive: &mut ZipArchive<impl Read + Seek>,
    prefix: &str,
    dest: &Path,
    totals: &mut u64,
) -> anyhow::Result<()> {
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if !entry.name().starts_with(prefix) {
            continue;
        }
        let relative = entry
            .enclosed_name()
            .with_context(|| format!("unsafe archive entry: {}", entry.name()))?;
        let relative = relative
            .strip_prefix(Path::new(prefix))
            .with_context(|| format!("entry escapes pack prefix: {}", entry.name()))?;
        let target = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if entry.is_symlink() {
            anyhow::bail!("archive entry is not a regular file: {}", entry.name());
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        *totals += entry.size();
        if *totals > MAX_TOTAL_BYTES {
            anyhow::bail!(
                "archive exceeds the {} MiB install budget",
                MAX_TOTAL_BYTES / 1048576
            );
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry.take(MAX_ENTRY_BYTES), &mut out)?;
    }
    Ok(())
}
use anyhow::Context as _;
use std::io::Seek;

/// Install every pack found in `archive_path` under `worlds_root/<world>/`.
/// Returns the installed packs with their final directory names.
pub fn install_archive(
    archive_path: &Path,
    worlds_root: &Path,
    world: &str,
    forced_type: Option<&str>,
) -> anyhow::Result<Vec<InstalledPack>> {
    let world_dir = worlds_root.join(world);
    std::fs::create_dir_all(&world_dir)?;
    let file = std::fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).context("read addon archive")?;
    let sources = find_source_packs(&mut archive)?;
    let mut installed = Vec::new();
    for source in sources {
        let pack_type: &'static str = match forced_type {
            Some("resource_packs") => "resource_packs",
            Some("behavior_packs") => "behavior_packs",
            Some(other) => anyhow::bail!("invalid pack type '{other}'"),
            None => detect_pack_type(&source.manifest).with_context(|| {
                format!(
                    "pack '{}' has no behavior or resource module",
                    header_string(&source.manifest, "name").unwrap_or_default()
                )
            })?,
        };
        let base = sanitize_name(
            &header_string(&source.manifest, "name")
                .or_else(|| {
                    source
                        .prefix
                        .trim_end_matches('/')
                        .split('/')
                        .next_back()
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "pack".into()),
        );
        let type_dir = world_dir.join(pack_type);
        std::fs::create_dir_all(&type_dir)?;
        let staging = type_dir.join(format!(".upload-{}", Uuid::new_v4()));
        let result = (|| {
            std::fs::create_dir_all(&staging)?;
            let mut totals = 0u64;
            extract_prefix(&mut archive, &source.prefix, &staging, &mut totals)?;
            if !staging.join("manifest.json").exists() {
                anyhow::bail!("extracted pack is missing manifest.json");
            }
            let final_dir = free_name(&type_dir, &base);
            std::fs::rename(&staging, &final_dir)?;
            Ok::<_, anyhow::Error>(final_dir)
        })();
        match result {
            Ok(final_dir) => installed.push(InstalledPack {
                name: final_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                pack_type,
                uuid: header_uuid(&source.manifest),
                version: header_version(&source.manifest),
            }),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e.context("install addon pack"));
            }
        }
    }
    Ok(installed)
}

/// Panel metadata sidecar stored inside each installed pack or world
/// directory, recording where the content came from (CurseForge Store vs a
/// manual upload) and when. Directories without one predate the Store.
pub const METADATA_FILE: &str = ".omnibedrock.json";

/// Read the sidecar if present. `None` means an untagged (treat as manual) pack.
pub fn read_pack_metadata(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join(METADATA_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the provenance sidecar into a pack or world directory.
pub async fn write_pack_metadata(dir: &Path, metadata: Value) -> anyhow::Result<()> {
    let path = dir.join(METADATA_FILE);
    omnibedrock_core::files::atomic_write(&path, &serde_json::to_vec_pretty(&metadata)?).await
}

/// What an uploaded archive holds, decided from its entry names.
pub enum ArchiveKind {
    /// `.mcaddon`/`.mcpack`/`.zip` holding one or more `manifest.json` packs.
    Addon,
    /// `.mcworld`/`.mctemplate` holding a `level.dat` world.
    World,
    Unknown,
}

/// Classify an archive. A pack manifest wins over a world level.dat, matching
/// how the manual upload flow treats bundles.
pub fn detect_archive(archive_path: &Path) -> anyhow::Result<ArchiveKind> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).context("read archive")?;
    let mut addon = false;
    let mut world = false;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name();
        if name.ends_with("manifest.json") && name.split('/').count() <= 2 {
            addon = true;
            break;
        }
        if name.ends_with("level.dat") && name.split('/').count() <= 2 {
            world = true;
        }
    }
    if addon {
        Ok(ArchiveKind::Addon)
    } else if world {
        Ok(ArchiveKind::World)
    } else {
        Ok(ArchiveKind::Unknown)
    }
}

/// Install a `.mcworld`/`.mctemplate` archive as a new world directory under
/// `worlds_root`. The directory name comes from `levelname.txt` unless forced;
/// name conflicts are suffixed like packs. Entry paths may sit at the archive
/// root or inside a single wrapper folder (both appear in the wild).
pub fn install_world(
    archive_path: &Path,
    worlds_root: &Path,
    forced_name: Option<&str>,
) -> anyhow::Result<InstalledPack> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).context("read world archive")?;
    let mut prefix = String::new();
    let mut wrapper: Option<String> = None;
    let mut mixed = false;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        match entry.name().split_once('/') {
            Some((folder, _)) => match &wrapper {
                Some(existing) if existing != folder => mixed = true,
                _ => wrapper = Some(folder.to_string()),
            },
            None => mixed = true,
        }
        if mixed {
            break;
        }
    }
    if !mixed && let Some(folder) = wrapper {
        prefix = format!("{folder}/");
    }
    let staging = worlds_root.join(format!(".upload-{}", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<String> {
        std::fs::create_dir_all(&staging)?;
        let mut totals = 0u64;
        extract_prefix(&mut archive, &prefix, &staging, &mut totals)?;
        if !staging.join("level.dat").exists() {
            anyhow::bail!("world archive is missing level.dat");
        }
        let name = match forced_name {
            Some(name) => sanitize_name(name),
            None => {
                let level_name =
                    std::fs::read_to_string(staging.join("levelname.txt")).unwrap_or_default();
                if level_name.trim().is_empty() {
                    "world".to_string()
                } else {
                    sanitize_name(level_name.trim())
                }
            }
        };
        let final_dir = free_name(worlds_root, &name);
        std::fs::rename(&staging, &final_dir)?;
        Ok(final_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default())
    })();
    match result {
        Ok(name) => Ok(InstalledPack {
            name,
            pack_type: "world",
            uuid: String::new(),
            version: Vec::new(),
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e.context("install world archive"))
        }
    }
}

/// A pack folder found inside an imported world.
struct WorldPack {
    version: Vec<u64>,
    /// `dependencies[].uuid` entries from the manifest.
    dependencies: Vec<String>,
}

/// Map every installed pack uuid in a world to its manifest-declared version
/// and dependencies. Bounded so a pathological world cannot stall a scan.
fn installed_world_packs(world_dir: &Path) -> HashMap<String, WorldPack> {
    let mut packs = HashMap::new();
    for pack_type in ["behavior_packs", "resource_packs"] {
        let Ok(entries) = std::fs::read_dir(world_dir.join(pack_type)) else {
            continue;
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
            let Some(uuid) = header["uuid"].as_str() else {
                continue;
            };
            let dependencies = manifest["dependencies"]
                .as_array()
                .map(|deps| {
                    deps.iter()
                        .filter_map(|d| d["uuid"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            packs.insert(
                uuid.to_string(),
                WorldPack {
                    version: header["version"]
                        .as_array()
                        .map(|v| v.iter().filter_map(Value::as_u64).collect())
                        .unwrap_or_default(),
                    dependencies,
                },
            );
        }
    }
    packs
}

/// Referenced pack uuids in discovery order: pack-order files first, then the
/// dependencies every installed manifest declares.
fn referenced_uuids(world_dir: &Path, packs: &HashMap<String, WorldPack>) -> Vec<String> {
    let mut referenced: Vec<String> = Vec::new();
    let push = |uuid: &str, referenced: &mut Vec<String>| {
        if !referenced.iter().any(|u| u == uuid) {
            referenced.push(uuid.to_string());
        }
    };
    for order_file in ["world_behavior_packs.json", "world_resource_packs.json"] {
        let Ok(text) = std::fs::read_to_string(world_dir.join(order_file)) else {
            continue;
        };
        let Ok(entries) = serde_json::from_str::<Vec<Value>>(&text) else {
            continue;
        };
        for entry in entries.iter().take(4096) {
            if let Some(uuid) = entry["pack_id"].as_str() {
                push(uuid, &mut referenced);
            }
        }
    }
    for info in packs.values() {
        for dep in &info.dependencies {
            push(dep, &mut referenced);
        }
    }
    referenced
}

/// What syncing an imported world's pack orders found.
#[derive(Debug, Default)]
pub struct PackSyncReport {
    /// Order-file entries rewritten to the installed manifest's version.
    pub synced: usize,
    /// UUIDs referenced by the order files or manifest dependencies that have
    /// no pack folder on disk, in discovery order.
    pub missing: Vec<String>,
}

/// Sync `world_behavior_packs.json` / `world_resource_packs.json` to the
/// versions the world's installed packs actually declare, and report
/// referenced-but-missing uuids. BDS resolves packs by (uuid, exact version),
/// so a template pinning stale versions would otherwise silently drop part of
/// its stack at boot ("was not found and was ignored"). Unparseable or absent
/// order files are left untouched.
pub fn sync_world_packs(world_dir: &Path) -> PackSyncReport {
    let packs = installed_world_packs(world_dir);
    let mut report = PackSyncReport::default();
    for order_file in ["world_behavior_packs.json", "world_resource_packs.json"] {
        let path = world_dir.join(order_file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut entries) = serde_json::from_str::<Vec<Value>>(&text) else {
            continue;
        };
        let mut changed = false;
        for entry in entries.iter_mut() {
            let Some(uuid) = entry["pack_id"].as_str() else {
                continue;
            };
            let Some(info) = packs.get(uuid) else {
                continue;
            };
            let current = serde_json::to_value(&info.version).unwrap_or(Value::Null);
            if entry.get("version") != Some(&current) {
                entry["version"] = current;
                changed = true;
                report.synced += 1;
            }
        }
        if changed && let Ok(text) = serde_json::to_string_pretty(&entries) {
            let _ = std::fs::write(&path, text);
        }
    }
    let referenced = referenced_uuids(world_dir, &packs);
    report.missing = referenced
        .into_iter()
        .filter(|uuid| !packs.contains_key(uuid))
        .collect();
    report
}

/// Missing pack uuids across every world of an instance, as
/// `"<world>: missing pack <uuid>"` strings. Read-only: backs the live
/// per-instance warning field without rewriting anything.
pub fn missing_dependencies(worlds_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(worlds) = std::fs::read_dir(worlds_root) else {
        return out;
    };
    for world in worlds.flatten().take(64) {
        if !world.file_type().is_ok_and(|t| t.is_dir())
            || world.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let packs = installed_world_packs(&world.path());
        for uuid in referenced_uuids(&world.path(), &packs) {
            if !packs.contains_key(&uuid) {
                out.push(format!(
                    "{}: missing pack {}",
                    world.file_name().to_string_lossy(),
                    uuid
                ));
            }
        }
    }
    out
}

/// Sync every world under `worlds_root` (healing imports that predate the
/// sync) and return one `"world: missing pack <uuid>"` entry per still-absent
/// reference, so the caller can emit a single console line.
pub fn sync_all_worlds(worlds_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(worlds) = std::fs::read_dir(worlds_root) else {
        return out;
    };
    for world in worlds.flatten().take(64) {
        if !world.file_type().is_ok_and(|t| t.is_dir())
            || world.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let report = sync_world_packs(&world.path());
        for uuid in report.missing {
            out.push(format!(
                "{}: missing pack {}",
                world.file_name().to_string_lossy(),
                uuid
            ));
        }
    }
    out
}

/// Total size, newest modification time, and icon presence for a pack folder.
/// Walks are bounded so a huge pack cannot stall the listing.
pub fn pack_stats(dir: &Path) -> (u64, u64, bool) {
    let mut size = 0u64;
    let mut modified = 0u64;
    let mut icon = false;
    let mut entries = 0usize;
    for entry in WalkDir::new(dir).follow_links(false) {
        let Ok(entry) = entry else { continue };
        entries += 1;
        if entries > 50_000 {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "pack_icon.png" || name == "pack_icon.jpg" {
            icon = true;
        }
        if let Ok(meta) = entry.metadata() {
            size += meta.len();
            if let Ok(mtime) = meta.modified()
                && let Ok(secs) = mtime.duration_since(std::time::UNIX_EPOCH)
            {
                modified = modified.max(secs.as_secs());
            }
        }
    }
    (size, modified, icon)
}

/// Remove a pack directory and strip its UUID from the world's pack order
/// files so BDS never references a missing pack.
pub async fn delete_pack(pack_dir: &Path, world_dir: &Path, uuid: &str) -> anyhow::Result<()> {
    tokio::fs::remove_dir_all(pack_dir).await?;
    if uuid.is_empty() {
        return Ok(());
    }
    for order_file in ["world_behavior_packs.json", "world_resource_packs.json"] {
        let path = world_dir.join(order_file);
        let Ok(text) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        let Ok(mut entries) = serde_json::from_str::<Vec<Value>>(&text) else {
            continue;
        };
        let before = entries.len();
        entries.retain(|e| e.get("pack_id").and_then(Value::as_str) != Some(uuid));
        if entries.len() != before {
            omnibedrock_core::files::atomic_write(
                &path,
                serde_json::to_string_pretty(&entries)?.as_bytes(),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    const BP_MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Cool Behavior","uuid":"11111111-1111-1111-1111-111111111111","version":[1,2,3]},"modules":[{"type":"data","uuid":"22222222-2222-2222-2222-222222222222","version":[1,2,3]}]}"#;
    const RP_MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Cool Resource","uuid":"33333333-3333-3333-3333-333333333333","version":[0,1,0]},"modules":[{"type":"resources","uuid":"44444444-4444-4444-4444-444444444444","version":[0,1,0]}]}"#;
    // Marketplace-style pack: lang-key name, subpacks, capabilities, metadata.
    const MARKETPLACE_RP_MANIFEST: &str = r#"{"format_version":2,"header":{"description":"pack.description","name":"pack.name","uuid":"985f7757-dbea-4f92-94dc-3ce02d14d83b","version":[1,0,86],"min_engine_version":[1,21,120]},"modules":[{"type":"resources","uuid":"2be875b6-3001-408a-a65c-3435d32b6f6c","version":[1,0,86]}],"subpacks":[{"folder_name":"a","name":"128x","memory_tier":12},{"folder_name":"d","name":"16x","memory_tier":2}],"capabilities":["pbr"],"metadata":{"authors":["Spark Universe"]}}"#;
    const ICON: &[u8] = b"\x89PNG\r\n\x1a\nfakeicon";

    #[test]
    fn mcpack_installs_with_type_and_icon() {
        let dir = tempfile::TempDir::new().unwrap();
        let archive = dir.path().join("pack.mcpack");
        write_zip(
            &archive,
            &[
                ("manifest.json", BP_MANIFEST.as_bytes()),
                ("pack_icon.png", ICON),
                ("scripts/main.js", b"console.log(1)"),
            ],
        );
        let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].pack_type, "behavior_packs");
        assert_eq!(installed[0].name, "Cool Behavior");
        assert_eq!(installed[0].version, vec![1, 2, 3]);
        let pack = dir.path().join("w/behavior_packs/Cool Behavior");
        assert!(pack.join("manifest.json").exists());
        assert!(pack.join("scripts/main.js").exists());
        assert!(pack_stats(&pack).2);
    }

    #[tokio::test]
    async fn marketplace_manifest_with_subpacks_installs() {
        // Root-manifest resource pack with lang-key name, subpack folders,
        // capabilities and metadata must install with everything intact.
        let dir = tempfile::TempDir::new().unwrap();
        let archive = dir.path().join("marketplace.mcpack");
        write_zip(
            &archive,
            &[
                ("manifest.json", MARKETPLACE_RP_MANIFEST.as_bytes()),
                ("pack_icon.png", ICON),
                (
                    "texts/en_US.lang",
                    b"pack.name=RealismCraft 2.6\t#\npack.description=Tiered textures\t#\n",
                ),
                ("subpacks/a/textures/a.png", b"a"),
                ("subpacks/d/textures/d.png", b"d"),
            ],
        );
        let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].pack_type, "resource_packs");
        // Lang-key name falls back to the archive folder ("pack"), and the
        // panel resolves the real name later from texts/en_US.lang.
        let pack = dir
            .path()
            .join("w/resource_packs")
            .read_dir()
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(installed[0].uuid, "985f7757-dbea-4f92-94dc-3ce02d14d83b");
        assert_eq!(installed[0].version, vec![1, 0, 86]);
        assert!(pack.join("subpacks/a/textures/a.png").exists());
        assert!(pack.join("subpacks/d/textures/d.png").exists());
        // The localization table resolves both keys; marketplace-style
        // trailing `\t#` comments are stripped from the values.
        let texts = read_texts(&pack).await;
        assert_eq!(
            resolve_lang(&texts, Some("pack.name")).as_deref(),
            Some("RealismCraft 2.6")
        );
        assert_eq!(
            resolve_lang(&texts, Some("pack.description")).as_deref(),
            Some("Tiered textures")
        );
    }

    #[test]
    fn lang_values_strip_comment_markers_but_keep_glued_hashes() {
        let langs =
            parse_lang("a=clean\t#\nb=two words # tail\nc=C# glued\nd=no hash\n# comment\n");
        assert_eq!(langs["a"], "clean");
        assert_eq!(langs["b"], "two words");
        // `#` glued to text stays part of the value.
        assert_eq!(langs["c"], "C# glued");
        assert_eq!(langs["d"], "no hash");
    }

    #[test]
    fn mcaddon_installs_both_types_and_conflicts_suffix() {
        let dir = tempfile::TempDir::new().unwrap();
        let archive = dir.path().join("bundle.mcaddon");
        write_zip(
            &archive,
            &[
                ("bp/manifest.json", BP_MANIFEST.as_bytes()),
                ("rp/manifest.json", RP_MANIFEST.as_bytes()),
            ],
        );
        let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
        assert_eq!(installed.len(), 2);
        assert!(
            dir.path()
                .join("w/behavior_packs/Cool Behavior/manifest.json")
                .exists()
        );
        assert!(
            dir.path()
                .join("w/resource_packs/Cool Resource/manifest.json")
                .exists()
        );
        // Same archive again: names collide and are suffixed, nothing is lost.
        let again = install_archive(&archive, dir.path(), "w", None).unwrap();
        assert_eq!(again.len(), 2);
        assert_ne!(again[0].name, installed[0].name);
    }

    #[test]
    fn traversal_and_missing_manifest_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let evil = dir.path().join("evil.zip");
        {
            let file = std::fs::File::create(&evil).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"nope").unwrap();
            zip.finish().unwrap();
        }
        assert!(install_archive(&evil, dir.path(), "w", None).is_err());
        assert!(!dir.path().join("escape.txt").exists());

        let bare = dir.path().join("bare.zip");
        write_zip(&bare, &[("readme.txt", b"no manifest here")]);
        assert!(
            install_archive(&bare, dir.path(), "w", None)
                .unwrap_err()
                .to_string()
                .contains("no manifest.json")
        );
    }

    #[test]
    fn sync_world_packs_rewrites_stale_versions_and_reports_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let world = dir.path().join("w");
        let bp = world.join("behavior_packs");
        std::fs::create_dir_all(bp.join("Alpha")).unwrap();
        std::fs::create_dir_all(bp.join("Beta")).unwrap();
        std::fs::create_dir_all(world.join("resource_packs/Gamma")).unwrap();
        // Alpha declares version [1,1,6] and depends on a pack missing on disk.
        std::fs::write(
            bp.join("Alpha/manifest.json"),
            r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000001","version":[1,1,6]},
                "dependencies":[{"uuid":"aaaaaaaa-0000-0000-0000-0000000000ff","version":[1,0,0]}]}"#,
        )
        .unwrap();
        std::fs::write(
            bp.join("Beta/manifest.json"),
            r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000002","version":[2,2,2]}}"#,
        )
        .unwrap();
        std::fs::write(
            world.join("resource_packs/Gamma/manifest.json"),
            r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000003","version":[1,0,0]}}"#,
        )
        .unwrap();
        // Marketplace-template style: every order entry pinned to [1,0,0]
        // while the real manifests differ; the missing pack has no folder.
        std::fs::write(
            world.join("world_behavior_packs.json"),
            r#"[{"pack_id":"aaaaaaaa-0000-0000-0000-000000000001","version":[1,0,0]},
                {"pack_id":"aaaaaaaa-0000-0000-0000-000000000002","version":[1,0,0]},
                {"pack_id":"aaaaaaaa-0000-0000-0000-0000000000ff","version":[1,0,0]}]"#,
        )
        .unwrap();
        std::fs::write(
            world.join("world_resource_packs.json"),
            r#"[{"pack_id":"aaaaaaaa-0000-0000-0000-000000000003","version":[1,0,0]}]"#,
        )
        .unwrap();

        let report = sync_world_packs(&world);
        assert_eq!(report.synced, 2, "Alpha and Beta order entries rewritten");
        assert_eq!(report.missing, vec!["aaaaaaaa-0000-0000-0000-0000000000ff"]);
        let orders: Value = serde_json::from_str(
            &std::fs::read_to_string(world.join("world_behavior_packs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(orders[0]["version"], serde_json::json!([1, 1, 6]));
        assert_eq!(orders[1]["version"], serde_json::json!([2, 2, 2]));
        // The missing pack's entry is reported, not silently dropped.
        assert_eq!(orders[2]["pack_id"], "aaaaaaaa-0000-0000-0000-0000000000ff");
        // Idempotent: a second pass finds nothing left to fix.
        let again = sync_world_packs(&world);
        assert_eq!(again.synced, 0);
        assert_eq!(again.missing, vec!["aaaaaaaa-0000-0000-0000-0000000000ff"]);
        // The instance-wide, read-only view reports the same gap.
        assert_eq!(
            missing_dependencies(dir.path()),
            vec!["w: missing pack aaaaaaaa-0000-0000-0000-0000000000ff"]
        );
    }

    #[test]
    fn sync_world_packs_tolerates_missing_order_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let world = dir.path().join("plain");
        std::fs::create_dir_all(&world).unwrap();
        let report = sync_world_packs(&world);
        assert_eq!(report.synced, 0);
        assert!(report.missing.is_empty());
        assert!(missing_dependencies(dir.path()).is_empty());
    }

    #[test]
    fn mcworld_installs_as_world_with_levelname() {
        let dir = tempfile::TempDir::new().unwrap();
        let archive = dir.path().join("world.mcworld");
        write_zip(
            &archive,
            &[
                ("levelname.txt", b"My Test World\n"),
                ("level.dat", b"fake-level-dat"),
                ("db/000003.log", b"log"),
            ],
        );
        assert!(matches!(
            detect_archive(&archive).unwrap(),
            ArchiveKind::World
        ));
        let installed = install_world(&archive, dir.path(), None).unwrap();
        assert_eq!(installed.pack_type, "world");
        assert_eq!(installed.name, "My Test World");
        assert!(dir.path().join("My Test World/level.dat").exists());
        assert!(dir.path().join("My Test World/db/000003.log").exists());
    }

    #[test]
    fn wrapped_mcworld_conflicts_and_forced_names() {
        let dir = tempfile::TempDir::new().unwrap();
        let archive = dir.path().join("wrapped.mcworld");
        write_zip(
            &archive,
            &[
                ("MyWorld/level.dat", b"dat"),
                ("MyWorld/levelname.txt", b"Wrapped"),
            ],
        );
        assert!(matches!(
            detect_archive(&archive).unwrap(),
            ArchiveKind::World
        ));
        let first = install_world(&archive, dir.path(), None).unwrap();
        assert_eq!(first.name, "Wrapped");
        let second = install_world(&archive, dir.path(), None).unwrap();
        assert!(second.name.starts_with("Wrapped-"));
        let forced = install_world(&archive, dir.path(), Some("Other/Name!")).unwrap();
        assert_eq!(forced.name, "Other_Name_");
    }

    #[test]
    fn addon_detection_wins_and_unknown_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let mixed = dir.path().join("mixed.zip");
        write_zip(
            &mixed,
            &[
                ("manifest.json", BP_MANIFEST.as_bytes()),
                ("level.dat", b"x"),
            ],
        );
        assert!(matches!(
            detect_archive(&mixed).unwrap(),
            ArchiveKind::Addon
        ));
        let unknown = dir.path().join("unknown.zip");
        write_zip(&unknown, &[("readme.txt", b"nothing here")]);
        assert!(matches!(
            detect_archive(&unknown).unwrap(),
            ArchiveKind::Unknown
        ));
    }

    #[tokio::test]
    async fn delete_strips_uuid_from_order_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let world = dir.path().join("w");
        let pack = world.join("behavior_packs/p");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(world.join("world_behavior_packs.json"),
            r#"[{"pack_id":"11111111-1111-1111-1111-111111111111","version":[1,2,3]},{"pack_id":"other"}]"#)
            .unwrap();
        std::fs::write(
            world.join("world_resource_packs.json"),
            r#"[{"pack_id":"zzz"}]"#,
        )
        .unwrap();
        delete_pack(&pack, &world, "11111111-1111-1111-1111-111111111111")
            .await
            .unwrap();
        assert!(!pack.exists());
        let bp: Value = serde_json::from_str(
            &std::fs::read_to_string(world.join("world_behavior_packs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(bp.as_array().unwrap().len(), 1);
        assert_eq!(bp[0]["pack_id"], "other");
        let rp: Value = serde_json::from_str(
            &std::fs::read_to_string(world.join("world_resource_packs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(rp.as_array().unwrap().len(), 1);
    }
}
