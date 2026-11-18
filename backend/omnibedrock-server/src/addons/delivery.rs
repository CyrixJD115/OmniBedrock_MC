//! What a joining client has to download: the world's active resource packs
//! (`world_resource_packs.json`), their on-disk sizes, and the things that
//! make every join heavier than it needs to be.
//!
//! Bedrock clients cache server packs by (uuid, version). The panel never
//! rewrites pack versions or manifests on start, so a pack only re-downloads
//! when its version changes — or when the client cannot keep it: console
//! clients (Xbox, PlayStation, Switch) hold server packs in a small,
//! platform-managed cache, and a large stack is evicted between sessions.
//! This report makes the payload visible so it can be trimmed.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A large stack on consoles: above this, re-downloads on every join are
/// the expected console behavior rather than a server fault.
pub const CONSOLE_HEAVY_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Serialize, PartialEq)]
pub struct DeliveredPack {
    pub uuid: String,
    pub name: String,
    pub folder: String,
    pub version: Vec<u64>,
    pub bytes: u64,
    pub files: u64,
    /// The pack ships subpacks (memory tiers) — every tier is downloaded.
    pub subpacks: bool,
    /// No ACTIVE behavior pack depends on this resource pack. Usually a
    /// leftover from a removed add-on: every join still downloads it.
    pub unpaired: bool,
    /// world_resource_packs.json pins a version that differs from the
    /// manifest (BDS then drops or mis-resolves it).
    pub version_mismatch: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct DeliveryReport {
    pub world: String,
    pub packs: Vec<DeliveredPack>,
    pub total_bytes: u64,
    /// Order-file uuids with no pack folder on disk.
    pub missing: Vec<String>,
    pub console_heavy: bool,
    pub texturepack_required: Option<bool>,
}

struct PackOnDisk {
    folder: String,
    name: String,
    version: Vec<u64>,
    dependencies: Vec<String>,
    subpacks: bool,
    path: std::path::PathBuf,
}

fn read_packs(dir: &Path) -> HashMap<String, PackOnDisk> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
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
        let Some(uuid) = header["uuid"].as_str() else {
            continue;
        };
        let folder = entry.file_name().to_string_lossy().into_owned();
        let raw_name = header["name"].as_str().unwrap_or_default();
        // Localized names ("pack.name") resolve through texts/en_US.lang.
        let name = if raw_name.is_empty() || raw_name == "pack.name" {
            localized_name(&entry.path()).unwrap_or_else(|| folder.clone())
        } else {
            raw_name.to_owned()
        };
        out.insert(
            uuid.to_ascii_lowercase(),
            PackOnDisk {
                folder,
                name,
                version: header["version"]
                    .as_array()
                    .map(|v| v.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default(),
                dependencies: manifest["dependencies"]
                    .as_array()
                    .map(|deps| {
                        deps.iter()
                            .filter_map(|d| d["uuid"].as_str().map(str::to_ascii_lowercase))
                            .collect()
                    })
                    .unwrap_or_default(),
                subpacks: manifest
                    .get("subpacks")
                    .and_then(Value::as_array)
                    .is_some_and(|s| !s.is_empty()),
                path: entry.path(),
            },
        );
    }
    out
}

/// `pack.name` from the pack's en_US (or first) `.lang` table.
fn localized_name(pack: &Path) -> Option<String> {
    let texts = pack.join("texts");
    let mut file = texts.join("en_US.lang");
    if !file.exists() {
        let mut langs: Vec<_> = std::fs::read_dir(&texts)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lang"))
            .collect();
        langs.sort();
        file = langs.into_iter().next()?;
    }
    let text = std::fs::read_to_string(file).ok()?;
    super::lang::parse_lang(&text)
        .remove("pack.name")
        .filter(|n| !n.is_empty())
}

fn order(world: &Path, file: &str) -> Vec<(String, Vec<u64>)> {
    let Ok(text) = std::fs::read_to_string(world.join(file)) else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<Value>>(&text) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| {
            let uuid = e["pack_id"].as_str()?.to_ascii_lowercase();
            let version = e["version"]
                .as_array()
                .map(|v| v.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            Some((uuid, version))
        })
        .collect()
}

