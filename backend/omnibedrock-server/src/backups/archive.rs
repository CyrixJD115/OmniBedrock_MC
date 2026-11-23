//! World archive creation (zip) and restore with safety and size checks.

use anyhow::{Context, ensure};
use std::io::{Read, Write};
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::live::Snapshot;
use super::types::BackupRequest;

/// What one archive run wrote.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchiveStats {
    pub files: u64,
    /// Uncompressed bytes written into the archive.
    pub bytes: u64,
    /// Files cut to the length `save query` reported (live backups).
    pub truncated: u64,
    /// LevelDB files present on disk but not part of the held snapshot.
    pub skipped: u64,
}

fn collect_roots(source: &Path, r: &BackupRequest) -> Vec<std::path::PathBuf> {
    if r.full_backup {
        vec![source.to_path_buf()]
    } else {
        r.include_items
            .as_ref()
            .unwrap_or(&vec![])
            .iter()
            .map(|p| source.join(p))
            .collect()
    }
}

/// Zip `source` (a world folder) into `destination`. With a `snapshot` from
/// `save query`, listed files are copied only up to their reported length and
/// unlisted `db/` files are skipped, giving a consistent LevelDB image while
/// the server keeps running. `progress(done, total)` reports bytes.
pub(super) fn create_zip(
    source: &Path,
    destination: &Path,
    r: &BackupRequest,
    snapshot: Option<&Snapshot>,
    progress: &dyn Fn(u64, u64),
) -> anyhow::Result<ArchiveStats> {
    ensure!(source.is_dir(), "World not found");
    let roots = collect_roots(source, r);
    let total: u64 = roots
        .iter()
        .flat_map(|root| WalkDir::new(root).follow_links(false).into_iter().flatten())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    let temp = destination.with_extension(format!("{}.part", Uuid::new_v4()));
    let mut stats = ArchiveStats::default();
    let result = || -> anyhow::Result<()> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut archive = ZipWriter::new(std::io::BufWriter::with_capacity(1 << 20, file));
        let options = SimpleFileOptions::default()
            .compression_method(if r.compression == "deflate" {
                CompressionMethod::Deflated
            } else {
                CompressionMethod::Stored
            })
            .large_file(true);
        let parent = source.parent().context("missing parent")?;
        let mut seen = std::collections::BTreeSet::new();
        let mut last_report = 0u64;
        progress(0, total);
        for root in roots {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(root).follow_links(false) {
                let entry = entry?;
                ensure!(
                    !entry.file_type().is_symlink(),
                    "Symlinks cannot be archived"
                );
                let path = entry.path();
                let name = path
                    .strip_prefix(parent)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !seen.insert(name.clone()) {
                    continue;
                }
                if entry.file_type().is_dir() {
                    archive.add_directory(format!("{name}/"), options)?;
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let in_world = path
                    .strip_prefix(source)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let limit = snapshot.and_then(|s| s.get(&in_world).copied());
                if snapshot.is_some() && limit.is_none() && in_world.starts_with("db/") {
                    stats.skipped += 1;
                    continue;
                }
                archive.start_file(name, options)?;
                let mut input = std::fs::File::open(path)?;
                let copied = match limit {
                    Some(len) => {
                        let on_disk = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        if on_disk > len {
                            stats.truncated += 1;
                        }
                        std::io::copy(&mut (&mut input).take(len), &mut archive)?
                    }
                    None => std::io::copy(&mut input, &mut archive)?,
                };
                stats.files += 1;
                stats.bytes += copied;
                if stats.bytes - last_report >= 8 * 1024 * 1024 {
                    last_report = stats.bytes;
                    progress(stats.bytes, total);
                }
            }
        }
        let writer = archive.finish()?;
        let file = writer.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        ensure!(!destination.exists(), "Backup destination already exists");
        std::fs::rename(&temp, destination)?;
        progress(total, total);
        Ok(())
    }();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map(|()| stats)
}

