//! Resolution of how the supervisor would launch Endstone: explicit
//! override, the managed venv, PATH lookup, or a probed Python module.
use omnibedrock_core::config::Config;
use std::path::{Path, PathBuf};

use super::paths::endstone_binary;

// ─── launch candidate resolution ────────────────────────────────────────────
/// How the supervisor would launch Endstone right now.
#[derive(Debug)]
pub enum EndstoneLaunch {
    /// An `endstone` executable (explicit override, the managed venv, or PATH).
    Binary(PathBuf),
    /// A Python interpreter with the `endstone` module importable.
    PythonModule(PathBuf),
}

/// Resolve the Endstone launch command without starting anything. Order:
/// explicit `ENDSTONE_BIN`, the panel-managed uv toolchain venv, `endstone`
/// on PATH, then a legacy `ENDSTONE_PYTHON` whose interpreter can actually
/// import the endstone module (probed, so a bare `python3` never crashes the
/// server into a restart loop).
pub fn resolve_launch(config: &Config) -> Option<EndstoneLaunch> {
    if let Some(bin) = &config.endstone_bin {
        return Some(EndstoneLaunch::Binary(bin.clone()));
    }
    let managed = endstone_binary(config);
    if managed.exists() {
        return Some(EndstoneLaunch::Binary(managed));
    }
    if let Some(found) = find_on_path("endstone") {
        return Some(EndstoneLaunch::Binary(found));
    }
    let python = config.endstone_python.as_ref()?;
    python_can_import_endstone(python).then(|| EndstoneLaunch::PythonModule(python.clone()))
}

/// Look `name` up in PATH, returning the first executable regular file.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe_names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{name}.{ext}"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path) {
        for exe in &exe_names {
            let candidate = dir.join(exe);
            let Ok(meta) = std::fs::metadata(&candidate) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Some(candidate);
        }
    }
    None
}

/// Probe whether `<python> -c "import endstone"` succeeds.
fn python_can_import_endstone(python: &Path) -> bool {
    std::process::Command::new(python)
        .arg("-c")
        .arg("import endstone")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
