//! Builds the compatibility report: scan logs and packs, resolve script-API
//! support, run static checks, then merge everything into ranked issues with
//! pack attribution, fix steps and a per-file timeline.

use super::housekeeping;
use super::knowledge;
use super::logs::{self, FileScan, Group};
use super::packs::Inventory;
use super::scan;
use super::script_api::{self, Ver, fmt_ver};
use super::statics::{self, Finding};
use super::types::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

/// Repeats at which a script error is promoted to critical (log flood).
const FLOOD: u64 = 10_000;
const MAX_ISSUES: usize = 400;

pub struct Inputs {
    pub server_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// `1.26.65.1` from the installed-version marker, when known.
    pub server_version: Option<String>,
    pub endstone_version: Option<String>,
    /// Live console lines, used when the server keeps no log files.
    pub console: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub world: Option<String>,
    pub max_files: usize,
    pub refresh: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            world: None,
            max_files: 14,
            refresh: false,
        }
    }
}

fn short_id(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    hex::encode(&digest[..6])
}

fn worlds(inputs: &Inputs, opts: &Options) -> Vec<String> {
    if let Some(w) = opts.world.clone().filter(|w| !w.is_empty()) {
        return vec![w];
    }
    let root = inputs.server_dir.join("worlds");
    let all: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    match housekeeping::level_name(&inputs.server_dir) {
        Some(level) if all.contains(&level) => vec![level],
        _ => all,
    }
}

struct Merged {
    g: Group,
    by_file: Vec<FileCount>,
}

fn merge(scans: &[FileScan]) -> Vec<Merged> {
    let mut out: BTreeMap<String, Merged> = BTreeMap::new();
    for scan in scans {
        for (key, g) in &scan.groups {
            let m = out.entry(key.clone()).or_insert_with(|| Merged {
                g: Group {
                    count: 0,
                    samples: Vec::new(),
                    first: g.first.clone(),
                    last: None,
                    c: g.c.clone(),
                },
                by_file: Vec::new(),
            });
            m.g.count += g.count;
            m.g.last = g.last.clone().or(m.g.last.take());
            for s in &g.samples {
                if m.g.samples.len() < 8 && !m.g.samples.contains(s) {
                    m.g.samples.push(s.clone());
                }
            }
            m.by_file.push(FileCount {
                label: scan.label.clone(),
                count: g.count,
            });
        }
    }
    out.into_values().collect()
}

fn timeline(scans: &[FileScan]) -> Vec<TimelineEntry> {
    let mut out: Vec<TimelineEntry> = Vec::new();
    let (mut version, mut endstone): (Option<String>, Option<String>) = (None, None);
    for s in scans {
        let changed = s.server_version.is_some()
            && version.is_some()
            && s.server_version != version
            || s.endstone_version.is_some() && endstone.is_some() && s.endstone_version != endstone;
        if s.server_version.is_some() {
            version = s.server_version.clone();
        }
        if s.endstone_version.is_some() {
            endstone = s.endstone_version.clone();
        }
        out.push(TimelineEntry {
            label: s.label.clone(),
            file: s.file.clone(),
            server_version: version.clone(),
            endstone_version: endstone.clone(),
            lines: s.lines,
            errors: s.errors,
            warnings: s.warnings,
            truncated: s.truncated,
            version_changed: changed,
        });
    }
    out
}

fn issue_from_log(m: Merged, inv: &Inventory, refs: &HashMap<String, Vec<String>>) -> Issue {
    let c = &m.g.c;
    let k = knowledge::lookup(c.rule);
    let pack = c.hint.as_ref().and_then(|h| inv.resolve(h));
    let mut fixes: Vec<String> = Vec::new();
    let mut severity = c.severity;
    if c.category == "script" && m.g.count >= FLOOD && severity >= Severity::Error {
        severity = Severity::Critical;
    }
    if let Some(p) = pack {
        if let Some((_, v)) = p
            .script_modules()
            .into_iter()
            .find(|(m, _)| m == "@minecraft/server")
            && c.category == "script"
        {
            fixes.push(format!(
                "{} declares @minecraft/server {v} ({}.x semantics).",
                p.r.name,
                v.split('.').next().unwrap_or("?")
            ));
        }
        if !p.active {
            fixes.push(format!(
                "{} is installed but not active in the world's pack list.",
                p.r.name
            ));
        }
    }
    if let Some(Hint::Namespace(ns)) = &c.hint {
        if matches!(c.rule, "content.unknown_block") && pack.is_none() {
            fixes.push(format!("No installed pack defines the '{ns}:' namespace."));
        }
        if let Some(users) = refs.get(ns).filter(|u| !u.is_empty()) {
            fixes.push(format!("Referenced by: {}.", users.join(", ")));
        }
    }
    fixes.extend(k.fixes.iter().map(|s| (*s).to_owned()));
    let context = match (pack, &c.location) {
        (Some(p), Some(loc)) if c.category == "script" => scan::context(p, loc),
        _ => None,
    };
    Issue {
        id: short_id(&c.key),
        rule: c.rule,
        severity,
        category: c.category,
        source: Source::Log,
        title: c.title.clone(),
        detail: c.detail.clone(),
        explanation: k.explanation.to_owned(),
        fixes,
        docs: k.docs.to_vec(),
        pack: pack.map(|p| p.r.clone()),
        hint: if pack.is_none() {
            c.hint.as_ref().map(Hint::label)
        } else {
            None
        },
        count: m.g.count,
        first_seen: m.g.first,
        last_seen: m.g.last,
        by_file: m.by_file,
        samples: m.g.samples,
        location: c.location.clone(),
        context,
    }
}

