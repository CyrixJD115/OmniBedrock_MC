//! Pack localization: `.lang` table parsing and manifest string resolution.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Parse a Bedrock `.lang` file: `key=value` lines, `#` comment lines, and
/// blank lines. Later duplicate keys win, matching the game's behavior.
/// Marketplace packs end values with a whitespace-then-`#` comment marker
/// (`…Pack\t#`); a `#` preceded by whitespace starts a trailing comment and
/// is stripped, while a glued `#` stays part of the value.
pub fn parse_lang(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = match value.find('#') {
                Some(idx) if idx > 0 && value[..idx].ends_with(char::is_whitespace) => {
                    &value[..idx]
                }
                _ => value,
            };
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// Read a pack's localization table from `texts/en_US.lang`, falling back to
/// the first `*.lang` in sorted order when en_US is absent. Marketplace-style
/// packs put localization keys like `pack.name` in the manifest and the real
/// strings here. Empty map when the pack ships no texts.
pub async fn read_texts(pack_dir: &Path) -> HashMap<String, String> {
    let texts = pack_dir.join("texts");
    let mut candidate = texts.join("en_US.lang");
    if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&texts).await {
            while let Ok(entry) = entries.next_entry().await {
                let Some(entry) = entry else { break };
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "lang") {
                    files.push(path);
                }
            }
        }
        files.sort();
        match files.first() {
            Some(path) => candidate = path.clone(),
            None => return HashMap::new(),
        }
    }
    match tokio::fs::read_to_string(&candidate).await {
        Ok(content) => parse_lang(&content),
        Err(_) => HashMap::new(),
    }
}

/// Resolve a manifest string against the pack's lang table: keys like
/// `pack.name` become their translation, plain strings pass through.
pub fn resolve_lang(texts: &HashMap<String, String>, raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    Some(texts.get(raw).cloned().unwrap_or_else(|| raw.to_string()))
}
