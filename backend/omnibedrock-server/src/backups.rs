//! Minecraft Bedrock world backups: zip creation and restore, trash
//! management, pre/post command hooks with console capture, and scheduled
//! automatic runs.

mod archive;
mod hooks;
mod live;
mod schedule;
mod service;
mod settings;
#[cfg(test)]
mod tests;
mod types;

pub use service::Backups;
pub use types::{BackupRequest, Hook, Hooks};
