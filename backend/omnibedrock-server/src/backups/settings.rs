//! Backup settings document: defaults, shallow merge, and validation.

use omnibedrock_core::error::{ApiError, Result};
use serde_json::{Value, json};

use super::types::Hooks;

pub(super) fn defaults() -> Value {
    json!({"manual":{"world":"","full_backup":true,"zip_prefix":"manual_backup","export_folder":"","compression":"deflate","dry_run":false,"include_items":[]},"auto":{"enabled":false,"interval_minutes":30,"keep_count":10,"export_folder":"","compression":"deflate","full_backup":true,"include_items":[],"worlds":[]},"pre_post":{"before":[],"after":[]}})
}
pub(super) fn merge(base: &mut Value, patch: &Value) {
    if let (Some(a), Some(b)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
}
pub(super) fn validate_settings(value: &Value) -> Result<()> {
    let hooks: Hooks = serde_json::from_value(value["pre_post"].clone())
        .map_err(|e| ApiError::bad(e.to_string()))?;
    hooks.validate(false)?;
    let auto = &value["auto"];
    if !auto["interval_minutes"]
        .as_u64()
        .is_some_and(|n| (1..=525600).contains(&n))
        || !auto["keep_count"]
            .as_u64()
            .is_some_and(|n| (1..=10000).contains(&n))
    {
        return Err(ApiError::bad("Invalid backup interval or retention count"));
    }
    if auto["enabled"] == true {
        hooks.validate(true)?;
    }
    for section in ["auto", "manual"] {
        if !["deflate", "store"].contains(&value[section]["compression"].as_str().unwrap_or("")) {
            return Err(ApiError::bad("Invalid compression"));
        }
    }
    Ok(())
}
