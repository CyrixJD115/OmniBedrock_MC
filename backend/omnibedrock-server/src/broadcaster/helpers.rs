//! Shared helpers for the broadcaster: jar resolution shared by the
//! `Broadcaster` handle and the supervisor, and the instance RakNet port
//! read out of server.properties.

use omnibedrock_core::config::Config;
use std::path::PathBuf;

use super::consts::JAR_NAME;

/// Jar resolution shared by `Broadcaster::jar_path` and the supervisor:
/// explicit override, then the instance's own dir, then the panel-level dir.
pub(super) fn resolve_jar(config: &Config, jar_home: Option<&std::path::Path>) -> PathBuf {
    if let Some(jar) = &config.broadcaster_jar {
        return jar.clone();
    }
    let local = config.broadcaster_dir.join(JAR_NAME);
    if local.exists() {
        return local;
    }
    if let Some(home) = jar_home {
        let shared = home.join(JAR_NAME);
        if shared.exists() {
            return shared;
        }
    }
    local
}

/// The instance's RakNet port from server.properties; 19132 when absent.
pub(super) fn instance_port(config: &Config) -> u16 {
    let Ok(text) = std::fs::read_to_string(config.bedrock_server_dir.join("server.properties"))
    else {
        return 19132;
    };
    text.lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("server-port=")?;
            value.trim().parse::<u16>().ok()
        })
        .unwrap_or(19132)
}
