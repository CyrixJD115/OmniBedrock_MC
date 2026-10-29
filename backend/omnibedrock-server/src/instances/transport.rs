//! `server-udp-ports` validation and launch-time transport guidance: parsing
//! of udp-port entries, backend-aware policy checks (Endstone NetherNet
//! single-port multiplexer vs vanilla per-player ports), and the console
//! note telling the operator what to forward.

use crate::server::{Backend, Console};
use omnibedrock_core::config::Config;

/// First Endstone release whose stable NetherNet multiplexes every player
/// onto one forwarded port (TCP+UDP on server-port) instead of needing a
/// per-player UDP port from `server-udp-ports`.
pub const NETHERNET_MULTIPLEXER_VERSION: &str = "0.11.15";

/// True when an Endstone toolchain ships the NetherNet single-port
/// multiplexer (>= [`NETHERNET_MULTIPLEXER_VERSION`]). An unknown or empty
/// version never claims it. [`crate::endstone::version_cmp`] sorts newest
/// first, so "a is not older than the gate" reads as "not Greater".
pub fn endstone_has_multiplexer(version: &str) -> bool {
    crate::endstone::version_cmp(version, NETHERNET_MULTIPLEXER_VERSION)
        != std::cmp::Ordering::Greater
}

/// One `server-udp-ports` comma-separated entry.
#[derive(Debug, PartialEq)]
enum UdpPortEntry {
    Port(u16),
    Range(u16, u16),
    /// `[ip:]external:internal`; the ip is optional and may be IPv4, IPv6,
    /// or a hostname (everything before the last two colon groups).
    Mapping(Option<String>, u16, u16),
}

/// Parse one `server-udp-ports` entry. Syntax errors carry the reason;
/// backend/transport policy lives in [`validate_udp_ports_value`].
fn parse_udp_port_entry(entry: &str) -> Result<UdpPortEntry, String> {
    let port = |text: &str| -> Result<u16, String> {
        text.parse::<u16>()
            .map_err(|_| format!("'{text}' is not a UDP port (1-65535)"))
    };
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("empty entry".into());
    }
    if let Some((lo, hi)) = entry.split_once('-') {
        let (lo, hi) = (port(lo.trim())?, port(hi.trim())?);
        if lo > hi {
            return Err(format!("range {lo}-{hi} starts above its end"));
        }
        return Ok(UdpPortEntry::Range(lo, hi));
    }
    // `[ip:]external:internal`: split from the right so an IPv6 address
    // survives as the ip part. A range in either port slot ("ip:a-b:a-b",
    // which silently bound nothing on vanilla) fails the integer parse.
    let groups: Vec<&str> = entry.rsplit(':').collect();
    if groups.len() >= 3 {
        let internal = port(groups[0])?;
        let external = port(groups[1])?;
        let ip = groups[2..]
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join(":");
        return Ok(UdpPortEntry::Mapping(
            (!ip.is_empty()).then_some(ip),
            external,
            internal,
        ));
    }
    if groups.len() == 2 {
        let internal = port(groups[0])?;
        let external = port(groups[1])?;
        return Ok(UdpPortEntry::Mapping(None, external, internal));
    }
    Ok(UdpPortEntry::Port(port(entry)?))
}

