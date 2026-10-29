//! Shared constants for the broadcaster integration: jar name, timing
//! budgets, the RakNet ping magic, and the seeded config.yml template.

use std::time::Duration;

pub const JAR_NAME: &str = "MCXboxBroadcastStandalone.jar";
pub(super) const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a takeover waits for a foreign broadcaster to exit after SIGTERM.
pub(super) const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(10);
/// RakNet ping budget, matching the jar's own query timeout.
pub(super) const PING_TIMEOUT: Duration = Duration::from_millis(1500);
/// How often the panel re-probes the relay target while the broadcaster runs.
pub(super) const PING_INTERVAL: Duration = Duration::from_secs(30);

/// RakNet offline magic shared by unconnected ping/pong packets.
pub(super) const RAKNET_MAGIC: [u8; 16] = [
    0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78,
];

pub const DEFAULT_CONFIG: &str = include_str!("../broadcaster_default_config.yml");
