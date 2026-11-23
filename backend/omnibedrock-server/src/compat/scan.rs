//! Heuristic scan of behavior-pack JavaScript for API forms that the
//! declared `@minecraft/server` major no longer has (1.x code shipped with a
//! 2.x manifest), plus source excerpts for runtime stack frames.

use super::packs::{Inventory, Pack};
use super::script_api::parse_ver;
use super::statics::Finding;
use super::types::{Location, Severity, Source};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

const MAX_FILES: usize = 300;

/// `import { …Vector3… } from "@minecraft/server"`: a type-only name that
/// is not exported at runtime, so the importing module fails to load.
static VECTOR3_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"import\s*\{[^}]*\bVector3\b[^}]*\}\s*from\s*["']@minecraft/server["']"#)
        .expect("regex")
});
const MAX_BYTES: u64 = 24 * 1024 * 1024;

struct Pattern {
    rule: &'static str,
    needle: &'static str,
    title: &'static str,
    fix: &'static str,
    min_major: u64,
}

/// Removed or reshaped in @minecraft/server 2.0.0 (Microsoft Learn,
/// Scripting V2 overview + changelog).
const PATTERNS: &[Pattern] = &[
    Pattern {
        rule: "scan.is_valid_call",
        needle: ".isValid()",
        title: "Calls isValid() as a method (a property since 2.0.0)",
        fix: "Replace `.isValid()` with `.isValid`.",
        min_major: 2,
    },
    Pattern {
        rule: "scan.removed_api",
        needle: "runCommandAsync(",
        title: "Uses runCommandAsync (removed in 2.0.0)",
        fix: "Use `runCommand` (synchronous) or `system.runJob`.",
        min_major: 2,
    },
    Pattern {
        rule: "scan.removed_api",
        needle: "worldInitialize",
        title: "Subscribes to worldInitialize (removed in 2.0.0)",
        fix: "Use `system.beforeEvents.startup` for component registration and `world.afterEvents.worldLoad` for world access.",
        min_major: 2,
    },
    Pattern {
        rule: "scan.removed_api",
        needle: "world.playSound(",
        title: "Uses World.playSound (removed in 2.0.0)",
        fix: "Play the sound per player or per dimension instead.",
        min_major: 2,
    },
    Pattern {
        rule: "scan.removed_api",
        needle: "Events.itemUseOn",
        title: "Subscribes to itemUseOn (removed in 2.0.0)",
        fix: "Use `playerInteractWithBlock` events.",
        min_major: 2,
    },
];

fn line_of(text: &str, pos: usize) -> usize {
    text[..pos].bytes().filter(|b| *b == b'\n').count() + 1
}

fn scripts(pack: &Pack) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkDir::new(pack.dir.join("scripts"))
        .max_depth(10)
        .into_iter()
        .flatten()
    {
        if out.len() >= MAX_FILES {
            break;
        }
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|e| e == "js" || e == "mjs")
        {
            out.push(entry.into_path());
        }
    }
    out
}

fn server_major(pack: &Pack) -> Option<u64> {
    pack.script_modules()
        .into_iter()
        .find(|(m, _)| m == "@minecraft/server")
        .and_then(|(_, v)| parse_ver(&v).map(|v| v[0]))
}

fn rel(pack: &Pack, path: &Path) -> String {
    path.strip_prefix(pack.dir.join("scripts"))
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Scan every active behavior pack that ships scripts.
pub fn scan(inv: &Inventory) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, pack) in inv.packs.iter().enumerate() {
        if !pack.active || pack.r.pack_type != "behavior_packs" {
            continue;
        }
        let Some(major) = server_major(pack) else {
            continue;
        };
        let mut budget = MAX_BYTES;
        let mut hits: Vec<(usize, String)> = Vec::new();
        let (mut components, mut guards) = (0usize, 0usize);
        let mut vector3: Option<String> = None;
        for path in scripts(pack) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            budget = budget.saturating_sub(text.len() as u64);
            components += text.matches("getComponent(").count();
            guards += text.matches("isValid").count();
            for (p, pat) in PATTERNS.iter().enumerate() {
                if major < pat.min_major {
                    continue;
                }
                for (pos, _) in text.match_indices(pat.needle).take(3) {
                    hits.push((p, format!("{}:{}", rel(pack, &path), line_of(&text, pos))));
                }
            }
            if vector3.is_none()
                && let Some(m) = VECTOR3_IMPORT.find(&text)
            {
                vector3 = Some(format!(
                    "{}:{}",
                    rel(pack, &path),
                    line_of(&text, m.start())
                ));
            }
            if budget == 0 {
                break;
            }
        }
        for (p, pat) in PATTERNS.iter().enumerate() {
            let places: Vec<String> = hits
                .iter()
                .filter(|(k, _)| *k == p)
                .map(|(_, s)| s.clone())
                .collect();
            if places.is_empty() {
                continue;
            }
            let mut f = Finding::new(
                pat.rule,
                Severity::Error,
                "script",
                pat.title.into(),
                format!("declares @minecraft/server {major}.x"),
                Some(i),
            );
            f.source = Source::ScriptScan;
            f.extra_fixes.push(pat.fix.into());
            f.samples = places;
            out.push(f);
        }
        if let Some(place) = vector3 {
            let mut f = Finding::new(
                "scan.vector3_import",
                Severity::Error,
                "script",
                "Constructs Vector3 imported from @minecraft/server".into(),
                "Vector3 is an interface, not a runtime export".into(),
                Some(i),
            );
            f.source = Source::ScriptScan;
            f.samples = vec![place];
            out.push(f);
        }
        if major >= 2 && components >= 15 && guards * 10 < components {
            let mut f = Finding::new(
                "scan.no_validity_guards",
                Severity::Warning,
                "script",
                format!("{components} getComponent calls, {guards} isValid checks"),
                "Few validity guards for a 2.x script".into(),
                Some(i),
            );
            f.source = Source::ScriptScan;
            out.push(f);
        }
    }
    out
}

/// A short excerpt of the failing function, for a runtime frame. Works on
/// minified bundles (one huge line) by cutting a window around the match.
pub fn context(pack: &Pack, loc: &Location) -> Option<String> {
    let file = loc.file.as_deref()?;
    let target = scripts(pack).into_iter().find(|p| {
        let r = rel(pack, p).replace('\\', "/");
        r == file || r.ends_with(&format!("/{file}"))
    })?;
    let text = std::fs::read_to_string(&target).ok()?;
    let pos = match loc.function.as_deref() {
        Some(f) => [format!("function {f}("), format!("{f}(")]
            .iter()
            .find_map(|n| text.find(n.as_str()))?,
        None => {
            let line = loc.line? as usize;
            text.split_inline_offset(line)?
        }
    };
    let start = floor_char(&text, pos.saturating_sub(120));
    let end = floor_char(&text, (pos + 480).min(text.len()));
    let mut s = text[start..end].to_owned();
    if start > 0 {
        s.insert(0, '…');
    }
    if end < text.len() {
        s.push('…');
    }
    let header = format!("{} (line {})\n", rel(pack, &target), line_of(&text, pos));
    Some(header + &s)
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

trait LineOffset {
    fn split_inline_offset(&self, line: usize) -> Option<usize>;
}

impl LineOffset for String {
    fn split_inline_offset(&self, line: usize) -> Option<usize> {
        if line <= 1 {
            return Some(0);
        }
        self.match_indices('\n').nth(line - 2).map(|(i, _)| i + 1)
    }
}
