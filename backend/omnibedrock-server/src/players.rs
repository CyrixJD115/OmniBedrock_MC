//! Player management: a persistent join/leave history fed by the console,
//! plus the server's allowlist, permission and ban files.

mod files;
#[cfg(test)]
mod tests;
mod tracker;

pub use files::{
    AllowEntry, BanEntry, PERMISSION_LEVELS, PermissionEntry, allowlist, bans, permissions,
    set_allowlisted, set_permission,
};
pub use tracker::{PlayerRecord, Players};
