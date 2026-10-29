//! CurseForge Core API client for Minecraft Bedrock content.
//!
//! Follows the same contract PrismLauncher's Flame integration uses: the
//! official `https://api.curseforge.com/v1` endpoints authenticated with an
//! `x-api-key` header, pointed at the Minecraft Bedrock game. Search terms
//! mirror the public website (class=addons|maps|texture-packs|scripts|skins,
//! sortBy=relevancy|popularity|latest-update|creation-date|total-downloads)
//! so the panel Store behaves like browsing the site.
//!
//! File downloads use the `downloadUrl` the API returns on each file; the
//! CDN (edge.forgecdn.net) answers with a redirect to mediafilez.forgecdn.net
//! and needs no key. Files without a `downloadUrl` are distribution-blocked
//! by CurseForge and can only be fetched manually.

mod client;
mod consts;
mod image_cache;
mod models;
mod query;

pub use client::CurseForge;
pub use consts::{CLASSES, GAME_ID_BEDROCK, MAX_DOWNLOAD_BYTES};
pub use models::{
    CfAuthor, CfCategory, CfCategoryRef, CfDependency, CfFile, CfLatestFileIndex, CfLinks, CfLogo,
    CfPagination, CfProject, CfScreenshot,
};
pub use query::{class_id, sort_field};
