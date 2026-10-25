//! Centralized data-directory resolution. Every runtime path the panel uses
//! (settings, instances, backups, logs, caches, toolchains) derives from one
//! data root resolved here — never from the source tree or the working
//! directory the binary happened to be launched from.
//!
//! Resolution order (highest wins):
//!
//! 1. `--portable` / `portable = true`  → `<executable dir>/data/`
//! 2. `--global-data-dir <path>` / `global_data_dir` → the given root
//! 3. default                           → the platform application-data dir
//!    (`~/.local/share/omnibedrock` on Linux, `%LOCALAPPDATA%\OmniBedrock`
//!    on Windows, `~/Library/Application Support/omnibedrock` on macOS)
//!
//! Supplying `--portable` and `--global-data-dir` together (or setting both
//! keys in one config file) is rejected with an explicit error; a CLI flag
//! always beats the same-named config-file setting.

use std::path::{Path, PathBuf};

/// Subdirectory name under the executable directory that portable mode uses.
pub const PORTABLE_DIR: &str = "data";

/// How the data root was chosen. Kept for diagnostics and boot logging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataMode {
    /// Platform application-data directory.
    Default,
    /// `--global-data-dir` / `global_data_dir`.
    Global(PathBuf),
    /// `--portable` / `portable = true`: `<exe dir>/data`.
    Portable(PathBuf),
}

impl DataMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Global(_) => "global-data-dir",
            Self::Portable(_) => "portable",
        }
    }
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("$HOME is not set; cannot locate the user home directory"))
}

/// The platform's default application-data root for OmniBedrock.
///
/// - Linux (and other XDG Unix): `$XDG_DATA_HOME/omnibedrock`, else
///   `~/.local/share/omnibedrock`
/// - Windows: `%LOCALAPPDATA%\OmniBedrock`
/// - macOS: `~/Library/Application Support/omnibedrock`
pub fn platform_data_root() -> anyhow::Result<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| {
                anyhow::anyhow!("%LOCALAPPDATA% is not set; cannot locate the app-data directory")
            })?;
        Ok(base.join("OmniBedrock"))
    } else if cfg!(target_os = "macos") {
        Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join("omnibedrock"))
    } else {
        let base = match std::env::var_os("XDG_DATA_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => home_dir()?.join(".local").join("share"),
        };
        Ok(base.join("omnibedrock"))
    }
}

/// Portable data root: a `data/` directory next to the actual executable,
/// independent of the current working directory.
pub fn portable_root(exe_dir: &Path) -> PathBuf {
    exe_dir.join(PORTABLE_DIR)
}

/// The canonical layout derived from one data root. Explicit per-path
/// overrides (config file, environment, CLI) may still replace individual
/// entries, but these are the values used when nothing overrides them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub data_dir: PathBuf,
    pub bedrock_server_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub broadcaster_dir: PathBuf,
}

impl Layout {
    pub fn from_root(root: &Path) -> Self {
        Self {
            data_dir: root.to_path_buf(),
            bedrock_server_dir: root.join("server"),
            backups_dir: root.join("backups"),
            logs_dir: root.join("logs"),
            cache_dir: root.join("cache"),
            broadcaster_dir: root.join("broadcaster"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_derives_every_path_from_the_root() {
        let l = Layout::from_root(Path::new("/srv/omb"));
        assert_eq!(l.data_dir, Path::new("/srv/omb"));
        assert_eq!(l.bedrock_server_dir, Path::new("/srv/omb/server"));
        assert_eq!(l.backups_dir, Path::new("/srv/omb/backups"));
        assert_eq!(l.logs_dir, Path::new("/srv/omb/logs"));
        assert_eq!(l.cache_dir, Path::new("/srv/omb/cache"));
        assert_eq!(l.broadcaster_dir, Path::new("/srv/omb/broadcaster"));
    }

    #[test]
    fn portable_root_is_exe_anchored() {
        assert_eq!(
            portable_root(Path::new("/opt/omnibedrock")),
            Path::new("/opt/omnibedrock/data")
        );
    }

    #[test]
    fn linux_default_uses_xdg_or_home() {
        // The exact value depends on the test environment's HOME/XDG vars;
        // what must hold is the suffix and that it always resolves.
        let root = platform_data_root().unwrap();
        assert!(root.ends_with("omnibedrock"));
        assert!(root.is_absolute(), "default data root must be absolute");
    }
}
