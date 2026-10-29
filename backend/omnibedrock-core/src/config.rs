//! Panel configuration: the CLI flag set, the runtime `Config` model with
//! its defaults, the on-disk config file format, and the layered loader that
//! merges defaults → config file → environment → CLI into one `Config`.
//!
//! The public names stay reachable at `omnibedrock_core::config::*` through
//! the re-exports below; the implementation lives in the `cli`, `model`,
//! `file`, and `load` submodules.

mod cli;
mod file;
mod load;
mod model;

pub use cli::Cli;
pub use model::Config;
