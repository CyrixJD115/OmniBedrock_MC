use anyhow::{Context, ensure};
use clap::Parser;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use crate::paths::{self, DataMode, Layout};

#[derive(Parser, Debug)]
#[command(version, about = "OmniBedrock native server control panel")]
pub struct Cli {
    /// Explicit configuration file (.toml or legacy .yaml). Overrides the
    /// auto-discovered one; the file must exist when given.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Portable mode: keep all application data under `<executable dir>/data/`
    /// and read only `<executable dir>/data/config.toml`. Mutually exclusive
    /// with --global-data-dir.
    #[arg(long)]
    pub portable: bool,
    /// Use this directory as the global data root (overriding the platform
    /// app-data location). Mutually exclusive with --portable.
    #[arg(long, env = "OMNI_GLOBAL_DATA_DIR")]
    pub global_data_dir: Option<PathBuf>,
    #[arg(long, env = "OMNI_HOST")]
    pub host: Option<IpAddr>,
    #[arg(long, env = "OMNI_PORT")]
    pub port: Option<u16>,
    #[arg(long, env = "BEDROCK_SERVER_DIR")]
    pub bedrock_server_dir: Option<PathBuf>,
    #[arg(long, env = "DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    #[arg(long, env = "BACKUPS_DIR")]
    pub backups_dir: Option<PathBuf>,
    #[arg(long, env = "LOGS_DIR")]
    pub logs_dir: Option<PathBuf>,
    /// Cache root (CurseForge image cache; default `<data dir>/cache`).
    #[arg(long, env = "CACHE_DIR")]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, env = "ENDSTONE_BIN")]
    pub endstone_bin: Option<PathBuf>,
    #[arg(long, env = "ENDSTONE_PYTHON")]
    pub endstone_python: Option<PathBuf>,
    #[arg(long, env = "BROADCASTER_DIR")]
    pub broadcaster_dir: Option<PathBuf>,
    #[arg(long, env = "BROADCASTER_JAR")]
    pub broadcaster_jar: Option<PathBuf>,
    #[arg(long, env = "JAVA_BIN")]
    pub java_bin: Option<PathBuf>,
    /// Docker image used for instances with the `docker` deployment backend.
    /// Missing local images are built automatically from the generated
    /// Dockerfiles (see deploy/docker).
    #[arg(long, env = "DOCKER_IMAGE_VANILLA")]
    pub docker_image_vanilla: Option<String>,
    #[arg(long, env = "DOCKER_IMAGE_ENDSTONE")]
    pub docker_image_endstone: Option<String>,
    /// Memory (MiB) kept free for the OS and the panel itself; instance
    /// allocations beyond total − reserve trigger overcommit warnings.
    #[arg(long, env = "HOST_RESERVE_MB")]
    pub host_reserve_mb: Option<u64>,
    /// Open the panel in a Tauri desktop window instead of the plain web
    /// server (requires the binary to be built with `--features desktop`).
    #[arg(long)]
    pub desktop: bool,
}

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

/// Everything a configuration file may set. Secrets (API keys, SECRET_KEY)
/// are deliberately excluded — they belong in the environment or `.env`.
/// Unknown keys are rejected so typos cannot pass silently.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
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
    host_reserve_mb: Option<u64>,
    ws_heartbeat_interval: Option<u64>,
    console_max_lines: Option<usize>,
    console_max_bytes: Option<usize>,
    console_max_line_bytes: Option<usize>,
    console_journal_bytes: Option<u64>,
    command_queue_capacity: Option<usize>,
    /// Data-root selection, mirroring the CLI flags. `portable = true` puts
    /// all data under `<executable dir>/data`.
    portable: Option<bool>,
    /// Custom global data root (relative paths resolve against this file's
    /// directory).
    global_data_dir: Option<PathBuf>,
}

