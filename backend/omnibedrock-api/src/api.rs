//! API v1 compatibility layer. Services own process, auth, backup and filesystem logic.

mod addons;
mod audit;
mod backups;
mod broadcaster;
mod dispatch;
mod dispatch_authenticated;
mod files;
mod helpers;
mod instances;
mod players;
mod properties;
mod router;
mod server;
mod state;
mod store;
mod upload;

pub use router::router;
pub use state::AppState;
