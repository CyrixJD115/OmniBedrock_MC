use anyhow::{Context, ensure};
use clap::Parser;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

#[derive(Parser, Debug)]
#[command(version, about = "OmniBedrock native server control panel")]
pub struct Cli {
    /// Explicit configuration file (.toml or legacy .yaml). Overrides the
    /// auto-discovered one; the file must exist when given.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Portable mode: keep config, data, server files, logs, and backups
    /// under `<executable dir>/portable/` and ignore other config locations.
    #[arg(long)]
    pub portable: bool,
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
    /// True when `--portable` resolved the layout. Never serialized.
    #[serde(skip)]
    pub portable: bool,
    /// Where the effective configuration came from (diagnostics only).
    #[serde(skip)]
    pub config_source: String,
}

impl Default for Config {
    fn default() -> Self {
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
            bedrock_server_dir: "bedrock_server".into(),
            backups_dir: "backend/backups".into(),
            data_dir: "backend/data".into(),
            logs_dir: "backend/logs".into(),
            cache_dir: "backend/data/cache".into(),
            endstone_bin: None,
            endstone_python: None,
            broadcaster_dir: "backend/broadcaster".into(),
            broadcaster_jar: None,
            java_bin: None,
            ws_heartbeat_interval: 30,
            console_max_lines: 2000,
            console_max_bytes: 16 * 1024 * 1024,
            console_max_line_bytes: 64 * 1024,
            console_journal_bytes: 256 * 1024 * 1024,
            command_queue_capacity: 256,
            secret_key: SecretString::new(String::new()),
            curseforge_api_key: SecretString::new(String::new()),
            portable: false,
            config_source: "defaults".into(),
        }
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
    ws_heartbeat_interval: Option<u64>,
    console_max_lines: Option<usize>,
    console_max_bytes: Option<usize>,
    console_max_line_bytes: Option<usize>,
    console_journal_bytes: Option<u64>,
    command_queue_capacity: Option<usize>,
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
    /// environment, and finally CLI flags. Precedence (lowest to highest):
    /// defaults → portable layout → config file → environment → CLI.
    pub fn load(cli: Cli) -> anyhow::Result<Self> {
        Self::load_with(&cli, exe_dir().as_deref())
    }

    fn load_with(cli: &Cli, exe_dir: Option<&Path>) -> anyhow::Result<Self> {
        let mut c = Self::default();
        let mut cache_dir_explicit = false;
        if cli.portable {
            let base = exe_dir
                .context("--portable requires locating the running executable")?
                .join("portable");
            // Portable mode keeps everything under one tree and deliberately
            // ignores the exe-dir and user-level config files so portable and
            // system state can never mix.
            c.portable = true;
            c.data_dir = base.join("data");
            c.bedrock_server_dir = base.join("server");
            c.logs_dir = base.join("logs");
            c.backups_dir = base.join("backups");
            let path = base.join("config.toml");
            if path.is_file() {
                c.config_source = path.display().to_string();
                apply_file(&mut c, parse_file(&path)?, &mut cache_dir_explicit);
            } else {
                c.config_source = format!("{} (no config.toml)", base.display());
            }
        } else {
            let path = match &cli.config {
                Some(p) => {
                    ensure!(p.is_file(), "config file {} does not exist", p.display());
                    Some(p.clone())
                }
                None => discover_config(exe_dir),
            };
            if let Some(path) = path {
                c.config_source = path.display().to_string();
                apply_file(&mut c, parse_file(&path)?, &mut cache_dir_explicit);
            }
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
            desktop: false,
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn defaults_without_any_files() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load_with(&cli(), Some(dir.path())).unwrap();
        assert!(!c.portable);
        assert_eq!(c.config_source, "defaults");
        assert_eq!(c.port, 17754);
        assert_eq!(c.cache_dir, PathBuf::from("backend/data/cache"));
    }

    #[test]
    fn portable_layout_and_config_file() {
        let exe = tempfile::tempdir().unwrap();
        write(
            &exe.path().join("portable/config.toml"),
            "port = 18000\ndata_dir = \"/custom/data\"\n",
        );
        let mut c = cli();
        c.portable = true;
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(config.port, 18000);
        assert_eq!(config.data_dir, PathBuf::from("/custom/data"));
        assert_eq!(
            config.bedrock_server_dir,
            exe.path().join("portable/server")
        );
        assert_eq!(config.logs_dir, exe.path().join("portable/logs"));
        assert_eq!(config.backups_dir, exe.path().join("portable/backups"));
        // cache follows the relocated data dir
        assert_eq!(config.cache_dir, PathBuf::from("/custom/data/cache"));
        assert!(config.config_source.contains("portable/config.toml"));
    }

    #[test]
    fn portable_ignores_exe_dir_config() {
        let exe = tempfile::tempdir().unwrap();
        write(&exe.path().join("config.toml"), "port = 19999\n");
        let mut c = cli();
        c.portable = true;
        let config = Config::load_with(&c, Some(exe.path())).unwrap();
        assert!(config.portable);
        assert_eq!(
            config.port, 17754,
            "exe-dir config must not leak into portable mode"
        );
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
}
