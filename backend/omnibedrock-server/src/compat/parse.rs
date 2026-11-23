//! Log line parsing: the three prefixes the checker reads (Endstone
//! `[HH:MM:SS] [Thread/LEVEL]: msg`, vanilla BDS `[YYYY-MM-DD HH:MM:SS:mmm
//! LEVEL] msg`, and the panel console `[HH:MM:SS LEVEL]: msg`), stack-frame
//! extraction, and message normalization for grouping.

use super::types::Location;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Unknown,
    Info,
    Warn,
    Error,
}

impl Level {
    fn from_word(word: &str) -> Option<Self> {
        Some(match word.to_ascii_uppercase().as_str() {
            "ERROR" | "SEVERE" | "FATAL" | "CRITICAL" => Self::Error,
            "WARN" | "WARNING" => Self::Warn,
            "INFO" | "INFORM" => Self::Info,
            "DEBUG" | "TRACE" | "VERBOSE" => Self::Debug,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub time: Option<&'a str>,
    pub level: Level,
    pub msg: &'a str,
}

fn split_bracket(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some((&rest[..end], &rest[end + 1..]))
}

fn trim_colon(s: &str) -> &str {
    let s = s.strip_prefix(':').unwrap_or(s);
    s.strip_prefix(' ').unwrap_or(s)
}

/// Parse one raw log line. Never fails: unknown shapes become `Unknown`
/// level with the whole line as the message.
pub fn parse_line(raw: &str) -> Line<'_> {
    let line = raw.trim_end_matches(['\r', '\n']);
    let line = line.strip_prefix("NO LOG FILE! - ").unwrap_or(line);
    let Some((head, rest)) = split_bracket(line) else {
        return Line {
            time: None,
            level: Level::Unknown,
            msg: line,
        };
    };
    // `[time LEVEL]` or `[date time:ms LEVEL]`
    if let Some((time, word)) = head.rsplit_once(' ')
        && let Some(level) = Level::from_word(word)
    {
        let time = time.rsplit(' ').next().map(|t| t.get(..8).unwrap_or(t));
        return Line {
            time,
            level,
            msg: trim_colon(rest),
        };
    }
    // `[time] [Thread/LEVEL]: msg`
    let after = rest.trim_start();
    if let Some((tag, msg)) = split_bracket(after)
        && let Some((_, word)) = tag.rsplit_once('/')
        && let Some(level) = Level::from_word(word)
    {
        return Line {
            time: Some(head),
            level,
            msg: trim_colon(msg),
        };
    }
    Line {
        time: None,
        level: Level::Unknown,
        msg: line,
    }
}

/// A continuation of the previous event: a JS stack frame or a Python
/// traceback line, emitted as its own log line.
pub fn is_continuation(msg: &str) -> bool {
    let t = msg.trim_start();
    let indented = msg.len() != t.len();
    (indented && t.starts_with("at "))
        || (indented && t.starts_with("File \""))
        || msg.trim().is_empty()
}

/// Parse `at fn (file:line)` / `at fn (native)` / `at <anonymous> (file)`.
pub fn parse_frame(frame: &str) -> Option<Location> {
    let t = frame.trim().strip_prefix("at ")?;
    let (function, loc) = match t.rfind(" (") {
        Some(i) if t.ends_with(')') => (Some(&t[..i]), &t[i + 2..t.len() - 1]),
        _ => (None, t),
    };
    if loc == "native" {
        return None;
    }
    let (file, line) = match loc.rsplit_once(':') {
        Some((f, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() => {
            (f, l.parse().ok())
        }
        _ => (loc, None),
    };
    Some(Location {
        function: function
            .filter(|f| *f != "<anonymous>" && !f.is_empty())
            .map(str::to_owned),
        file: Some(file.to_owned()).filter(|f| !f.is_empty()),
        line,
    })
}

/// Split an inline `msg    at fn (file:line)    at …` into the message and
/// its first useful frame.
pub fn split_inline_frames(msg: &str) -> (&str, Option<Location>) {
    let Some(i) = msg.find("    at ") else {
        return (msg.trim_end(), None);
    };
    let head = msg[..i].trim_end();
    let frame = msg[i..]
        .split("    at ")
        .filter(|f| !f.trim().is_empty())
        .find_map(|f| parse_frame(&format!("at {}", f.trim())));
    (head, frame)
}

/// Strip Minecraft `§x` colour codes and ANSI escapes.
pub fn strip_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '§' => {
                chars.next();
            }
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Read a balanced `[...]` at the start of `s` (pack names may contain
/// brackets: `[[v5.1] WAILA [BP]] Error…`). Returns (inner, rest).
pub fn balanced_bracket(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('[') {
        return None;
    }
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..i], s[i + 1..].trim_start()));
                }
            }
            _ => {}
        }
    }
    None
}

/// Normalize a message into a grouping key: digit runs become `#`, uuids
/// `<uuid>`, and (when `ids`) non-vanilla `ns:name` identifiers `<id>`.
pub fn normalize(msg: &str, ids: bool) -> String {
    let mut out = String::with_capacity(msg.len().min(240));
    let bytes = msg.as_bytes();
    let mut i = 0;
    while i < bytes.len() && out.len() < 220 {
        let c = bytes[i];
        if is_uuid_at(bytes, i) {
            out.push_str("<uuid>");
            i += 36;
        } else if c.is_ascii_digit() {
            out.push('#');
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
        } else if ids && let Some(len) = identifier_at(msg, i) {
            out.push_str("<id>");
            i += len;
        } else {
            let ch = msg[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_uuid_at(b: &[u8], i: usize) -> bool {
    if b.len() < i + 36 {
        return false;
    }
    b[i..i + 36].iter().enumerate().all(|(k, c)| match k {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

/// Length of a non-`minecraft:` `ns:path` identifier starting at `i` on a
/// word boundary.
fn identifier_at(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
        return None;
    }
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut j = i;
    while j < b.len() && word(b[j]) {
        j += 1;
    }
    if j == i || j >= b.len() || b[j] != b':' || &s[i..j] == "minecraft" {
        return None;
    }
    let mut k = j + 1;
    while k < b.len() && (word(b[k]) || b[k] == b'/' || b[k] == b'.') {
        k += 1;
    }
    (k > j + 1).then_some(k - i)
}

/// All `ns:path` identifiers in a message, vanilla included.
pub fn identifiers(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphanumeric() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if i < b.len() && b[i] == b':' {
                let mut k = i + 1;
                while k < b.len()
                    && (b[k].is_ascii_alphanumeric()
                        || b[k] == b'_'
                        || b[k] == b'/'
                        || b[k] == b'.')
                {
                    k += 1;
                }
                if k > i + 1 {
                    out.push(s[start..k].trim_end_matches('.'));
                    i = k;
                    continue;
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Namespace of an identifier (`lv_ce` from `lv_ce:slimy_grass`).
pub fn namespace(id: &str) -> Option<&str> {
    id.split_once(':')
        .map(|(ns, _)| ns)
        .filter(|ns| !ns.is_empty())
}

/// Text between the first pair of single quotes.
pub fn quoted(s: &str) -> Option<&str> {
    let start = s.find('\'')? + 1;
    let end = s[start..].find('\'')? + start;
    Some(&s[start..end])
}
