//! The runtime configuration model: every setting the panel uses, its
//! defaults derived from one data root, and the boot-time directory
//! creation that prepares a fresh installation.

use anyhow::Context;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use crate::paths::{self, DataMode, Layout};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub app_name: String,
    pub debug: bool,
    pub host: IpAddr,
    pub port: u16,
    pub api_prefix: String,
    pub cors_origins: Vec<String>,
    pub bedrock_server_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    /// Cache root; defaults to `<data_dir>/cache` unless set explicitly.
    pub cache_dir: PathBuf,
    pub endstone_bin: Option<PathBuf>,
    /// Legacy fallback: a Python interpreter with the `endstone` module
    /// installed. Only used when no endstone executable exists anywhere;
    /// the module is probed before this is trusted.
    pub endstone_python: Option<PathBuf>,
    pub broadcaster_dir: PathBuf,
    pub broadcaster_jar: Option<PathBuf>,
    pub java_bin: Option<PathBuf>,
    /// Docker images used by the `docker` deployment backend (one per runtime).
    pub docker_image_vanilla: String,
    pub docker_image_endstone: String,
    /// Docker image for the broadcaster sidecar container.
    pub docker_image_broadcaster: String,
    /// Headroom reserved for the OS/panel when judging overcommit (MiB).
    pub host_reserve_mb: u64,
    pub ws_heartbeat_interval: u64,
    pub console_max_lines: usize,
    pub console_max_bytes: usize,
    pub console_max_line_bytes: usize,
    pub console_journal_bytes: u64,
    pub command_queue_capacity: usize,
    #[serde(skip)]
    pub secret_key: SecretString,
    /// CurseForge Core API key; empty disables the Store.
    #[serde(skip)]
    pub curseforge_api_key: SecretString,
    /// True when the data root is `<executable dir>/data` (portable mode).
    /// Never serialized.
    #[serde(skip)]
    pub portable: bool,
    /// How the data root was resolved (diagnostics only).
    #[serde(skip)]
    pub data_mode: String,
    /// Where the effective configuration came from (diagnostics only).
    #[serde(skip)]
    pub config_source: String,
}

impl Config {
    /// Defaults derived from one data root. This is the single place that
    /// knows how the layout under the root looks; explicit per-path settings
    /// layer on top of it.
    pub fn for_root(root: &Path) -> Self {
        let layout = Layout::from_root(root);
        Self {
            app_name: "OmniBedrock MC".into(),
            debug: false,
            host: "0.0.0.0".parse().expect("literal IP"),
            port: 17754,
            api_prefix: "/api/v1".into(),
            cors_origins: vec![
                "http://localhost:17755".into(),
                "http://localhost:5173".into(),
                "http://localhost:3000".into(),
            ],
            bedrock_server_dir: layout.bedrock_server_dir,
            backups_dir: layout.backups_dir,
            data_dir: layout.data_dir,
            logs_dir: layout.logs_dir,
            cache_dir: layout.cache_dir,
            endstone_bin: None,
            endstone_python: None,
            broadcaster_dir: layout.broadcaster_dir,
            broadcaster_jar: None,
            java_bin: None,
            docker_image_vanilla: "omnibedrock-vanilla:latest".into(),
            docker_image_endstone: "omnibedrock-endstone:latest".into(),
            docker_image_broadcaster: "omnibedrock-broadcaster:latest".into(),
            host_reserve_mb: 1024,
            ws_heartbeat_interval: 30,
            console_max_lines: 2000,
            console_max_bytes: 16 * 1024 * 1024,
            console_max_line_bytes: 64 * 1024,
            console_journal_bytes: 256 * 1024 * 1024,
            command_queue_capacity: 256,
            secret_key: SecretString::new(String::new()),
            curseforge_api_key: SecretString::new(String::new()),
            portable: false,
            data_mode: DataMode::Default.label().into(),
            config_source: "defaults".into(),
        }
    }

    /// Create every directory the panel writes to under the resolved layout.
    /// Called once at boot so a fresh installation is ready to use.
    pub async fn ensure_dirs(&self) -> anyhow::Result<()> {
        for dir in [
            &self.data_dir,
            &self.bedrock_server_dir,
            &self.backups_dir,
            &self.logs_dir,
            &self.cache_dir,
            &self.broadcaster_dir,
        ] {
            tokio::fs::create_dir_all(dir)
                .await
                .with_context(|| format!("create data directory {}", dir.display()))?;
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        // Default construction is only a fallback shape (tests override the
        // path fields explicitly); `Config::load` computes the real root with
        // proper error reporting when the platform directory is unavailable.
        let root =
            paths::platform_data_root().unwrap_or_else(|_| PathBuf::from("omnibedrock-data"));
        Self::for_root(&root)
    }
}
