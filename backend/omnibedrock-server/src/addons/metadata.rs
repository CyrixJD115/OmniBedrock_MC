//! Pack provenance sidecar: reading and writing `.omnibedrock.json`.

use serde_json::Value;
use std::path::Path;

/// Panel metadata sidecar stored inside each installed pack or world
/// directory, recording where the content came from (CurseForge Store vs a
/// manual upload) and when. Directories without one predate the Store.
pub const METADATA_FILE: &str = ".omnibedrock.json";

/// Read the sidecar if present. `None` means an untagged (treat as manual) pack.
pub fn read_pack_metadata(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join(METADATA_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the provenance sidecar into a pack or world directory.
pub async fn write_pack_metadata(dir: &Path, metadata: Value) -> anyhow::Result<()> {
    let path = dir.join(METADATA_FILE);
    omnibedrock_core::files::atomic_write(&path, &serde_json::to_vec_pretty(&metadata)?).await
}