fn dir_size(path: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .flatten()
        .take(200_000)
    {
        if entry.file_type().is_file() {
            files += 1;
            bytes += entry.metadata().map_or(0, |m| m.len());
        }
    }
    (bytes, files)
}

/// Build the delivery report for one world directory.
pub fn delivery_report(world_dir: &Path, texturepack_required: Option<bool>) -> DeliveryReport {
    let resource = read_packs(&world_dir.join("resource_packs"));
    let behavior = read_packs(&world_dir.join("behavior_packs"));
    let active_bp: HashSet<String> = order(world_dir, "world_behavior_packs.json")
        .into_iter()
        .map(|(u, _)| u)
        .collect();
    let mut packs = Vec::new();
    let mut missing = Vec::new();
    for (uuid, pinned) in order(world_dir, "world_resource_packs.json") {
        let Some(pack) = resource.get(&uuid) else {
            missing.push(uuid);
            continue;
        };
        let paired_active = behavior
            .iter()
            .any(|(bp_uuid, bp)| active_bp.contains(bp_uuid) && bp.dependencies.contains(&uuid));
        let (bytes, files) = dir_size(&pack.path);
        packs.push(DeliveredPack {
            uuid: uuid.clone(),
            name: pack.name.clone(),
            folder: pack.folder.clone(),
            version: pack.version.clone(),
            bytes,
            files,
            subpacks: pack.subpacks,
            unpaired: !paired_active,
            version_mismatch: !pinned.is_empty() && pinned != pack.version,
        });
    }
    let total_bytes = packs.iter().map(|p| p.bytes).sum();
    packs.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    DeliveryReport {
        world: world_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        packs,
        total_bytes,
        missing,
        console_heavy: total_bytes >= CONSOLE_HEAVY_BYTES,
        texturepack_required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(dir: &Path, folder: &str, uuid: &str, version: [u64; 3], deps: &[&str], bytes: usize) {
        let p = dir.join(folder);
        std::fs::create_dir_all(&p).unwrap();
        let deps: Vec<Value> = deps
            .iter()
            .map(|d| serde_json::json!({"uuid": d, "version": [1,0,0]}))
            .collect();
        std::fs::write(
            p.join("manifest.json"),
            serde_json::json!({"format_version": 2, "header": {"name": folder, "uuid": uuid, "version": version}, "dependencies": deps}).to_string(),
        )
        .unwrap();
        std::fs::write(p.join("blob.bin"), vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn reports_sizes_unpaired_and_mismatched_packs() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("W");
        pack(
            &world.join("resource_packs"),
            "rp_a",
            "aaaa",
            [1, 0, 0],
            &[],
            1000,
        );
        pack(
            &world.join("resource_packs"),
            "rp_orphan",
            "bbbb",
            [2, 0, 0],
            &[],
            5000,
        );
        pack(
            &world.join("behavior_packs"),
            "bp_a",
            "cccc",
            [1, 0, 0],
            &["aaaa"],
            10,
        );
        std::fs::write(
            world.join("world_behavior_packs.json"),
            r#"[{"pack_id":"cccc","version":[1,0,0]}]"#,
        )
        .unwrap();
        std::fs::write(
            world.join("world_resource_packs.json"),
            r#"[{"pack_id":"aaaa","version":[1,0,0]},{"pack_id":"bbbb","version":[1,9,9]},{"pack_id":"dddd","version":[1,0,0]}]"#,
        )
        .unwrap();
        let report = delivery_report(&world, Some(true));
        assert_eq!(report.packs.len(), 2);
        assert_eq!(report.missing, vec!["dddd".to_string()]);
        // Largest first.
        assert_eq!(report.packs[0].uuid, "bbbb");
        assert!(report.packs[0].unpaired);
        assert!(report.packs[0].version_mismatch);
        assert!(!report.packs[1].unpaired);
        assert!(!report.packs[1].version_mismatch);
        assert!(report.total_bytes >= 6000);
        assert!(!report.console_heavy);
    }
}