/// Validate a `server-udp-ports` write against what the running backend
/// would actually do with it. Vanilla NetherNet binds the listed ports and
/// ranges directly (one UDP port per player slot); Endstone since 0.11.15
/// multiplexes everything onto server-port, so only the
/// `[ip:]external:internal` mapping with `internal == server-port` has any
/// effect — ranges and mismatched mappings are rejected loudly instead of
/// silently binding nothing. On `transport=raknet` the key is inert and only
/// the syntax is checked.
pub fn validate_udp_ports_value(
    backend: Backend,
    transport: &str,
    server_port: u16,
    value: &str,
) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(
            "server-udp-ports cannot be blank; delete the key instead of emptying it".into(),
        );
    }
    let nethernet = transport.eq_ignore_ascii_case("nethernet");
    for entry in value.split(',') {
        let parsed = parse_udp_port_entry(entry)
            .map_err(|e| format!("server-udp-ports entry '{}': {e}", entry.trim()))?;
        if !nethernet {
            continue;
        }
        match (backend, &parsed) {
            (Backend::Endstone, UdpPortEntry::Range(lo, hi)) => {
                return Err(format!(
                    "server-udp-ports range {lo}-{hi} does nothing on Endstone: its NetherNet \
                     multiplexer needs ONE port (TCP+UDP on server-port), not a range"
                ));
            }
            (Backend::Endstone, UdpPortEntry::Mapping(_, _, internal))
                if *internal != server_port =>
            {
                return Err(format!(
                    "server-udp-ports mapping internal port {internal} must equal server-port \
                     {server_port} — that is the one port Endstone multiplexes NetherNet onto"
                ));
            }
            (Backend::Vanilla, UdpPortEntry::Mapping(_, external, internal)) => {
                return Err(format!(
                    "server-udp-ports mapping {external}:{internal} is Endstone-only syntax: \
                     vanilla BDS binds the listed ports directly, and the ip:a-b:a-b range \
                     form silently binds nothing"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Launch-time transport guidance. The old blanket warning ("nethernet
/// ignores server-port") predates the Endstone single-port multiplexer and
/// is now wrong exactly when it mattered most: since Endstone 0.11.15
/// server-port IS the port to forward (TCP and UDP). Each backend/transport
/// combination gets the one line that is true for it.
pub async fn advise_transport(
    config: &Config,
    console: &Console,
    backend: Backend,
    endstone_version: Option<&str>,
) {
    let Ok(text) =
        tokio::fs::read_to_string(config.bedrock_server_dir.join("server.properties")).await
    else {
        return;
    };
    let mut transport = String::from("nethernet");
    let mut port: u16 = 19132;
    let mut max_players: u16 = 10;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("transport=") {
            let v = v.trim();
            if !v.is_empty() {
                transport = v.to_string();
            }
        } else if let Some(v) = line.strip_prefix("server-port=")
            && let Ok(p) = v.trim().parse()
        {
            port = p;
        } else if let Some(v) = line.strip_prefix("max-players=")
            && let Ok(m) = v.trim().parse()
        {
            max_players = m;
        }
    }
    let message = if transport.eq_ignore_ascii_case("raknet") {
        format!(
            "transport=raknet: fully updated Bedrock clients (1.26.60+) removed RakNet and \
             CANNOT join — forward UDP {port} only, and switch transport=nethernet in the \
             Properties editor unless staying on RakNet for older clients is deliberate"
        )
    } else if transport.eq_ignore_ascii_case("nethernet") {
        match backend {
            Backend::Endstone if endstone_version.is_some_and(endstone_has_multiplexer) => format!(
                "NetherNet single-port mode (Endstone {}): forward ONE port on the router — \
                 TCP AND UDP on {port} (server-port). No per-player ports needed.",
                endstone_version.unwrap_or(NETHERNET_MULTIPLEXER_VERSION),
            ),
            Backend::Endstone => format!(
                "transport=nethernet on Endstone {}: this toolchain has no NetherNet \
                 multiplexer, so game traffic needs one UDP port per player slot from \
                 server-udp-ports and ignores server-port — upgrade the Endstone toolchain \
                 to {NETHERNET_MULTIPLEXER_VERSION}+ for single-port mode",
                endstone_version.unwrap_or("unknown"),
            ),
            Backend::Vanilla => format!(
                "transport=nethernet on vanilla BDS: TCP {port} is signaling ONLY and game \
                 traffic needs one UDP port per player slot from server-udp-ports \
                 ({max_players} players = {max_players} ports) — home routers usually cannot \
                 forward port ranges. The Endstone backend multiplexes NetherNet onto one \
                 TCP+UDP port instead"
            ),
        }
    } else {
        format!(
            "transport={transport} is not a known value (raknet or nethernet); the server \
             will fall back to its default and clients may not reach it"
        )
    };
    console
        .publish(format!("NOTE: {message}"), "panel", false)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transport_guidance_matches_backend_and_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            bedrock_server_dir: dir.path().join("server"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.bedrock_server_dir).unwrap();
        let console = Console::with_journal_root(dir.path().join("journal"), 128, 65536, 1 << 20);
        // No properties file yet: no noise.
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        assert!(console.tail(10).is_empty());
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=raknet\nserver-port=19140\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        // Each publish appends; tail is cumulative, so check the newest line.
        let tail = console.tail(10);
        assert_eq!(tail.len(), 1, "{:?}", tail);
        assert!(tail[0].line.contains("raknet"), "{}", tail[0].line);
        assert!(tail[0].line.contains("CANNOT join"), "{}", tail[0].line);
        // Endstone 0.11.15 + nethernet: the single-port recipe, not a warning.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=nethernet\nserver-port=24474\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 2, "{:?}", tail);
        assert!(
            tail[1].line.contains("single-port") && tail[1].line.contains("24474"),
            "{}",
            tail[1].line
        );
        assert!(
            !tail[1].line.contains("ignores server-port"),
            "{}",
            tail[1].line
        );
        // Endstone 0.11.14 + nethernet: the multiplexer is not there yet.
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.14")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 3, "{:?}", tail);
        assert!(
            tail[2].line.contains("no NetherNet multiplexer"),
            "{}",
            tail[2].line
        );
        // Vanilla + nethernet: per-player ports warning with the max count.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=nethernet\nserver-port=19140\nmax-players=12\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Vanilla, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 4, "{:?}", tail);
        assert!(
            tail[3].line.contains("signaling ONLY") && tail[3].line.contains("12"),
            "{}",
            tail[3].line
        );
        // An unknown transport value is called out, never silently accepted.
        std::fs::write(
            config.bedrock_server_dir.join("server.properties"),
            "transport=wat\n",
        )
        .unwrap();
        advise_transport(&config, &console, Backend::Endstone, Some("0.11.15")).await;
        let tail = console.tail(10);
        assert_eq!(tail.len(), 5, "{:?}", tail);
        assert!(tail[4].line.contains("transport=wat"), "{}", tail[4].line);
    }

    #[test]
    fn udp_ports_validation_follows_backend_matrix() {
        use validate_udp_ports_value as v;
        // Syntax errors are backend-independent.
        assert!(v(Backend::Endstone, "nethernet", 24474, "").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "  ").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474,").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474-24470").is_err());
        // The 3-part RANGE form binds nothing on any backend: hard reject.
        assert!(
            v(
                Backend::Vanilla,
                "nethernet",
                19132,
                "1.2.3.4:19132-19134:19132-19134"
            )
            .is_err()
        );
        // Endstone nethernet: single mapping (with optional ip), no ranges.
        // internal must equal server-port; the advertised external may differ.
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "1.2.3.4:20000:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "[::1]:24474:24474").is_ok());
        assert!(v(Backend::Endstone, "nethernet", 24474, "20000-20100").is_err());
        assert!(v(Backend::Endstone, "nethernet", 24474, "1.2.3.4:24474:20000").is_err());
        // Bare ports are a harmless 0.11.11-era artifact.
        assert!(v(Backend::Endstone, "nethernet", 24474, "24474").is_ok());
        // Vanilla nethernet: ports and ranges bind directly; the mapping
        // form is Endstone-only syntax.
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132-19142").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132,19134-19136").is_ok());
        assert!(v(Backend::Vanilla, "nethernet", 19132, "19132:19132").is_err());
        // On raknet the key is inert — only the syntax is checked.
        assert!(v(Backend::Endstone, "raknet", 19132, "20000-20100").is_ok());
        assert!(v(Backend::Vanilla, "raknet", 19132, "1.2.3.4:1:2").is_ok());
    }

    #[test]
    fn endstone_multiplexer_version_gate() {
        assert!(endstone_has_multiplexer("0.11.15"));
        assert!(endstone_has_multiplexer("0.11.16"));
        assert!(endstone_has_multiplexer("0.12.0"));
        assert!(!endstone_has_multiplexer("0.11.14"));
        assert!(!endstone_has_multiplexer("0.9.3"));
        assert!(!endstone_has_multiplexer(""));
        assert!(!endstone_has_multiplexer("unknown"));
    }
}
