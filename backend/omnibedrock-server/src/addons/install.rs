//! Installing addon packs and world archives into a world tree: staging,
//! rename-into-place, conflict suffixing, and cleanup on failure.

use anyhow::Context as _;
use std::path::Path;
use uuid::Uuid;
use zip::ZipArchive;

use super::archive::{extract_prefix, find_source_packs};
use super::manifest::{
    detect_pack_type, free_name, header_string, header_uuid, header_version, sanitize_name,
};

#[derive(Debug, serde::Serialize)]
pub struct InstalledPack {
    pub name: String,
    #[serde(rename = "pack_type")]
    pub pack_type: &'static str,
    pub uuid: String,
    pub version: Vec<u64>,
}

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
