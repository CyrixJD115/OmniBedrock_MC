//! Configuration loading: auto-discovery of the config file and the layered
//! `Config::load` pipeline that resolves the data root (portable / global /
//! platform default), merges defaults → config file → environment → CLI,
//! and validates the result. The tests for that pipeline live here too.

use anyhow::{Context, ensure};
use std::path::{Path, PathBuf};

use crate::paths::{self, DataMode};

use super::cli::Cli;
use super::file::{apply_file, parse_file};
use super::model::Config;

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
        fn env_flag(name: &str) -> anyhow::Result<bool> {
            match std::env::var(name).map(|v| v.trim().to_ascii_lowercase()) {
                Ok(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => Ok(true),
                Ok(v) if matches!(v.as_str(), "0" | "false" | "no" | "off" | "") => Ok(false),
                Ok(v) => anyhow::bail!("{name} must be a boolean (got '{v}')"),
                Err(_) => Ok(false),
            }
        }
        c.docker_allow_host_mounts = env_flag("DOCKER_ALLOW_HOST_MOUNTS")?;
        c.backup_hooks_enabled = env_flag("BACKUP_HOOKS_ENABLED")?;
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
        if let Some(v) = cli.docker_image_broadcaster.clone() {
            c.docker_image_broadcaster = v;
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
            docker_image_broadcaster: None,
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
