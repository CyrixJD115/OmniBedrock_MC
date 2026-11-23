//! Installed-pack inventory for attribution: maps the names, folders,
//! namespaces and uuids that log lines mention back to pack folders.

use super::parse::strip_codes;
use super::types::{Hint, PackRef};
use crate::addons::{parse_lang, resolve_lang};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const NAMESPACE_DIRS: &[&str] = &[
    "blocks",
    "items",
    "entities",
    "feature_rules",
    "features",
    "recipes",
    "biomes",
    "spawn_rules",
    "item_catalog",
    "entity",
    "attachables",
];
const NAMESPACE_FILES: usize = 400;
const REFERENCE_BYTES: u64 = 192 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Pack {
    pub r: PackRef,
    pub dir: PathBuf,
    pub manifest: Option<Value>,
    pub manifest_error: Option<String>,
    pub active: bool,
    pub names: Vec<String>,
    pub namespaces: HashSet<String>,
}

impl Pack {
    pub fn script_modules(&self) -> Vec<(String, String)> {
        self.manifest
            .as_ref()
            .and_then(|m| m["dependencies"].as_array())
            .map(|deps| {
                deps.iter()
                    .filter_map(|d| {
                        let name = d["module_name"].as_str()?;
                        let version = match &d["version"] {
                            Value::String(s) => s.clone(),
                            Value::Array(a) => a
                                .iter()
                                .filter_map(Value::as_u64)
                                .map(|n| n.to_string())
                                .collect::<Vec<_>>()
                                .join("."),
                            _ => return None,
                        };
                        Some((name.to_owned(), version))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub struct Inventory {
    pub packs: Vec<Pack>,
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Some packs ship a UTF-8 BOM or // comments; tolerate the BOM.
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
}

fn lang_table(dir: &Path) -> HashMap<String, String> {
    let texts = dir.join("texts");
    let mut path = texts.join("en_US.lang");
    if !path.exists() {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&texts)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "lang"))
            .collect();
        found.sort();
        match found.into_iter().next() {
            Some(p) => path = p,
            None => return HashMap::new(),
        }
    }
    std::fs::read_to_string(path)
        .map(|t| parse_lang(&t))
        .unwrap_or_default()
}

fn active_uuids(world: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    for file in ["world_behavior_packs.json", "world_resource_packs.json"] {
        if let Ok(Value::Array(entries)) = read_json(&world.join(file)) {
            for e in entries {
                if let Some(id) = e["pack_id"].as_str() {
                    out.insert(id.to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// `"identifier": "ns:name"` namespaces declared by a pack (sampled).
fn namespaces(dir: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut seen = 0usize;
    for sub in NAMESPACE_DIRS {
        let root = dir.join(sub);
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root).max_depth(6).into_iter().flatten() {
            if seen >= NAMESPACE_FILES {
                return out;
            }
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "json")
            {
                continue;
            }
            seen += 1;
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let mut rest = text.as_str();
            while let Some(i) = rest.find("\"identifier\"") {
                rest = &rest[i + 12..];
                let Some(q) = rest.find('"') else { break };
                let value = &rest[q + 1..];
                let Some(end) = value.find('"') else { break };
                if let Some((ns, _)) = value[..end].split_once(':')
                    && !ns.is_empty()
                    && ns != "minecraft"
                {
                    out.insert(ns.to_owned());
                }
                rest = &value[end..];
            }
        }
    }
    out
}

fn normalize_name(s: &str) -> String {
    strip_codes(s).trim().to_ascii_lowercase()
}

impl Inventory {
    /// Every pack folder of the given worlds.
    pub fn load(worlds_root: &Path, worlds: &[String]) -> Self {
        let mut packs = Vec::new();
        for world in worlds {
            let world_dir = worlds_root.join(world);
            let active = active_uuids(&world_dir);
            for pack_type in ["behavior_packs", "resource_packs"] {
                let Ok(entries) = std::fs::read_dir(world_dir.join(pack_type)) else {
                    continue;
                };
                for entry in entries.flatten().take(512) {
                    if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                        continue;
                    }
                    let dir = entry.path();
                    let folder = entry.file_name().to_string_lossy().into_owned();
                    let (manifest, manifest_error) = match read_json(&dir.join("manifest.json")) {
                        Ok(v) => (Some(v), None),
                        Err(e) => (None, Some(e)),
                    };
                    let header = manifest.as_ref().map(|m| &m["header"]);
                    let raw = header
                        .and_then(|h| h["name"].as_str())
                        .unwrap_or(&folder)
                        .to_owned();
                    let texts = lang_table(&dir);
                    let resolved = resolve_lang(&texts, Some(&raw)).unwrap_or_else(|| raw.clone());
                    let uuid = header
                        .and_then(|h| h["uuid"].as_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    let mut names = vec![
                        normalize_name(&raw),
                        normalize_name(&resolved),
                        folder.to_ascii_lowercase(),
                    ];
                    names.dedup();
                    packs.push(Pack {
                        r: PackRef {
                            uuid: uuid.clone(),
                            name: strip_codes(&resolved).trim().to_owned(),
                            folder: folder.clone(),
                            pack_type: pack_type.to_owned(),
                            world: world.clone(),
                        },
                        active: active.contains(&uuid),
                        namespaces: namespaces(&dir),
                        dir,
                        manifest,
                        manifest_error,
                        names,
                    });
                }
            }
        }
        Self { packs }
    }

    /// Match a hint to a pack, preferring active behavior packs.
    pub fn resolve(&self, hint: &Hint) -> Option<&Pack> {
        let rank = |p: &&Pack| (!p.active, p.r.pack_type != "behavior_packs");
        let mut hits: Vec<&Pack> = match hint {
            Hint::Name(name) => {
                let n = normalize_name(name);
                let exact: Vec<&Pack> =
                    self.packs.iter().filter(|p| p.names.contains(&n)).collect();
                if exact.is_empty() {
                    self.packs
                        .iter()
                        .filter(|p| {
                            p.names
                                .iter()
                                .any(|x| x.len() > 3 && (x.contains(&n) || n.contains(x.as_str())))
                        })
                        .collect()
                } else {
                    exact
                }
            }
            Hint::Folder(folder) => self
                .packs
                .iter()
                .filter(|p| p.r.folder == *folder)
                .collect(),
            Hint::Namespace(ns) => self
                .packs
                .iter()
                .filter(|p| p.namespaces.contains(ns))
                .collect(),
            Hint::Uuid(uuid) => self.packs.iter().filter(|p| p.r.uuid == *uuid).collect(),
            Hint::Plugin(_) => Vec::new(),
        };
        hits.sort_by_key(rank);
        hits.into_iter().next()
    }

    pub fn defines_namespace(&self, ns: &str) -> bool {
        self.packs.iter().any(|p| p.namespaces.contains(ns))
    }

    /// Active packs whose JSON mentions `ns:` (bounded full-text search).
    pub fn referencing(&self, needles: &[String]) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        if needles.is_empty() {
            return out;
        }
        let patterns: Vec<String> = needles.iter().map(|n| format!("\"{n}:")).collect();
        let mut budget = REFERENCE_BYTES;
        for pack in self.packs.iter().filter(|p| p.active) {
            for entry in WalkDir::new(&pack.dir).max_depth(8).into_iter().flatten() {
                if budget == 0 {
                    return out;
                }
                if !entry.file_type().is_file()
                    || entry.path().extension().is_none_or(|e| e != "json")
                {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                budget = budget.saturating_sub(text.len() as u64);
                for (needle, pattern) in needles.iter().zip(&patterns) {
                    if text.contains(pattern.as_str()) {
                        let list = out.entry(needle.clone()).or_default();
                        if !list.contains(&pack.r.name) {
                            list.push(pack.r.name.clone());
                        }
                    }
                }
            }
        }
        out
    }
}