fn issue_from_finding(f: Finding, inv: &Inventory) -> Issue {
    let k = knowledge::lookup(f.rule);
    let pack = f.pack.and_then(|i| inv.packs.get(i));
    let mut fixes = f.extra_fixes;
    fixes.extend(k.fixes.iter().map(|s| (*s).to_owned()));
    let key = format!(
        "{}|{}|{}|{}",
        f.rule,
        pack.map(|p| p.r.folder.as_str()).unwrap_or(""),
        f.title,
        f.detail
    );
    Issue {
        id: short_id(&key),
        rule: f.rule,
        severity: f.severity,
        category: f.category,
        source: f.source,
        title: f.title,
        detail: f.detail,
        explanation: k.explanation.to_owned(),
        fixes,
        docs: k.docs.to_vec(),
        pack: pack.map(|p| p.r.clone()),
        hint: None,
        count: 1,
        first_seen: None,
        last_seen: None,
        by_file: Vec::new(),
        samples: f.samples,
        location: f.location,
        context: f.context,
    }
}

fn disk_issues(h: &Housekeeping, issues: &[Issue], scans: &[FileScan]) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut push = |rule: &'static str, severity, title: String, detail: String| {
        let k = knowledge::lookup(rule);
        out.push(Issue {
            id: short_id(rule),
            rule,
            severity,
            category: "housekeeping",
            source: Source::Disk,
            title,
            detail,
            explanation: k.explanation.to_owned(),
            fixes: k.fixes.iter().map(|s| (*s).to_owned()).collect(),
            docs: k.docs.to_vec(),
            pack: None,
            hint: None,
            count: 1,
            first_seen: None,
            last_seen: None,
            by_file: Vec::new(),
            samples: Vec::new(),
            location: None,
            context: None,
        });
    };
    if h.content_log_bytes > 200 * 1024 * 1024 {
        push(
            "housekeeping.content_logs",
            Severity::Warning,
            format!(
                "{} ContentLog files use {} MB",
                h.content_log_files,
                h.content_log_bytes / (1024 * 1024)
            ),
            format!(
                "content-log-file-enabled={}",
                h.content_log_file_enabled
                    .map_or("?".into(), |b| b.to_string())
            ),
        );
    }
    let busiest = scans.iter().map(|s| s.lines).max().unwrap_or(0);
    if busiest > 200_000
        && let Some(top) = issues
            .iter()
            .filter(|i| i.source == Source::Log)
            .max_by_key(|i| i.count)
    {
        push(
            "housekeeping.log_flood",
            Severity::Warning,
            format!("Log flood: up to {busiest} lines per log file"),
            format!("Top source: {} ({}×)", top.title, top.count),
        );
    }
    out
}