/// Read every entry back (the zip reader checks each CRC-32) and confirm the
/// archive holds a restorable world. Returns (entries, uncompressed bytes).
pub(super) fn verify_archive(archive: &Path, world: &str) -> anyhow::Result<(u64, u64)> {
    let mut zip = ZipArchive::new(std::fs::File::open(archive)?)?;
    let (mut entries, mut bytes) = (0u64, 0u64);
    let mut has_world_data = false;
    let mut sink = std::io::sink();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_owned();
        ensure!(
            name.split('/').next() == Some(world),
            "Entry {name} is outside the world folder"
        );
        if name == format!("{world}/level.dat") || name.starts_with(&format!("{world}/db/")) {
            has_world_data = true;
        }
        bytes += std::io::copy(&mut entry, &mut sink)
            .with_context(|| format!("{name} is corrupt (CRC or data error)"))?;
        entries += 1;
    }
    ensure!(
        has_world_data,
        "Archive has no level.dat or db/ for {world}"
    );
    Ok((entries, bytes))
}

/// Total bytes an archive may extract to (matches the zip bomb guard in
/// [`restore_archive`]).
const EXTRACTION_LIMIT: u64 = 64 * 1024 * 1024 * 1024;

/// Static checks for one archive entry: it lives under `world/`, is not a
/// symlink, and its size still fits the extraction budget `total` tracks.
fn check_entry(
    path: Option<&Path>,
    unix_mode: Option<u32>,
    size: u64,
    total: &mut u64,
    world: &str,
) -> anyhow::Result<()> {
    *total = total.checked_add(size).context("archive size overflow")?;
    ensure!(
        *total <= EXTRACTION_LIMIT,
        "Archive exceeds 64 GiB extraction limit"
    );
    let path = path.context("Archive contains unsafe path")?;
    ensure!(
        path.components()
            .next()
            .is_some_and(|c| c.as_os_str() == world),
        "Archive contains a different world"
    );
    ensure!(
        !unix_mode.is_some_and(|m| m & 0o170000 == 0o120000),
        "Archive contains a symlink"
    );
    Ok(())
}

/// Run every archive-only safety check [`restore_archive`] enforces — same
/// world folder, no symlinks, within the extraction limit — without
/// extracting. Called before the server is stopped so a rejected archive
/// never causes downtime.
pub(super) fn prevalidate_archive(archive: &Path, world: &str) -> anyhow::Result<()> {
    let mut zip = ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        check_entry(
            entry.enclosed_name().as_deref(),
            entry.unix_mode(),
            entry.size(),
            &mut total,
            world,
        )?;
    }
    Ok(())
}

pub(super) fn restore_archive(archive: &Path, target: &Path, world: &str) -> anyhow::Result<()> {
    let mut zip = ZipArchive::new(std::fs::File::open(archive)?)?;
    let parent = target.parent().context("missing world root")?;
    let stage = parent.join(format!(".{world}.restore.{}", Uuid::new_v4()));
    std::fs::create_dir(&stage)?;
    let result = || -> anyhow::Result<()> {
        let mut total = 0u64;
        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            check_entry(
                entry.enclosed_name().as_deref(),
                entry.unix_mode(),
                entry.size(),
                &mut total,
                world,
            )?;
        }
        let mut remaining = EXTRACTION_LIMIT;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let path = entry.enclosed_name().context("unsafe path")?;
            let relative = path.strip_prefix(world)?;
            let output = stage.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(output)?;
            } else {
                std::fs::create_dir_all(output.parent().context("missing parent")?)?;
                let mut file = std::fs::File::create(output)?;
                let mut limited = (&mut entry).take(remaining + 1);
                let copied = std::io::copy(&mut limited, &mut file)?;
                ensure!(copied <= remaining, "Archive exceeds extraction limit");
                remaining -= copied;
                file.flush()?;
            }
        }
        ensure!(
            stage.join("level.dat").exists() || stage.join("db").is_dir(),
            "Archive is not a restorable world (missing level.dat/db)"
        );
        // Hidden, so world listings and "all worlds" backups skip it.
        let previous = parent.join(format!(
            ".{world}.pre-restore-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        let existed = target.exists();
        if existed {
            std::fs::rename(target, &previous)?;
        }
        if let Err(e) = std::fs::rename(&stage, target) {
            if existed {
                let _ = std::fs::rename(&previous, target);
            }
            return Err(e.into());
        }
        Ok(())
    }();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}
