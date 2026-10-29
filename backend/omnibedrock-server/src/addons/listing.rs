//! Pack folder statistics and removal, including order-file cleanup.

use serde_json::Value;
use std::path::Path;
use walkdir::WalkDir;

/// Total size, newest modification time, and icon presence for a pack folder.
/// Walks are bounded so a huge pack cannot stall the listing.
pub fn pack_stats(dir: &Path) -> (u64, u64, bool) {
    let mut size = 0u64;
    let mut modified = 0u64;
    let mut icon = false;
    let mut entries = 0usize;
    for entry in WalkDir::new(dir).follow_links(false) {
        let Ok(entry) = entry else { continue };
        entries += 1;
        if entries > 50_000 {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "pack_icon.png" || name == "pack_icon.jpg" {
            icon = true;
        }
        if let Ok(meta) = entry.metadata() {
            size += meta.len();
            if let Ok(mtime) = meta.modified()
                && let Ok(secs) = mtime.duration_since(std::time::UNIX_EPOCH)
            {
                modified = modified.max(secs.as_secs());
            }
        }
    }
    (size, modified, icon)
}

/// Remove a pack directory and strip its UUID from the world's pack order
/// files so BDS never references a missing pack.
pub async fn delete_pack(pack_dir: &Path, world_dir: &Path, uuid: &str) -> anyhow::Result<()> {
    tokio::fs::remove_dir_all(pack_dir).await?;
    if uuid.is_empty() {
        return Ok(());
    }
    for order_file in ["world_behavior_packs.json", "world_resource_packs.json"] {
        let path = world_dir.join(order_file);
        let Ok(text) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        let Ok(mut entries) = serde_json::from_str::<Vec<Value>>(&text) else {
            continue;
        };
        let before = entries.len();
        entries.retain(|e| e.get("pack_id").and_then(Value::as_str) != Some(uuid));
        if entries.len() != before {
            omnibedrock_core::files::atomic_write(
                &path,
                serde_json::to_string_pretty(&entries)?.as_bytes(),
            )
            .await?;
        }
    }
    Ok(())
}
