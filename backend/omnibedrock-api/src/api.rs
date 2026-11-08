//! API v1 compatibility layer. Services own process, auth, backup and filesystem logic.

mod addons;
mod audit;
mod backups;
mod broadcaster;
mod command;
mod dispatch;
mod dispatch_authenticated;
mod docs;
mod explorer;
mod files;
mod helpers;
mod instances;
mod middleware;
mod players;
mod plugins;
mod properties;
mod router;
mod routes_list;
mod server;
mod state;
mod store;
mod upload;

pub use router::router;
pub use state::AppState;