fn pack_health(inv: &Inventory, issues: &[Issue]) -> Vec<PackHealth> {
    let mut out: Vec<PackHealth> = inv
        .packs
        .iter()
        .map(|p| PackHealth {
            uuid: p.r.uuid.clone(),
            name: p.r.name.clone(),
            folder: p.r.folder.clone(),
            pack_type: p.r.pack_type.clone(),
            world: p.r.world.clone(),
            active: p.active,
            status: if p.active { "ok" } else { "inactive" },
            min_engine: p.manifest.as_ref().and_then(|m| {
                let a = m["header"]["min_engine_version"].as_array()?;
                Some(
                    a.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            }),
            script_modules: p
                .script_modules()
                .into_iter()
                .map(|(m, v)| format!("{m}@{v}"))
                .collect(),
            ..PackHealth::default()
        })
        .collect();
    for issue in issues {
        let Some(r) = &issue.pack else { continue };
        let Some(h) = out
            .iter_mut()
            .find(|h| h.folder == r.folder && h.pack_type == r.pack_type && h.world == r.world)
        else {
            continue;
        };
        h.issues += 1;
        let n = if issue.source == Source::Log {
            issue.count
        } else {
            0
        };
        h.events += n;
        match issue.severity {
            Severity::Critical | Severity::Error => {
                h.errors += n.max(1);
                h.status = "error";
            }
            Severity::Warning => {
                h.warnings += n.max(1);
                if h.status != "error" {
                    h.status = "warning";
                }
            }
            Severity::Info => {}
        }
    }
    out.sort_by(|a, b| {
        (b.errors, b.warnings, b.active)
            .cmp(&(a.errors, a.warnings, a.active))
            .then(a.name.cmp(&b.name))
    });
    out
}

/// Run the full analysis.
pub async fn analyze(mut inputs: Inputs, opts: Options) -> anyhow::Result<CompatReport> {
    let started = Instant::now();
    let world_list = worlds(&inputs, &opts);
    let server_dir = inputs.server_dir.clone();
    let max = opts.max_files.clamp(1, 60);
    let console = std::mem::take(&mut inputs.console);
    let (inv, scans) = tokio::task::spawn_blocking({
        let worlds = world_list.clone();
        move || {
            let inv = Inventory::load(&server_dir.join("worlds"), &worlds);
            let files = logs::discover(&server_dir, max);
            let mut scans = logs::scan_all(&files);
            if scans.is_empty() && !console.is_empty() {
                scans.push(logs::scan_lines(&console, "console"));
            }
            (inv, scans)
        }
    })
    .await?;
    let newest = |f: fn(&FileScan) -> Option<String>| scans.iter().rev().find_map(f);
    let version = inputs
        .server_version
        .clone()
        .or_else(|| newest(|s| s.server_version.clone()));
    let engine: Option<Ver> = version.as_deref().and_then(script_api::engine_of);
    let experiments = newest(|s| s.experiments.clone().map(|e| e.join(","))).map(|e| {
        e.split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    // `gtst` is the level.dat key of the "Beta APIs" experiment.
    let beta_apis = experiments.as_ref().map(|e| {
        e.iter()
            .any(|x| x == "gtst" || x.eq_ignore_ascii_case("beta apis"))
    });
    let modules: Vec<String> = inv
        .packs
        .iter()
        .filter(|p| p.active)
        .flat_map(|p| p.script_modules().into_iter().map(|(m, _)| m))
        .filter(|m| m.starts_with("@minecraft/"))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut modules = modules;
    modules.sort();
    let support = script_api::resolve(&inputs.cache_dir, &modules, engine, opts.refresh).await;
    let server_dir = inputs.server_dir.clone();
    let support_for_checks = support.clone();
    let (issues, packs, housekeeping, timeline_rows, bytes, truncated, files) =
        tokio::task::spawn_blocking(move || {
            let merged = merge(&scans);
            let unknown_ns: Vec<String> = merged
                .iter()
                .filter(|m| m.g.c.rule == "content.unknown_block")
                .filter_map(|m| match &m.g.c.hint {
                    Some(Hint::Namespace(ns)) if !inv.defines_namespace(ns) => Some(ns.clone()),
                    _ => None,
                })
                .collect();
            let refs = inv.referencing(&unknown_ns);
            let mut issues: Vec<Issue> = merged
                .into_iter()
                .map(|m| issue_from_log(m, &inv, &refs))
                .collect();
            for f in statics::check(&inv, engine, &support_for_checks, beta_apis)
                .into_iter()
                .chain(scan::scan(&inv))
            {
                issues.push(issue_from_finding(f, &inv));
            }
            let hk = housekeeping::measure(&server_dir);
            let extra = disk_issues(&hk, &issues, &scans);
            issues.extend(extra);
            issues.sort_by(|a, b| {
                b.severity
                    .cmp(&a.severity)
                    .then(b.count.cmp(&a.count))
                    .then(a.title.cmp(&b.title))
            });
            issues.truncate(MAX_ISSUES);
            let packs = pack_health(&inv, &issues);
            let bytes = scans.iter().map(|s| s.bytes).sum::<u64>();
            let truncated = scans.iter().any(|s| s.truncated);
            (
                issues,
                packs,
                hk,
                timeline(&scans),
                bytes,
                truncated,
                scans.len(),
            )
        })
        .await?;
    let count = |s: Severity| issues.iter().filter(|i| i.severity == s).count();
    let summary = Summary {
        critical: count(Severity::Critical),
        errors: count(Severity::Error),
        warnings: count(Severity::Warning),
        info: count(Severity::Info),
        events: issues
            .iter()
            .filter(|i| i.source == Source::Log)
            .map(|i| i.count)
            .sum(),
        packs_affected: packs
            .iter()
            .filter(|p| p.status == "error" || p.status == "warning")
            .count(),
        files_scanned: files,
        bytes_scanned: bytes,
        truncated,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    Ok(CompatReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        world: world_list
            .first()
            .cloned()
            .filter(|_| world_list.len() == 1),
        server: ServerInfo {
            version,
            engine: engine.map(fmt_ver),
            endstone_version: inputs.endstone_version.clone().or_else(|| {
                timeline_rows
                    .iter()
                    .rev()
                    .find_map(|t| t.endstone_version.clone())
            }),
            experiments: experiments.unwrap_or_default(),
            beta_apis,
        },
        script_api: support,
        summary,
        packs,
        issues,
        timeline: timeline_rows,
        housekeeping,
    })
}
