//! Backup request payloads and pre/post command-hook configuration types.

use omnibedrock_core::error::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupRequest {
    pub world: String,
    pub tag: String,
    pub full_backup: bool,
    pub zip_prefix: String,
    pub export_folder: String,
    pub compression: String,
    pub include_items: Option<Vec<String>>,
    pub dry_run: bool,
    pub run_hooks: bool,
}
impl Default for BackupRequest {
    fn default() -> Self {
        Self {
            world: String::new(),
            tag: "manual".into(),
            full_backup: true,
            zip_prefix: "manual_backup".into(),
            export_folder: String::new(),
            compression: "deflate".into(),
            include_items: None,
            dry_run: false,
            run_hooks: true,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Hook {
    pub r#type: String,
    pub value: Value,
}
impl Hook {
    pub(super) fn text(&self) -> String {
        self.value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.value.to_string())
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Hooks {
    pub before: Vec<Hook>,
    pub after: Vec<Hook>,
}
impl Hooks {
    pub fn validate(&self, require_save: bool) -> Result<()> {
        for h in self.before.iter().chain(&self.after) {
            if !["command", "wait", "comment", "send"].contains(&h.r#type.as_str()) {
                return Err(ApiError::bad("Invalid command type"));
            }
            if h.r#type == "wait"
                && !h
                    .value
                    .as_u64()
                    .or_else(|| h.value.as_str()?.parse().ok())
                    .is_some_and(|n| (1..=600).contains(&n))
            {
                return Err(ApiError::bad("wait value must be 1..600 seconds"));
            }
            if ["send", "command", "comment"].contains(&h.r#type.as_str()) && !h.value.is_string() {
                return Err(ApiError::bad("Command value must be a string"));
            }
        }
        if require_save
            && (!self
                .before
                .iter()
                .any(|h| h.r#type == "send" && h.text() == "save hold")
                || !self
                    .after
                    .iter()
                    .any(|h| h.r#type == "send" && h.text() == "save resume"))
        {
            return Err(ApiError::bad(
                "Before commands must include 'save hold' (send type) and After commands must include 'save resume' (send type)",
            ));
        }
        Ok(())
    }
}
