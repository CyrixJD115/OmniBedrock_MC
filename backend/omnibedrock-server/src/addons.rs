//! Addon archive installation and metadata.
//!
//! Handles `.mcpack` (single pack), `.mcaddon` (multiple packs), and plain
//! `.zip` uploads. Manifests are detected at the archive root or one folder
//! deep; behavior vs resource is decided from module types. Extraction is
//! staged per pack and moved into place with a rename, refusing symlink and
//! traversal entries.

mod archive;
mod classify;
mod install;
mod lang;
mod listing;
mod manifest;
mod metadata;
mod sync;
#[cfg(test)]
mod tests;

pub use classify::{ArchiveKind, detect_archive};
pub use install::{InstalledPack, install_archive, install_world};
pub use lang::{parse_lang, read_texts, resolve_lang};
pub use listing::{delete_pack, pack_stats};
pub use manifest::detect_pack_type;
pub use metadata::{METADATA_FILE, read_pack_metadata, write_pack_metadata};
pub use sync::{
    PackSyncReport, manifest_versions, missing_dependencies, sync_all_worlds, sync_world_packs,
};
