//! Game-server domain layer. The supervisor exclusively owns the Endstone or
//! vanilla BDS child process (or its systemd user service), its console
//! pipeline, derived metrics, backup jobs, addon archives, BDS version
//! management, and the MCXboxBroadcast integration.
pub mod addon_cache;
pub mod addons;
pub mod backups;
pub mod bds;
pub mod broadcaster;
pub mod curseforge;
pub mod endstone;
pub mod instances;
mod journal;
pub mod metrics;
pub mod server;
pub mod systemd;
pub mod unit_helper;
