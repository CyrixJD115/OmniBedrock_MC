//! Wall-clock and console-line classification helpers shared across the
//! server supervisor: monotonic-ish microsecond timestamps and log-level
//! detection from raw server output.

use regex::Regex;
use std::sync::LazyLock;

static LEVEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[(ERROR|WARN(?:ING)?|INFO|DEBUG|LOCAL)\]").unwrap());
static ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)error|exception|traceback|fatal").unwrap());

pub fn now() -> f64 {
    chrono::Utc::now().timestamp_micros() as f64 / 1_000_000.0
}
pub fn detect_level(line: &str) -> &'static str {
    if let Some(c) = LEVEL.captures(line) {
        match c[1].to_ascii_lowercase().as_str() {
            "error" => "error",
            "warn" | "warning" => "warn",
            "debug" => "debug",
            "local" => "local",
            _ => "info",
        }
    } else if ERROR.is_match(line) {
        "error"
    } else {
        "info"
    }
}
