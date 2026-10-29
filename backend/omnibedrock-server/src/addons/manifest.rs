//! Manifest field inspection and safe pack/world name derivation.

use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

pub(super) fn header_string(manifest: &Value, key: &str) -> Option<String> {
    manifest
        .get("header")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}
pub(super) fn header_uuid(manifest: &Value) -> String {
    header_string(manifest, "uuid").unwrap_or_default()
}
pub(super) fn header_version(manifest: &Value) -> Vec<u64> {
    manifest
        .get("header")
        .and_then(|h| h.get("version"))
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

/// Sanitize a user-controlled pack folder name into a single safe component.
pub(super) fn sanitize_name(raw: &str) -> String {
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
pub(super) fn free_name(parent: &Path, base: &str) -> PathBuf {
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
