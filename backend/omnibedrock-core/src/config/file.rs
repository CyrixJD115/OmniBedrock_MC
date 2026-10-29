//! The on-disk configuration file: the typed `FileConfig` schema, merging a
//! parsed file into the runtime `Config`, and extension-based parsing
//! (`.toml` preferred, legacy `.yaml`/`.yml` accepted).

use anyhow::Context;
use serde::Deserialize;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use super::model::Config;

/// Everything a configuration file may set. Secrets (API keys, SECRET_KEY)
/// are deliberately excluded — they belong in the environment or `.env`.
/// Unknown keys are rejected so typos cannot pass silently.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileConfig {
    app_name: Option<String>,
    debug: Option<bool>,
    host: Option<IpAddr>,
    port: Option<u16>,
    api_prefix: Option<String>,
    cors_origins: Option<Vec<String>>,
    bedrock_server_dir: Option<PathBuf>,
    backups_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    logs_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    endstone_bin: Option<PathBuf>,
    endstone_python: Option<PathBuf>,
    broadcaster_dir: Option<PathBuf>,
    broadcaster_jar: Option<PathBuf>,
    java_bin: Option<PathBuf>,
    docker_image_vanilla: Option<String>,
    docker_image_endstone: Option<String>,
    docker_image_broadcaster: Option<String>,
    host_reserve_mb: Option<u64>,
    ws_heartbeat_interval: Option<u64>,
    console_max_lines: Option<usize>,
    console_max_bytes: Option<usize>,
    console_max_line_bytes: Option<usize>,
    console_journal_bytes: Option<u64>,
    command_queue_capacity: Option<usize>,
    /// Data-root selection, mirroring the CLI flags. `portable = true` puts
    /// all data under `<executable dir>/data`.
    pub(super) portable: Option<bool>,
    /// Custom global data root (relative paths resolve against this file's
    /// directory).
    pub(super) global_data_dir: Option<PathBuf>,
}

pub(super) fn apply_file(c: &mut Config, file: FileConfig, cache_dir_explicit: &mut bool) {
    if let Some(v) = file.app_name {
        c.app_name = v;
    }
    if let Some(v) = file.debug {
        c.debug = v;
    }
    if let Some(v) = file.host {
        c.host = v;
    }
    if let Some(v) = file.port {
        c.port = v;
    }
    if let Some(v) = file.api_prefix {
        c.api_prefix = v;
    }
    if let Some(v) = file.cors_origins {
        c.cors_origins = v;
    }
    if let Some(v) = file.bedrock_server_dir {
        c.bedrock_server_dir = v;
    }
    if let Some(v) = file.backups_dir {
        c.backups_dir = v;
    }
    if let Some(v) = file.data_dir {
        c.data_dir = v;
    }
    if let Some(v) = file.logs_dir {
        c.logs_dir = v;
    }
    if let Some(v) = file.cache_dir {
        c.cache_dir = v;
        *cache_dir_explicit = true;
    }
    if let Some(v) = file.endstone_bin {
        c.endstone_bin = Some(v);
    }
    if let Some(v) = file.endstone_python {
        c.endstone_python = Some(v);
    }
    if let Some(v) = file.broadcaster_dir {
        c.broadcaster_dir = v;
    }
    if let Some(v) = file.broadcaster_jar {
        c.broadcaster_jar = Some(v);
    }
    if let Some(v) = file.java_bin {
        c.java_bin = Some(v);
    }
    if let Some(v) = file.docker_image_vanilla {
        c.docker_image_vanilla = v;
    }
    if let Some(v) = file.docker_image_endstone {
        c.docker_image_endstone = v;
    }
    if let Some(v) = file.docker_image_broadcaster {
        c.docker_image_broadcaster = v;
    }
    if let Some(v) = file.host_reserve_mb {
        c.host_reserve_mb = v;
    }
    if let Some(v) = file.ws_heartbeat_interval {
        c.ws_heartbeat_interval = v;
    }
    if let Some(v) = file.console_max_lines {
        c.console_max_lines = v;
    }
    if let Some(v) = file.console_max_bytes {
        c.console_max_bytes = v;
    }
    if let Some(v) = file.console_max_line_bytes {
        c.console_max_line_bytes = v;
    }
    if let Some(v) = file.console_journal_bytes {
        c.console_journal_bytes = v;
    }
    if let Some(v) = file.command_queue_capacity {
        c.command_queue_capacity = v;
    }
}

/// Parse a configuration file by extension: `.toml` (preferred) or the
/// legacy `.yaml`/`.yml`. Anything else is refused instead of guessed.
pub(super) fn parse_file(path: &Path) -> anyhow::Result<FileConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read config file {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "toml" => {
            toml::from_str(&text).with_context(|| format!("parse config file {}", path.display()))
        }
        "yaml" | "yml" => serde_yaml::from_str(&text)
            .with_context(|| format!("parse config file {}", path.display())),
        other => anyhow::bail!(
            "unsupported config format '{other}' in {} (use .toml, or legacy .yaml)",
            path.display()
        ),
    }
}
