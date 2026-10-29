//! World archive creation (zip) and restore with safety and size checks.

use anyhow::{Context, ensure};
use std::io::{Read, Write};
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::types::BackupRequest;

pub(super) fn create_zip(
    source: &Path,
    destination: &Path,
    r: &BackupRequest,
) -> anyhow::Result<()> {
    ensure!(source.is_dir(), "World not found");
    let temp = destination.with_extension(format!("{}.part", Uuid::new_v4()));
    let result = || -> anyhow::Result<()> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut archive = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(if r.compression == "deflate" {
                CompressionMethod::Deflated
            } else {
                CompressionMethod::Stored
            });
        let roots = if r.full_backup {
            vec![source.to_path_buf()]
        } else {
            r.include_items
                .as_ref()
                .unwrap_or(&vec![])
                .iter()
                .map(|p| source.join(p))
                .collect()
        };
        let mut seen = std::collections::BTreeSet::new();
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
                let relative = path.strip_prefix(source.parent().context("missing parent")?)?;
                let name = relative.to_string_lossy().replace('\\', "/");
                if !seen.insert(name.clone()) {
                    continue;
                }
                if entry.file_type().is_dir() {
                    archive.add_directory(format!("{name}/"), options)?;
                } else if entry.file_type().is_file() {
                    archive.start_file(name, options)?;
                    let mut file = std::fs::File::open(path)?;
                    std::io::copy(&mut file, &mut archive)?;
                }
            }
        }
        let file = archive.finish()?;
        file.sync_all()?;
        ensure!(!destination.exists(), "Backup destination already exists");
        std::fs::rename(&temp, destination)?;
        Ok(())
    }();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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
            total = total
                .checked_add(entry.size())
                .context("archive size overflow")?;
            ensure!(
                total <= 64 * 1024 * 1024 * 1024,
                "Archive exceeds 64 GiB extraction limit"
            );
            let path = entry
                .enclosed_name()
                .context("Archive contains unsafe path")?;
            let mut components = path.components();
            ensure!(
                components.next().is_some_and(|c| c.as_os_str() == world),
                "Archive contains a different world"
            );
            ensure!(
                !entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
                "Archive contains a symlink"
            );
        }
        let mut remaining = 64u64 * 1024 * 1024 * 1024;
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
        let previous = parent.join(format!(
            "{world}.bak.{}.{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            Uuid::new_v4().simple()
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
