//! The supervisor exclusively owns the child. Readers only mutate bounded console state.
//! No process wait or pipe operation blocks a Tokio worker thread.
//!
//! Split by responsibility:
//! - `types`: shared supervisor data types (status, snapshot, backend,
//!   settings, console events, paths, launch context)
//! - `helpers`: timestamps and console log-level detection
//! - `console`: the bounded console ring with stats and disk journal
//! - `pipes`: child output line splitting and port-in-use advisories
//! - `commands`: control messages and stdin/FIFO command sinks
//! - `runtime`: the running-instance handle, drain, exit watching, stop ladder
//! - `spawn`: spawning owned child processes and wiring stdio
//! - `unit`: systemd user-unit FIFO/journal reader/monitor/attach
//! - `docker`: docker run argv, container monitor, container attach
//! - `launch`: per-deployment launch paths and backend argv
//! - `manager`: the public `ServerManager` handle
//! - `supervisor`: the event loop owning the running instance

mod commands;
mod console;
mod docker;
mod helpers;
mod launch;
mod manager;
mod pipes;
mod runtime;
mod spawn;
mod supervisor;
mod types;
mod unit;

pub use console::{Console, Replay};
pub use helpers::{detect_level, now};
pub use manager::ServerManager;
pub use types::{
    Backend, ConsoleEvent, ErrorStat, LaunchContext, ServerPaths, ServerSettings, Snapshot, Status,
};

pub(crate) use pipes::read_pipe;
