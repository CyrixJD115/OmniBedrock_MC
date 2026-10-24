//! Game-server domain layer. The supervisor exclusively owns the Endstone or
//! vanilla BDS child process, its console pipeline, derived metrics, backup
//! jobs, addon archives, BDS version management, and the MCXboxBroadcast
//! integration.
pub mod addons;
pub mod addon_cache;
pub mod backups;
pub mod bds;
pub mod broadcaster;
pub mod curseforge;
pub mod endstone;
mod journal;
pub mod metrics;
pub mod server;
