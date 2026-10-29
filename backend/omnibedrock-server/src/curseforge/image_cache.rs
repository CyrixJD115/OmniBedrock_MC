//! On-disk cache for proxied forgecdn images: extension mapping, cached
//! reads with mtime refresh, and the prune-in-progress guard.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Reset the prune-in-progress flag even if the prune panics.
pub(super) struct PruneGuard<'a>(pub(super) &'a AtomicBool);
impl Drop for PruneGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Cached image content type per file extension; unknown types bypass the cache.
pub(super) fn image_ext(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// Load a cached image, refreshing its mtime so hot entries outlive cold ones.
pub(super) async fn read_cached_image(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    for (ext, content_type) in [
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("webp", "image/webp"),
        ("gif", "image/gif"),
    ] {
        let candidate = path.with_extension(ext);
        if let Ok(bytes) = tokio::fs::read(&candidate).await {
            // Touch the mtime so LRU eviction keeps hot thumbnails; a single
            // metadata syscall, not worth a spawn_blocking round-trip.
            if let Ok(file) = std::fs::File::options().append(true).open(&candidate) {
                let _ = file.set_modified(std::time::SystemTime::now());
            }
            return Some((bytes, content_type));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{image_ext, read_cached_image};
    use crate::curseforge::CurseForge;
    use sha2::{Digest, Sha256};

    #[test]
    fn image_content_types_map_to_extensions() {
        assert_eq!(image_ext("image/png"), Some("png"));
        assert_eq!(image_ext("image/jpeg"), Some("jpg"));
        assert_eq!(image_ext("image/webp"), Some("webp"));
        assert_eq!(image_ext("image/gif"), Some("gif"));
        assert_eq!(image_ext("text/html"), None);
    }

    #[tokio::test]
    async fn image_cache_roundtrip_and_unknown_type_bypass() {
        let dir = tempfile::tempdir().unwrap();
        let cf = CurseForge::new("", Some(dir.path().to_path_buf()));
        let path = dir
            .path()
            .join(hex::encode(Sha256::digest(b"https://x.forgecdn.net/a.png")));
        cf.store_cached_image(&path, "image/png", b"pretend-png")
            .await;
        let (bytes, content_type) = read_cached_image(&path).await.expect("cached hit");
        assert_eq!(bytes, b"pretend-png");
        assert_eq!(content_type, "image/png");
        // Unknown content types are not cached: the original entry is intact.
        cf.store_cached_image(&path, "image/avif", b"nope").await;
        let (bytes, content_type) = read_cached_image(&path).await.expect("still cached");
        assert_eq!(bytes, b"pretend-png");
        assert_eq!(content_type, "image/png");
    }
}
