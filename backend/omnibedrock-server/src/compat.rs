//! Add-on compatibility checker.
//!
//! Reads what the server actually reported (log files, or the live console
//! when there are none), classifies every warning/error into a stable issue
//! with pack attribution, and combines it with static checks: manifest
//! engine/module versions against what this server build supports (learned
//! from the npm registry), beta pinning, missing dependencies, and a scan of
//! pack JavaScript for 1.x-only APIs shipped under a 2.x manifest. Every
//! issue carries an explanation, concrete fix steps and documentation links.
//! The checker only reads; it never modifies packs.

mod export;
mod housekeeping;
mod knowledge;
mod logs;
mod packs;
mod parse;
mod report;
mod rules;
mod scan;
mod script_api;
mod statics;
#[cfg(test)]
mod tests;
mod types;

pub use export::markdown;
pub use housekeeping::prune_content_logs;
pub use logs::forget as forget_log_cache;
pub use report::{Inputs, Options, analyze};
pub use types::*;