fn apply_file(c: &mut Config, file: FileConfig, cache_dir_explicit: &mut bool) {
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
fn parse_file(path: &Path) -> anyhow::Result<FileConfig> {
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

/// Where the directory holding the executable is, when discoverable.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Auto-discovered config file, checked only when `--config` is absent:
/// first next to the executable, then the user-level location on Unix.
/// Discovery never consults the data directory, so data-root settings in a
/// config file cannot create a circular resolution.
fn discover_config(exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = exe_dir {
        let candidate = dir.join("config.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if !cfg!(windows) {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        if let Some(base) = base {
            let candidate = base.join("omnibedrock").join("config.toml");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

impl Config {
    /// Load configuration from defaults, an optional config file, the
    /// environment, and finally CLI flags.
    ///
    /// Data-root precedence (highest wins): `--portable` /
    /// `--global-data-dir` → `portable` / `global_data_dir` in the config
    /// file → the platform app-data directory. Setting both flags (or both
    /// file keys) is an error. Remaining settings follow the usual order:
    /// defaults → config file → environment → CLI.
    pub fn load(cli: Cli) -> anyhow::Result<Self> {
        Self::load_with(&cli, exe_dir().as_deref())
    }

    fn load_with(cli: &Cli, exe_dir: Option<&Path>) -> anyhow::Result<Self> {
        ensure!(
            !(cli.portable && cli.global_data_dir.is_some()),
            "--portable and --global-data-dir are mutually exclusive; pick one data location"
        );
        let mut cache_dir_explicit = false;
        // Step 1: CLI flags choose the mode outright.
        let mut mode = if cli.portable {
            let dir = exe_dir.context(
                "--portable requires locating the running executable to find <exe dir>/data",
            )?;
            Some(DataMode::Portable(paths::portable_root(dir)))
        } else {
            cli.global_data_dir.clone().map(DataMode::Global)
        };
        // Step 2: read the config file. Portable mode reads only
        // `<exe dir>/data/config.toml` so portable and system state never
        // mix; otherwise --config beats exe-dir then user-level discovery.
        let portable_root = match &mode {
            Some(DataMode::Portable(root)) => Some(root.clone()),
            _ => None,
        };
        let file = if let Some(root) = portable_root {
            let path = root.join("config.toml");
            path.is_file().then_some(path)
        } else {
            match &cli.config {
                Some(p) => {
                    ensure!(p.is_file(), "config file {} does not exist", p.display());
                    Some(p.clone())
                }
                None => discover_config(exe_dir),
            }
        };
        let file_config = file.as_ref().map(|p| parse_file(p)).transpose()?;
        // Step 3: without a CLI mode, the file may choose one.
        if mode.is_none()
            && let Some(f) = &file_config
        {
            let portable = f.portable.unwrap_or(false);
            let global = f.global_data_dir.as_ref();
            ensure!(
                !(portable && global.is_some()),
                "config file sets both 'portable' and 'global_data_dir'; pick one data location{}",
                file.map(|p| format!(" ({})", p.display()))
                    .unwrap_or_default()
            );
            mode = if portable {
                let dir = exe_dir.context(
                    "portable = true requires locating the running executable to find <exe dir>/data",
                )?;
                Some(DataMode::Portable(paths::portable_root(dir)))
            } else {
                global.map(|g| {
                    // Relative file roots resolve against the config file's
                    // directory so a config can point at a tree beside itself.
                    let base = file
                        .as_ref()
                        .and_then(|p| p.parent())
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    let path = if g.is_absolute() {
                        g.clone()
                    } else {
                        base.join(g)
                    };
                    DataMode::Global(path)
                })
            };
        }
        // Step 4: fall back to the platform app-data directory.
        let mode = mode.unwrap_or(DataMode::Default);
        let root = match &mode {
            DataMode::Default => paths::platform_data_root()?,
            DataMode::Global(p) | DataMode::Portable(p) => p.clone(),
        };
        let mut c = Self::for_root(&root);
        c.portable = matches!(mode, DataMode::Portable(_));
        c.data_mode = mode.label().into();
        if let Some(path) = &file {
            c.config_source = path.display().to_string();
        } else if let DataMode::Portable(root) = &mode {
            c.config_source = format!("{} (no config.toml)", root.display());
        }
        if let Some(f) = file_config {
            apply_file(&mut c, f, &mut cache_dir_explicit);
        }
        macro_rules! setting {
            ($name:literal, $field:ident) => {
                if let Ok(v) = std::env::var($name) {
                    c.$field = v.parse().with_context(|| format!("invalid {}", $name))?;
                }
            };
        }
        setting!("DEBUG", debug);
        setting!("WS_HEARTBEAT_INTERVAL", ws_heartbeat_interval);
        setting!("CONSOLE_MAX_LINES", console_max_lines);
        setting!("CONSOLE_MAX_BYTES", console_max_bytes);
        setting!("CONSOLE_MAX_LINE_BYTES", console_max_line_bytes);
        setting!("CONSOLE_JOURNAL_BYTES", console_journal_bytes);
        setting!("COMMAND_QUEUE_CAPACITY", command_queue_capacity);
        if let Ok(v) = std::env::var("CORS_ORIGINS") {
            c.cors_origins = if v.starts_with('[') {
                serde_json::from_str(&v)?
            } else {
                v.split(',').map(|s| s.trim().to_owned()).collect()
            };
        }
        if std::env::var_os("CACHE_DIR").is_some() {
            cache_dir_explicit = true;
        }
        if let Some(v) = cli.host {
            c.host = v;
        }
        if let Some(v) = cli.port {
            c.port = v;
        }
        if let Some(v) = cli.bedrock_server_dir.clone() {
            c.bedrock_server_dir = v;
        }
        if let Some(v) = cli.data_dir.clone() {
            c.data_dir = v;
        }
        if let Some(v) = cli.backups_dir.clone() {
            c.backups_dir = v;
        }
        if let Some(v) = cli.logs_dir.clone() {
            c.logs_dir = v;
        }
        if let Some(v) = cli.cache_dir.clone() {
            c.cache_dir = v;
            cache_dir_explicit = true;
        }
        if let Some(v) = cli.endstone_bin.clone() {
            c.endstone_bin = Some(v);
        }
        if let Some(v) = cli.endstone_python.clone() {
            c.endstone_python = Some(v);
        }
        if let Some(v) = cli.broadcaster_dir.clone() {
            c.broadcaster_dir = v;
        }
        if let Some(v) = cli.broadcaster_jar.clone() {
            c.broadcaster_jar = Some(v);
        }
        if let Some(v) = cli.java_bin.clone() {
            c.java_bin = Some(v);
        }
        if let Some(v) = cli.docker_image_vanilla.clone() {
            c.docker_image_vanilla = v;
        }
        if let Some(v) = cli.docker_image_endstone.clone() {
            c.docker_image_endstone = v;
        }
        if let Some(v) = cli.host_reserve_mb {
            c.host_reserve_mb = v;
        }
        // The cache root follows the data dir unless it was set explicitly,
        // so relocating data never leaves the cache behind.
        if !cache_dir_explicit {
            c.cache_dir = c.data_dir.join("cache");
        }
        c.secret_key = std::env::var("SECRET_KEY").unwrap_or_default().into();
        c.curseforge_api_key = std::env::var("CURSEFORGE_API_KEY")
            .unwrap_or_default()
            .into();
        ensure!(
            c.console_max_lines > 0 && c.command_queue_capacity > 0 && c.ws_heartbeat_interval > 0,
            "capacities and heartbeat must be positive"
        );
        ensure!(
            c.console_max_line_bytes >= 1024 && c.console_max_bytes >= c.console_max_line_bytes,
            "console byte limits are too small"
        );
        ensure!(
            c.console_journal_bytes >= 4 * 1024 * 1024,
            "console journal must have at least 4 MiB"
        );
        ensure!(
            c.api_prefix.starts_with('/') && !c.api_prefix.ends_with('/'),
            "api_prefix must start with / and have no trailing slash"
        );
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> Cli {
        Cli {
            config: None,
            portable: false,
            global_data_dir: None,
            host: None,
            port: None,
            bedrock_server_dir: None,
            data_dir: None,
            backups_dir: None,
            logs_dir: None,
            cache_dir: None,
            endstone_bin: None,
            endstone_python: None,
            broadcaster_dir: None,
            broadcaster_jar: None,
            java_bin: None,
            docker_image_vanilla: None,
            docker_image_endstone: None,
            host_reserve_mb: None,
            desktop: false,
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn defaults_without_any_files_use_platform_root() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load_with(&cli(), Some(dir.path())).unwrap();
        assert!(!c.portable);
        assert_eq!(c.data_mode, "default");
        assert_eq!(c.config_source, "defaults");
        assert_eq!(c.port, 17754);
        // Every path derives from the platform root (absolute on every host).
        assert!(c.data_dir.is_absolute());
        assert_eq!(c.bedrock_server_dir, c.data_dir.join("server"));
        assert_eq!(c.backups_dir, c.data_dir.join("backups"));
        assert_eq!(c.logs_dir, c.data_dir.join("logs"));
        assert_eq!(c.cache_dir, c.data_dir.join("cache"));
        assert_eq!(c.broadcaster_dir, c.data_dir.join("broadcaster"));
    }

    #[test]
    fn global_data_dir_cli_overrides_platform_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cli();
        c.global_data_dir = Some(PathBuf::from("/srv/omb-data"));
        let config = Config::load_with(&c, Some(dir.path())).unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/srv/omb-data"));
        assert_eq!(config.data_mode, "global-data-dir");
        assert_eq!(
            config.bedrock_server_dir,
            PathBuf::from("/srv/omb-data/server")
        );
        assert_eq!(config.cache_dir, PathBuf::from("/srv/omb-data/cache"));
    }

    #[test]
    fn global_data_dir_from_config_file_resolves_relative_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, "global_data_dir = \"storage\"\nport = 18001\n");
        let mut c = cli();
        c.config = Some(path);
        let config = Config::load_with(&c, Some(dir.path())).unwrap();
        assert_eq!(config.data_mode, "global-data-dir");
        assert_eq!(config.data_dir, dir.path().join("storage"));
        assert_eq!(config.port, 18001);
        // The layout followed the file-provided root.
        assert_eq!(config.logs_dir, dir.path().join("storage/logs"));
    }

    #[test]
    fn portable_flag_uses_data_dir_next_to_exe() {
        let exe = tempfile::tempdir().unwrap();
        let mut c = cli();
        c.portable = true;
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(config.data_mode, "portable");
        assert_eq!(config.data_dir, exe.path().join("data"));
        assert_eq!(config.bedrock_server_dir, exe.path().join("data/server"));
        assert_eq!(config.logs_dir, exe.path().join("data/logs"));
        assert_eq!(config.backups_dir, exe.path().join("data/backups"));
    }

    #[test]
    fn portable_reads_only_its_own_config() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("data/config.toml"), "port = 18000\n");
        write(&exe.path().join("config.toml"), "port = 19999\n");
        let mut c = cli();
        c.portable = true;
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(config.port, 18000, "portable reads <exe>/data/config.toml");
        assert!(
            config.config_source.contains("data/config.toml"),
            "exe-dir config must not leak into portable mode: {}",
            config.config_source
        );
    }

    #[test]
    fn portable_from_config_file_key() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("config.toml"), "portable = true\n");
        let config = Config::load_with(&cli(), Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(config.data_dir, exe.path().join("data"));
    }

    #[test]
    fn portable_and_global_conflict_rejected() {
        let mut c = cli();
        c.portable = true;
        c.global_data_dir = Some(PathBuf::from("/srv/x"));
        let err = Config::load_with(&c, None)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn file_level_conflict_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, "portable = true\nglobal_data_dir = \"/srv/x\"\n");
        let mut c = cli();
        c.config = Some(path);
        let err = Config::load_with(&c, Some(dir.path()))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("pick one data location"), "{err}");
    }

    #[test]
    fn cli_portable_beats_file_global_data_dir() {
        let exe = tempfile::tempdir().unwrap();
        write(
            &exe.path().join("data/config.toml"),
            "global_data_dir = \"/elsewhere\"\n",
        );
        let mut c = cli();
        c.portable = true;
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(config.data_dir, exe.path().join("data"));
    }

    #[test]
    fn cli_global_data_dir_beats_file_portable() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("config.toml"), "portable = true\n");
        let mut c = cli();
        c.global_data_dir = Some(PathBuf::from("/srv/wanted"));
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(!config.portable);
        assert_eq!(config.data_dir, PathBuf::from("/srv/wanted"));
    }

    #[test]
    fn discovered_exe_config_applies() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("config.toml"), "port = 18111\n");
        let config = Config::load_with(&cli(), Some(exe.path())).unwrap();
        assert_eq!(config.port, 18111);
        assert_eq!(
            config.config_source,
            exe.path().join("config.toml").display().to_string()
        );
    }

    #[test]
    fn explicit_config_yaml_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.yaml");
        write(&path, "port: 18222\n");
        let mut c = cli();
        c.config = Some(path.clone());
        let config = Config::load_with(&c, Some(dir.path())).unwrap();
        assert_eq!(config.port, 18222);
    }

    #[test]
    fn explicit_config_missing_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cli();
        c.config = Some(dir.path().join("nope.toml"));
        assert!(Config::load_with(&c, Some(dir.path())).is_err());
    }

    #[test]
    fn cli_overrides_config_file() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("config.toml"), "port = 18111\n");
        let mut c = cli();
        c.port = Some(12345);
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert_eq!(config.port, 12345);
    }

    #[test]
    fn unknown_config_key_rejected() {
        let exe = tempfile::tempdir().unwrap();
        write(
            &exe.path().join("config.toml"),
            "port = 18111\nnonsense_key = 1\n",
        );
        assert!(Config::load_with(&cli(), Some(exe.path())).is_err());
    }

    #[test]
    fn unsupported_config_extension_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write(&path, "{}");
        let mut c = cli();
        c.config = Some(path);
        assert!(
            Config::load_with(&c, Some(dir.path()))
                .err()
                .unwrap()
                .to_string()
                .contains("unsupported config format")
        );
    }

    #[tokio::test]
    async fn ensure_dirs_creates_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::for_root(&dir.path().join("root"));
        config.ensure_dirs().await.unwrap();
        for d in [
            &config.data_dir,
            &config.bedrock_server_dir,
            &config.backups_dir,
            &config.logs_dir,
            &config.cache_dir,
            &config.broadcaster_dir,
        ] {
            assert!(d.is_dir(), "{} missing", d.display());
        }
    }
}
