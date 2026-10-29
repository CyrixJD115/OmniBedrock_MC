//! Endstone toolchain management through a panel-managed uv.
//!
//! The panel bundles no Python. Instead it provisions, all under the panel
//! data dir and without admin rights or a system Python:
//!
//! 1. a pinned standalone `uv` binary (SHA256-verified download from the
//!    official astral-sh/uv release CDN),
//! 2. a uv-managed CPython (`uv python install`),
//! 3. an Endstone virtualenv (`uv venv` + `uv pip install endstone`).
//!
//! The server supervisor launches the venv's `endstone` executable, so
//! switching to the Endstone backend works on a machine with no Python at all.

mod install;
mod launch;
mod manager;
mod paths;
mod pypi;
mod toolchain;

pub(crate) use install::read_marker_version;
pub use launch::{EndstoneLaunch, resolve_launch};
pub use manager::Endstone;
pub use paths::{endstone_binary, python_install_dir, uv_binary, uv_cache_dir, uv_dir, venv_dir};
pub(crate) use pypi::version_cmp;
pub use pypi::{EndstoneRegistry, version_exists};
