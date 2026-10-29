//! CurseForge API constants: the Bedrock game id, category classes, cache
//! TTLs, and the image-proxy and download byte budgets.
use std::time::Duration;

/// Minecraft Bedrock in CurseForge's game catalog (site slug `minecraft-bedrock`).
pub const GAME_ID_BEDROCK: u64 = 78022;

pub(super) const API_BASE: &str = "https://api.curseforge.com/v1";

/// Root category (class) ids for Bedrock, as returned by
/// `/categories?gameId=78022` where `parentCategoryId` is null.
pub const CLASSES: &[(&str, u64)] = &[
    ("addons", 4984),
    ("maps", 6913),
    ("texture-packs", 6929),
    ("scripts", 6940),
    ("skins", 6925),
];

pub(super) const SEARCH_TTL: Duration = Duration::from_secs(60);
pub(super) const META_TTL: Duration = Duration::from_secs(3600);
pub(super) const VERSIONS_TTL: Duration = Duration::from_secs(24 * 3600);
pub(super) const PROJECT_TTL: Duration = Duration::from_secs(300);
pub(super) const FILES_TTL: Duration = Duration::from_secs(120);

/// Per-request deadline for proxied thumbnails; forgecdn hiccups should show
/// a broken image fast instead of hanging the card grid for 30s.
pub(super) const IMAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(12);
/// Hard cap for a single proxied image (thumbnails are ~10–100 KiB).
pub(super) const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// On-disk thumbnail cache budget; eviction is LRU-by-mtime once exceeded.
pub(super) const IMAGE_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Hard cap mirroring the manual upload limit.
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
