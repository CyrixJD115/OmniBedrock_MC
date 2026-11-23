//! Disk side of log noise: BDS `ContentLog*.txt` files (one per start,
//! never rotated) and the `logs/` directory size.

use super::types::Housekeeping;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn content_logs(server_dir: &Path) -> Vec<(SystemTime, PathBuf, u64)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(server_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("ContentLog") && name.ends_with(".txt")) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() {
                out.push((
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    entry.path(),
                    meta.len(),
                ));
            }
        }
    }
    out.sort_by_key(|e| e.0);
    out
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn property(server_dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(server_dir.join("server.properties")).ok()?;
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| {
            l.split_once('=')
                .filter(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_owned())
        })
}

pub fn level_name(server_dir: &Path) -> Option<String> {
    property(server_dir, "level-name").filter(|v| !v.is_empty())
}

pub fn measure(server_dir: &Path) -> Housekeeping {
    let logs = content_logs(server_dir);
    Housekeeping {
        content_log_files: logs.len() as u64,
        content_log_bytes: logs.iter().map(|l| l.2).sum(),
        logs_dir_bytes: dir_bytes(&server_dir.join("logs")),
        content_log_file_enabled: property(server_dir, "content-log-file-enabled")
            .map(|v| v == "true"),
    }
}

/// Delete every ContentLog file except the newest (which a running server
/// may still be writing). Returns (files deleted, bytes freed).
pub fn prune_content_logs(server_dir: &Path) -> (u64, u64) {
    let mut logs = content_logs(server_dir);
    logs.pop();
    let (mut files, mut bytes) = (0, 0);
    for (_, path, len) in logs {
        if std::fs::remove_file(&path).is_ok() {
            files += 1;
            bytes += len;
        }
    }
    (files, bytes)
}
