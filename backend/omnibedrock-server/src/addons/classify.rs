//! Uploaded-archive classification: addon packs versus world templates.

use anyhow::Context as _;
use std::path::Path;
use zip::ZipArchive;

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
