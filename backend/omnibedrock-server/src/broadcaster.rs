//! MCXboxBroadcast Standalone integration.
//!
//! Every server instance owns its own broadcaster: a Java process running in
//! that instance's broadcaster directory holding `config.yml`,
//! `screenshot.jpg`, and the `cache/` token store, so accounts, settings, and
//! sign-in state never leak between instances. Output streams through a
//! per-instance Console ring like the game server; device-code prompts
//! emitted during Microsoft authentication are parsed out of the log stream so
//! the UI can surface the code and verification URL.
//!
//! The jar itself is shared: instance dirs stay small because `jar_path`
//! falls back to the panel-level broadcaster directory when the instance has
//! no jar of its own.
//!
//! Lifecycle safety nets:
//! - Exactly one broadcaster per instance. Before spawning, the panel scans
//!   for a foreign `java -jar …/{JAR_NAME}` process working in this
//!   instance's broadcaster directory (started by hand or orphaned by a
//!   previous panel run) and refuses to start a second one; the UI can stop
//!   or take over the foreign process instead.
//! - Relay-target validation at every start. `session.remote-address` is the
//!   offer-relay target and MUST be loopback on a co-located setup; a public
//!   IP there is announced loudly because hairpin NAT breaks every join.
//!
//! Module layout (pure code motion from the former single file):
//! - [`types`]: shared data types and supervisor control plumbing
//! - [`consts`]: shared constants and the seeded config template
//! - [`helpers`]: jar resolution and instance-port lookup
//! - [`config`]: config.yml relay-target parsing, validation, announcements
//! - [`health`]: RakNet ping health checking and console-line mirroring
//! - [`external`]: /proc detection of foreign broadcasters and takeover
//! - [`process`]: process spawn/teardown plumbing and java detection
//! - [`lifecycle`]: the supervisor event loop and pidfile bookkeeping
//! - [`handle`]: the per-instance `Broadcaster` handle and device-code watcher

mod config;
mod consts;
mod external;
mod handle;
mod health;
mod helpers;
mod lifecycle;
mod process;
mod types;

pub use consts::{DEFAULT_CONFIG, JAR_NAME};
pub use external::find_external;
pub use handle::Broadcaster;
pub use process::detect_java;
pub use types::{
    BcSnapshot, BcStatus, DeviceCode, DockerSpec, ExternalBroadcaster, PingHealth, RelayAnalysis,
    RelayTarget,
};
