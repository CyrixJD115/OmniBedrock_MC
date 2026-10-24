//! Filesystem operations are constrained to configured roots; symlinks are rejected.
use crate::error::{ApiError, Result};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub fn component(name: &str) -> Result<&str> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(ApiError::bad("Invalid filename"));
    }
    Ok(name)
}
pub async fn safe_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(ApiError::bad("Invalid path"));
    }
    let root = tokio::fs::canonicalize(root).await?;
    let mut target = root.clone();
    for part in relative.components() {
        target.push(part);
        match tokio::fs::symlink_metadata(&target).await {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(ApiError::bad("Symlinks are not allowed"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if !target.starts_with(&root) {
        return Err(ApiError::bad("Path outside configured root"));
    }
    Ok(target)
}
/// Atomic replacement on the same filesystem. Restrictive permissions protect auth data.
pub async fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent"))?;
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut f = options.open(&temp).await?;
        f.write_all(bytes).await?;
        f.sync_all().await?;
        drop(f);
        tokio::fs::rename(&temp, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}
pub async fn dirs(root: &Path) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            result.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    result.sort();
    Ok(result)
}
pub fn properties(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return json!({"key":"", "value":"", "comment":line, "inline_comment":""});
            }
            let (data, comment) = line
                .split_once('#')
                .map_or((line, String::new()), |(a, b)| {
                    (a.trim(), format!("#{}", b.trim_end()))
                });
            let (key, value) = data.split_once('=').unwrap_or((data, ""));
            json!({"key":key.trim(),"value":value.trim(),"comment":"","inline_comment":comment})
        })
        .collect()
}
pub fn validate_property(key: &str, value: &str) -> Result<()> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(ApiError::bad("Property value must be a single line"));
    }
    let range = match key {
        "max-players" => Some((1, 1000)),
        "server-port" | "server-portv6" => Some((1, 65535)),
        "view-distance" => Some((2, 96)),
        "tick-distance" => Some((4, 12)),
        "player-idle-timeout" => Some((0, i64::MAX)),
        "compression-threshold" => Some((0, 65535)),
        _ => None,
    };
    if let Some((min, max)) = range {
        let n = value
            .parse::<i64>()
            .map_err(|_| ApiError::bad(format!("'{key}' must be an integer")))?;
        if n < min || n > max {
            return Err(ApiError::bad(format!(
                "'{key}' must be between {min} and {max}"
            )));
        }
    }
    let choices: &[&str] = match key {
        "gamemode" => &["survival", "creative", "adventure"],
        "difficulty" => &["peaceful", "easy", "normal", "hard"],
        "default-player-permission-level" => &["visitor", "member", "operator"],
        "chat-restriction" => &["None", "Dropped", "Disabled"],
        _ => &[],
    };
    if !choices.is_empty() && !choices.contains(&value) {
        return Err(ApiError::bad(format!(
            "'{key}' must be one of: {}",
            choices.join(", ")
        )));
    }
    if [
        "allow-cheats",
        "online-mode",
        "white-list",
        "texturepack-required",
        "content-log-file-enabled",
        "server-authoritative-movement",
        "enable-lan-visibility",
        "disable-player-interaction",
    ]
    .contains(&key)
        && !["true", "false", "1", "0"].contains(&value.to_lowercase().as_str())
    {
        return Err(ApiError::bad(format!("'{key}' must be true/false")));
    }
    Ok(())
}
pub fn world_tree(root: &Path) -> anyhow::Result<Value> {
    fn tree(path: &Path, budget: &mut usize, depth: usize) -> anyhow::Result<Value> {
        anyhow::ensure!(*budget > 0 && depth < 64, "world tree exceeds limit");
        *budget -= 1;
        let m = std::fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() {
            return Ok(json!({"name":path.file_name(),"type":"symlink"}));
        }
        if m.is_dir() {
            let mut paths = std::fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            let children = paths
                .iter()
                .map(|p| tree(p, budget, depth + 1))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(json!({"name":path.file_name(),"type":"directory","children":children}))
        } else {
            Ok(json!({"name":path.file_name(),"type":"file","size":m.len()}))
        }
    }
    tree(root, &mut 100_000, 0)
}
