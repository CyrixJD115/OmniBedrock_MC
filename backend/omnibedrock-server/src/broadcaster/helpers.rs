//! Shared helpers for the broadcaster: jar resolution shared by the
//! `Broadcaster` handle and the supervisor, and the server.properties facts
//! (port, transport, names) the relay checks and health probe need.

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

/// The server.properties facts the broadcaster cares about.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ServerFacts {
    /// `server-port`, 19132 when absent.
    pub(super) port: u16,
    /// `transport` lowercased; BDS 1.26.60+ defaults to nethernet.
    pub(super) transport: String,
    pub(super) server_name: Option<String>,
    pub(super) level_name: Option<String>,
    pub(super) max_players: Option<u32>,
    pub(super) gamemode: Option<String>,
}

pub(super) fn parse_server_facts(text: &str) -> ServerFacts {
    let mut facts = ServerFacts {
        port: 19132,
        transport: "nethernet".into(),
        server_name: None,
        level_name: None,
        max_players: None,
        gamemode: None,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "server-port" => {
                if let Ok(p) = value.parse() {
                    facts.port = p;
                }
            }
            "transport" if !value.is_empty() => facts.transport = value.to_ascii_lowercase(),
            "server-name" if !value.is_empty() => facts.server_name = Some(value.to_owned()),
            "level-name" if !value.is_empty() => facts.level_name = Some(value.to_owned()),
            "max-players" => facts.max_players = value.parse().ok(),
            "gamemode" if !value.is_empty() => facts.gamemode = Some(value.to_owned()),
            _ => {}
        }
    }
    facts
}

/// Read server.properties facts; defaults when the file is missing.
pub(super) fn server_facts(config: &Config) -> ServerFacts {
    let text = std::fs::read_to_string(config.bedrock_server_dir.join("server.properties"))
        .unwrap_or_default();
    parse_server_facts(&text)
}

/// The instance's game port from server.properties; 19132 when absent.
pub(super) fn instance_port(config: &Config) -> u16 {
    server_facts(config).port
}
