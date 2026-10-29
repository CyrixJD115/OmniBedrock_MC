//! Layout of the uv toolchain pieces under (or, for space-sensitive paths,
//! relocated away from) the panel data dir.
use omnibedrock_core::config::Config;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ─── layout under the panel data dir ────────────────────────────────────────
pub fn uv_dir(config: &Config) -> PathBuf {
    config.data_dir.join("tools").join("uv")
}
pub fn uv_binary(config: &Config) -> PathBuf {
    uv_dir(config).join(if cfg!(windows) { "uv.exe" } else { "uv" })
}
/// Root for the toolchain pieces whose paths leak into the launched server's
/// environment. On Linux, Endstone's launcher injects its runtime through
/// LD_PRELOAD and LD_LIBRARY_PATH built from its own install location and the
/// interpreter's sysconfig — and ld.so splits both variables on spaces, so a
/// data dir with spaces (e.g. "Minecraft Server Panel") silently disables the
/// Endstone runtime. The managed CPython, the Endstone venv, and the uv cache
/// therefore live at a space-free location keyed by a digest of the data dir;
/// temp is the last resort (a reboot there just triggers a quick automatic
/// reinstall on next start). The uv binary itself is only exec'd, so it stays
/// under the data dir.
pub(super) fn runtime_dir(config: &Config) -> PathBuf {
    let preferred = config.data_dir.join("tools");
    if cfg!(windows) || !contains_whitespace(&preferred) {
        return preferred;
    }
    let digest = Sha256::digest(config.data_dir.to_string_lossy().as_bytes());
    let name = format!("omb-{}", hex::encode(&digest[..4]));
    for base in [
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")),
    ]
    .into_iter()
    .flatten()
    {
        if !base.as_os_str().is_empty() && !contains_whitespace(&base) {
            return base.join("omnibedrock").join(name);
        }
    }
    std::env::temp_dir().join(name)
}
pub fn python_install_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("python")
}
pub fn uv_cache_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("uv-cache")
}
pub fn venv_dir(config: &Config) -> PathBuf {
    runtime_dir(config).join("endstone")
}
fn contains_whitespace(path: &Path) -> bool {
    path.to_string_lossy().chars().any(char::is_whitespace)
}
/// The venv's `endstone` console script, used by the server supervisor.
pub fn endstone_binary(config: &Config) -> PathBuf {
    venv_dir(config)
        .join(if cfg!(windows) { "Scripts" } else { "bin" })
        .join(if cfg!(windows) {
            "endstone.exe"
        } else {
            "endstone"
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_relocates_only_for_spaced_data_dirs() {
        let mut config = Config {
            data_dir: PathBuf::from("/tmp/omb-clean/data"),
            ..Default::default()
        };
        assert_eq!(
            venv_dir(&config),
            PathBuf::from("/tmp/omb-clean/data/tools/endstone")
        );

        config.data_dir = PathBuf::from("/tmp/omb space test/data");
        for dir in [
            venv_dir(&config),
            python_install_dir(&config),
            uv_cache_dir(&config),
        ] {
            assert!(
                !contains_whitespace(&dir),
                "relocated runtime dir must be space-free, got {dir:?}"
            );
            assert_eq!(dir, dir, "paths must be deterministic");
        }
        assert_eq!(venv_dir(&config), venv_dir(&config));
    }
}
