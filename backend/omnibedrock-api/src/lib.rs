//! HTTP surface of the panel: REST routes, authenticated WebSocket streams,
//! the JWT user/role store, the embedded UI, and the desktop launch mode.
pub mod api;
pub mod auth;
pub mod bundle;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod server_proc;
pub mod websocket;
