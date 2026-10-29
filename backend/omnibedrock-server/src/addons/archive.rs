//! Addon zip plumbing: source-pack discovery and bounded, safe extraction.

use anyhow::Context as _;
use serde_json::Value;
use std::io::{Read, Seek};
use std::path::Path;
use zip::ZipArchive;

pub(super) const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_ENTRIES: usize = 20_000;
pub(super) const MAX_PACKS: usize = 64;

pub(super) struct SourcePack {
    /// Archive entry prefix holding the pack ("", "SomePack/", …).
    pub(super) prefix: String,
    pub(super) manifest: Value,
}

/// Locate manifests at the archive root or exactly one folder deep.
pub(super) fn find_source_packs(
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
pub(super) fn extract_prefix(
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
