//! World pack-order syncing: pinning `world_behavior_packs.json` /
//! `world_resource_packs.json` entries to installed manifest versions and
//! reporting referenced-but-missing pack uuids.

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

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

/// Installed pack uuid → declared header version. Order files must pin this
/// exact version or BDS drops the pack ("was not found and was ignored").
pub fn manifest_versions(world_dir: &Path) -> HashMap<String, Vec<u64>> {
    installed_world_packs(world_dir)
        .into_iter()
        .map(|(uuid, pack)| (uuid, pack.version))
        .collect()
}
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
