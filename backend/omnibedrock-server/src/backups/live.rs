//! Consistent backups of a running Bedrock server.
//!
//! Microsoft's documented procedure (bedrock_server_how_to.html): `save hold`,
//! poll `save query` until it answers "Data saved. Files are now ready to be
//! copied." followed by a `path:length, path:length, …` list, copy each listed
//! file truncated to its length (LevelDB keeps appending to `.log` and
//! `MANIFEST-*` while held), then `save resume`. Copying whole files instead
//! can capture half-written records and produce a world that fails to load.

use std::collections::HashMap;

/// Relative path inside the world folder → bytes to copy.
pub type Snapshot = HashMap<String, u64>;

/// Parse the console lines that followed a successful `save query`.
/// `lines` are (text, continued) pairs; a `continued` event carries the next
/// chunk of a line longer than the console's per-line cap, so it is joined to
/// the previous one without a separator.
pub fn parse_save_query(lines: &[(String, bool)], world: &str) -> Option<Snapshot> {
    let ready = lines.iter().position(|(l, _)| {
        let l = l.to_ascii_lowercase();
        l.contains("files are now ready") || l.contains("ready to be copied")
    })?;
    let mut joined = String::new();
    for (i, (line, continued)) in lines.iter().enumerate().skip(ready) {
        if i > ready && !continued {
            joined.push('\n');
        }
        joined.push_str(line);
    }
    let prefix = format!("{world}/");
    let start = joined.find(&prefix)?;
    let list = joined[start..].lines().next().unwrap_or_default();
    let mut out = Snapshot::new();
    for entry in list.split(", ") {
        let Some((path, len)) = entry.trim().rsplit_once(':') else {
            continue;
        };
        let Ok(len) = len.trim().parse::<u64>() else {
            continue;
        };
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if !rel.is_empty() && !rel.contains("..") {
            out.insert(rel.replace('\\', "/"), len);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::parse_save_query;

    fn l(s: &str, c: bool) -> (String, bool) {
        (s.to_owned(), c)
    }

    #[test]
    fn parses_the_ready_list_and_joins_continued_chunks() {
        let lines = vec![
            l(
                "[12:00:00 INFO] Data saved. Files are now ready to be copied.",
                false,
            ),
            l(
                "[12:00:00 INFO] Bedrock level/db/000005.ldb:2115, Bedrock level/db/CURRENT:16, Bedrock level/db/MANI",
                false,
            ),
            l(
                "FEST-000004:180, Bedrock level/level.dat:2512, Bedrock level/db/000003.log:0",
                true,
            ),
            l("[12:00:01 INFO] unrelated", false),
        ];
        let s = parse_save_query(&lines, "Bedrock level").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s["db/MANIFEST-000004"], 180);
        assert_eq!(s["level.dat"], 2512);
        assert_eq!(s["db/000003.log"], 0);
    }

    #[test]
    fn not_ready_yields_none() {
        let lines = vec![l("A previous save has not been completed.", false)];
        assert!(parse_save_query(&lines, "W").is_none());
    }
}
