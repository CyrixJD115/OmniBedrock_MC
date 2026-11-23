//! The server's own player lists: `allowlist.json`, `permissions.json`
//! (xuid → operator/member/visitor) and Endstone's `banned-players.json`.
//! Writes are atomic; the API uses them while the server is stopped and the
//! console commands (`allowlist add`, `op`, …) while it runs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AllowEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xuid: Option<String>,
    #[serde(rename = "ignoresPlayerLimit", default)]
    pub ignores_player_limit: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PermissionEntry {
    pub permission: String,
    pub xuid: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct BanEntry {
    pub name: String,
    pub reason: Option<String>,
    pub created: Option<String>,
    pub source: Option<String>,
    pub expires: Option<String>,
}

pub const PERMISSION_LEVELS: &[&str] = &["visitor", "member", "operator"];

fn read_array(path: &Path) -> Vec<Value> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

pub fn allowlist(server_dir: &Path) -> Vec<AllowEntry> {
    read_array(&server_dir.join("allowlist.json"))
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

pub fn permissions(server_dir: &Path) -> Vec<PermissionEntry> {
    read_array(&server_dir.join("permissions.json"))
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

pub fn bans(server_dir: &Path) -> Vec<BanEntry> {
    let text = |v: &Value, k: &str| v[k].as_str().map(str::to_owned).filter(|s| !s.is_empty());
    read_array(&server_dir.join("banned-players.json"))
        .iter()
        .filter_map(|v| {
            Some(BanEntry {
                name: text(v, "name")?,
                reason: text(v, "reason"),
                created: text(v, "created"),
                source: text(v, "source"),
                expires: text(v, "expires"),
            })
        })
        .collect()
}

async fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    omnibedrock_core::files::atomic_write(path, &bytes).await
}

/// Add or remove one allowlist entry (by name, case-insensitive).
pub async fn set_allowlisted(
    server_dir: &Path,
    name: &str,
    xuid: Option<String>,
    on: bool,
) -> anyhow::Result<bool> {
    let mut list = allowlist(server_dir);
    let before = list.len();
    list.retain(|e| !e.name.eq_ignore_ascii_case(name));
    let removed = list.len() != before;
    if on {
        list.push(AllowEntry {
            name: name.to_owned(),
            xuid,
            ignores_player_limit: false,
        });
    }
    write_json(&server_dir.join("allowlist.json"), &list).await?;
    Ok(on || removed)
}

/// Set an xuid's permission level in permissions.json; "member" (the
/// default) removes the entry rather than storing it.
pub async fn set_permission(server_dir: &Path, xuid: &str, level: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        PERMISSION_LEVELS.contains(&level),
        "unknown permission level '{level}'"
    );
    let mut list = permissions(server_dir);
    list.retain(|e| e.xuid != xuid);
    if level != "member" {
        list.push(PermissionEntry {
            permission: level.to_owned(),
            xuid: xuid.to_owned(),
        });
    }
    write_json(&server_dir.join("permissions.json"), &list).await
}
