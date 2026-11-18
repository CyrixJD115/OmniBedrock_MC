//! Wall-clock and console-line classification helpers shared across the
//! server supervisor: monotonic-ish microsecond timestamps and log-level
//! detection from raw server output.

use regex::Regex;
use std::sync::LazyLock;

/// The level tag of a log line. Real-world shapes, all of which must match:
/// `[ERROR]`, `[01:35:27 WARNING]` (BDS/Endstone), `[2026-11-18 01:35:27:123
/// INFO]` (vanilla BDS), `[01:41:41.540 main/INFO]` and `[… Thread-1/ERROR]`
/// (MCXboxBroadcast/log4j). The old pattern only accepted the bare form, so
/// every timestamped line fell through to a keyword search: `[01:35 WARNING]`
/// came out as info and `[01:35 INFO]: … errorMessage=(null)` as an error.
static LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\[(?:[^\[\]\n]{0,48}?[\s/])?(ERROR|SEVERE|CRITICAL|FATAL|WARN(?:ING)?|INFO|DEBUG|TRACE|LOCAL)\]",
    )
    .unwrap()
});
/// Untagged lines (Python tracebacks, launcher output): whole words only, so
/// `errorMessage=(null)` or "0 errors" are not errors.
static ERROR: LazyLock<Regex> = LazyLock::new(|| {
    // Plus exception class names (`RuntimeError`, `NullPointerException`).
    Regex::new(
        r"(?i:\b(?:error|exception|traceback|fatal)\b)|\b[A-Z][A-Za-z]*(?:Error|Exception)\b",
    )
    .unwrap()
});
/// BDS follows every content-log entry with an empty line carrying the same
/// level prefix (`[01:35:30 ERROR]: `). They carry no text but doubled the
/// error count and split every collapsed run.
static BLANK_TAGGED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*\[[^\[\]\n]{0,48}?(?:ERROR|WARN(?:ING)?|INFO|DEBUG)\]:?\s*$").unwrap()
});

/// True for a level-prefixed line with no message (see [`BLANK_TAGGED`]).
pub fn is_blank_tagged_line(line: &str) -> bool {
    BLANK_TAGGED.is_match(line)
}

pub fn now() -> f64 {
    chrono::Utc::now().timestamp_micros() as f64 / 1_000_000.0
}
pub fn detect_level(line: &str) -> &'static str {
    if let Some(c) = LEVEL.captures(line) {
        match c[1].to_ascii_lowercase().as_str() {
            "error" | "severe" | "critical" | "fatal" => "error",
            "warn" | "warning" => "warn",
            "debug" | "trace" => "debug",
            "local" => "local",
            _ => "info",
        }
    } else if ERROR.is_match(line) {
        "error"
    } else {
        "info"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_tags_win_over_keywords() {
        assert_eq!(
            detect_level("[01:35:28 WARNING]: [Scripting] alias in use"),
            "warn"
        );
        assert_eq!(
            detect_level(
                "[01:35:27 INFO]: Profiler config load result: success=1, errorMessage=(null)"
            ),
            "info"
        );
        assert_eq!(
            detect_level("[01:35:33 INFO]: Failed to import command: rtp | SyntaxError: x"),
            "info"
        );
        assert_eq!(detect_level("[01:35:30 ERROR]: [Json] bad"), "error");
        assert_eq!(
            detect_level("[2026-11-18 01:35:27:123 INFO] Starting Server"),
            "info"
        );
        assert_eq!(
            detect_level("[01:41:41.540 main/INFO] Starting MCXboxBroadcast"),
            "info"
        );
        assert_eq!(
            detect_level("[01:56:44.942 MCXboxBroadcast Thread-1/ERROR] [Primary Session] x"),
            "error"
        );
        assert_eq!(
            detect_level("[23:58:53.769 MCXboxBroadcast Thread-1/WARN] lost"),
            "warn"
        );
        assert_eq!(detect_level("[LOCAL] Start failed"), "local");
    }

    #[test]
    fn untagged_keyword_fallback_uses_whole_words() {
        assert_eq!(detect_level("Traceback (most recent call last):"), "error");
        assert_eq!(detect_level("RuntimeError: boom"), "error");
        assert_eq!(detect_level("ValueError exception raised"), "error");
        assert_eq!(detect_level("0 errors, 0 warnings"), "info");
        assert_eq!(detect_level("Quit correctly"), "info");
    }

    #[test]
    fn blank_content_log_trailers_are_recognized() {
        assert!(is_blank_tagged_line("[01:35:30 ERROR]: "));
        assert!(is_blank_tagged_line("[01:35:32 WARNING]:"));
        assert!(!is_blank_tagged_line("[01:35:30 ERROR]: [Json] bad"));
        assert!(!is_blank_tagged_line(""));
    }
}
