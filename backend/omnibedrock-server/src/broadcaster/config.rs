//! Broadcaster config.yml analysis and validation: resolve the v7 relay
//! target (`session.remote-address` is THE key), detect the legacy
//! `session-info` address the way the jar migrates it, flag publicly
//! routable relay targets (the hairpin-NAT join trap), and announce the
//! verdicts on the console at every broadcaster start.

use crate::server::Console;
use omnibedrock_core::config::Config;
use std::net::IpAddr;

use super::helpers::instance_port;
use super::types::{RelayAnalysis, RelayTarget};

/// True for globally routable addresses — the ones a co-located broadcaster
/// must never use as its relay target because reaching them loops through the
/// router (hairpin NAT), which most consumer routers refuse.
fn publicly_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                // 100.64/10 carrier-grade NAT (Tailscale and friends): locally
                // bound on this host's VPN interface, so relaying to it works.
                || v4.octets()[0] == 100)
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified())
        }
    }
}

/// Resolve one config address/port pair. `auto` resolves exactly like the
/// jar's standalone bootstrap: loopback, with this instance's real port
/// instead of the jar's hardcoded 19132 default. Hostnames pass through
/// unvalidated (no IP to judge routability with).
fn resolve_target(
    addr: Option<&serde_yaml::Value>,
    port: Option<&serde_yaml::Value>,
    fallback_port: u16,
) -> RelayTarget {
    fn as_text(value: Option<&serde_yaml::Value>) -> Option<String> {
        match value {
            Some(serde_yaml::Value::String(s)) => Some(s.trim().to_string()),
            Some(serde_yaml::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }
    let addr_text = as_text(addr).filter(|s| !s.is_empty());
    let port_text = as_text(port).filter(|s| !s.is_empty());
    let ip = match addr_text.as_deref() {
        None | Some("auto") => IpAddr::from([127, 0, 0, 1]),
        Some(text) => text
            .parse::<IpAddr>()
            .unwrap_or(IpAddr::from([127, 0, 0, 1])),
    };
    let port = match port_text.as_deref() {
        None | Some("auto") => fallback_port,
        Some(text) => text.parse::<u16>().unwrap_or(fallback_port),
    };
    RelayTarget {
        text: format!("{ip}:{port}"),
        public: publicly_routable(ip),
    }
}

/// Parse config.yml for everything the panel validates: the operational
/// relay target, the legacy session-info address, and whether query pings
/// are enabled at all.
pub(super) fn analyze_relay_config(text: &str, fallback_port: u16) -> RelayAnalysis {
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return RelayAnalysis::unknown();
    };
    let session = doc.get("session");
    let query_server = session
        .and_then(|s| s.get("query-server"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let relay = match session {
        Some(s) if s.get("remote-address").is_some() || s.get("remote-port").is_some() => Some(
            resolve_target(s.get("remote-address"), s.get("remote-port"), fallback_port),
        ),
        _ => None,
    };
    let legacy = session
        .and_then(|s| s.get("session-info"))
        .filter(|info| info.get("ip").is_some() || info.get("port").is_some())
        .map(|info| resolve_target(info.get("ip"), info.get("port"), fallback_port));
    RelayAnalysis {
        query_server,
        relay,
        legacy,
    }
}

/// Announce the resolved relay targets at every broadcaster start and warn
/// loudly about the public-IP trap that costs hours to diagnose by log alone.
pub(super) async fn announce_relay(config: &Config, console: &Console) {
    let text = match tokio::fs::read_to_string(config.broadcaster_dir.join("config.yml")).await {
        Ok(text) => text,
        Err(e) => {
            console
                .publish(
                    format!("[LOCAL] Could not read config.yml for relay validation: {e}"),
                    "panel",
                    false,
                )
                .await;
            return;
        }
    };
    let analysis = analyze_relay_config(&text, instance_port(config));
    match &analysis.relay {
        Some(target) => {
            console
                .publish(
                    format!(
                        "[LOCAL] Relay target (session.remote-address): {}",
                        target.text
                    ),
                    "panel",
                    false,
                )
                .await;
            if target.public {
                console
                    .publish(
                        format!(
                            "[LOCAL] WARNING: the relay target {} is a PUBLIC IP, but this \
                             broadcaster runs on the same host as the server. Routers refuse the \
                             hairpin loop, so friends-list joins will fail (\"Failed to relay \
                             offer ... connection refused\", client-side InitialConnection-81). \
                             Set session.remote-address to 127.0.0.1.",
                            target.text
                        ),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
        None => {
            console
                .publish(
                    "[LOCAL] config.yml has no session.remote-address; the jar default (auto = \
                     127.0.0.1) applies."
                        .to_string(),
                    "panel",
                    false,
                )
                .await;
        }
    }
    if let Some(legacy) = &analysis.legacy {
        console
            .publish(
                format!(
                    "[LOCAL] Legacy session-info address: {} {}",
                    legacy.text,
                    if legacy.public {
                        "(PUBLIC — this pre-v7 key becomes the relay target on jar migration; \
                         switch it to 127.0.0.1)"
                    } else {
                        "(migrated into session.remote-address by the jar on load)"
                    }
                ),
                "panel",
                false,
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::consts::DEFAULT_CONFIG;
    use super::*;

    #[test]
    fn analyzes_current_v7_config() {
        let text = r"
session:
  remote-address: 127.0.0.1
  remote-port: 24474
  query-server: true
  session-info:
    host-name: Omni
";
        let a = analyze_relay_config(text, 19132);
        assert_eq!(
            a.relay,
            Some(RelayTarget {
                text: "127.0.0.1:24474".into(),
                public: false
            })
        );
        assert!(a.legacy.is_none());
        assert!(a.query_server);
    }

    #[test]
    fn flags_public_relay_target() {
        let text = r"
session:
  remote-address: 203.0.113.7
  remote-port: 24474
";
        let a = analyze_relay_config(text, 19132);
        let relay = a.relay.expect("relay parsed");
        assert!(relay.public, "a public IP must be flagged");
        assert_eq!(relay.text, "203.0.113.7:24474");
    }

    #[test]
    fn private_addresses_are_not_flagged() {
        for ip in [
            "127.0.0.1",
            "192.168.1.115",
            "10.0.0.5",
            "100.74.133.65",
            "::1",
        ] {
            let text = format!("session:\n  remote-address: {ip}\n  remote-port: 19132\n");
            let a = analyze_relay_config(&text, 19132);
            assert!(!a.relay.unwrap().public, "{ip} must not be flagged public");
        }
    }

    #[test]
    fn legacy_session_info_is_detected() {
        // Pre-v7 layout (config-version 5): the address lived in session-info
        // and the jar migrates it into the relay keys on load.
        let text = r"
session:
  query-server: true
  session-info:
    ip: 203.0.113.7
    port: 19132
";
        let a = analyze_relay_config(text, 19132);
        assert!(a.relay.is_none());
        let legacy = a.legacy.expect("legacy parsed");
        assert!(legacy.public);
        assert_eq!(legacy.text, "203.0.113.7:19132");
    }

    #[test]
    fn auto_resolves_like_the_jar_but_with_the_real_port() {
        let text = r"
session:
  remote-address: auto
  remote-port: auto
";
        let a = analyze_relay_config(text, 24474);
        assert_eq!(a.relay.unwrap().text, "127.0.0.1:24474");
    }

    #[test]
    fn query_server_off_is_reported() {
        let text = "session:\n  query-server: false\n";
        assert!(!analyze_relay_config(text, 19132).query_server);
        assert!(analyze_relay_config("session: {}", 19132).query_server);
    }

    #[test]
    fn broken_config_degrades_to_unknown() {
        let a = analyze_relay_config("\t:not yaml: [", 19132);
        assert_eq!(a, RelayAnalysis::unknown());
        assert!(a.relay.is_none());
    }

    #[test]
    fn fresh_template_carries_loopback_relay_target() {
        let text = DEFAULT_CONFIG.replace("{{PORT}}", "24474");
        let a = analyze_relay_config(&text, 24474);
        assert_eq!(
            a.relay,
            Some(RelayTarget {
                text: "127.0.0.1:24474".into(),
                public: false
            })
        );
        assert!(
            !text.contains("{{PORT}}"),
            "placeholder must be substituted"
        );
        // Post-v7 layouts have no session-info address to trip over (Bug 1).
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert!(doc["session"]["session-info"].get("ip").is_none());
        assert_eq!(doc["config-version"].as_i64(), Some(7));
    }
}
