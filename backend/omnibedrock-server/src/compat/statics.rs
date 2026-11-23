//! Static manifest checks: engine and script-module versions against what
//! the server build supports, beta pinning and the Beta APIs experiment,
//! missing dependencies, duplicate uuids, invalid manifests.

use super::packs::Inventory;
use super::script_api::{Ver, fmt_ver, parse_ver};
use super::types::{Location, ModuleSupport, Severity, Source};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub source: Source,
    pub title: String,
    pub detail: String,
    /// Index into `Inventory::packs`.
    pub pack: Option<usize>,
    pub extra_fixes: Vec<String>,
    pub samples: Vec<String>,
    pub location: Option<Location>,
    pub context: Option<String>,
}

impl Finding {
    pub fn new(
        rule: &'static str,
        severity: Severity,
        category: &'static str,
        title: String,
        detail: String,
        pack: Option<usize>,
    ) -> Self {
        Self {
            rule,
            severity,
            category,
            source: Source::Manifest,
            title,
            detail,
            pack,
            extra_fixes: Vec::new(),
            samples: Vec::new(),
            location: None,
            context: None,
        }
    }
}

fn version_array(v: &Value) -> Option<Ver> {
    let a = v.as_array()?;
    Some([
        a.first()?.as_u64()?,
        a.get(1)?.as_u64()?,
        a.get(2).and_then(Value::as_u64).unwrap_or(0),
    ])
}

fn beta_base(v: &str) -> Option<&str> {
    v.split_once("-beta").map(|(b, _)| b)
}

/// Run every manifest check for the inventory's packs.
pub fn check(
    inv: &Inventory,
    engine: Option<Ver>,
    support: &[ModuleSupport],
    beta_apis: Option<bool>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let support: HashMap<&str, &ModuleSupport> =
        support.iter().map(|s| (s.module.as_str(), s)).collect();
    // Duplicate uuids per world.
    let mut seen: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, p) in inv.packs.iter().enumerate() {
        if !p.r.uuid.is_empty() {
            seen.entry((p.r.world.clone(), p.r.uuid.clone()))
                .or_default()
                .push(i);
        }
    }
    for ((_, uuid), idx) in seen.iter().filter(|(_, v)| v.len() > 1) {
        let names: Vec<String> = idx
            .iter()
            .map(|i| format!("{} ({})", inv.packs[*i].r.name, inv.packs[*i].r.folder))
            .collect();
        let mut f = Finding::new(
            "manifest.duplicate_uuid",
            Severity::Error,
            "manifest",
            format!("{} pack folders share uuid {uuid}", idx.len()),
            names.join(", "),
            idx.first().copied(),
        );
        f.samples = names;
        out.push(f);
    }
    for (i, pack) in inv.packs.iter().enumerate() {
        if let Some(err) = &pack.manifest_error {
            out.push(Finding::new(
                "manifest.invalid",
                if pack.active {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                "manifest",
                "manifest.json is missing or invalid".into(),
                err.clone(),
                Some(i),
            ));
            continue;
        }
        if !pack.active {
            continue;
        }
        let Some(m) = &pack.manifest else { continue };
        let header = &m["header"];
        if let (Some(min), Some(engine)) = (version_array(&header["min_engine_version"]), engine) {
            if min > engine {
                out.push(Finding::new(
                    "manifest.engine_too_new",
                    Severity::Error,
                    "manifest",
                    format!(
                        "Needs game {} but the server is {}",
                        fmt_ver(min),
                        fmt_ver(engine)
                    ),
                    format!("min_engine_version {:?}", min),
                    Some(i),
                ));
            } else if min < [1, 20, 0] && pack.r.pack_type == "behavior_packs" {
                out.push(Finding::new(
                    "manifest.legacy_engine",
                    Severity::Info,
                    "manifest",
                    format!("Built for game {}: old content schemas", fmt_ver(min)),
                    format!(
                        "min_engine_version {:?} on a {} server",
                        min,
                        fmt_ver(engine)
                    ),
                    Some(i),
                ));
            }
        }
        if m["capabilities"]
            .as_array()
            .is_some_and(|c| c.iter().any(|v| v == "script_eval"))
        {
            out.push(Finding::new(
                "manifest.script_eval",
                Severity::Info,
                "manifest",
                "Uses the script_eval capability".into(),
                "capabilities: [\"script_eval\"]".into(),
                Some(i),
            ));
        }
        for (module, version) in pack.script_modules() {
            let Some(s) = support.get(module.as_str()) else {
                continue;
            };
            if version == "beta" {
                out.push(Finding::new(
                    "manifest.beta_floating",
                    Severity::Warning,
                    "manifest",
                    format!("{module} floats on whatever beta the server build has"),
                    format!("{module} \"beta\""),
                    Some(i),
                ));
            } else if version.contains("beta") {
                let wanted = beta_base(&version).unwrap_or("");
                if !s.beta.is_empty() && !s.beta.iter().any(|b| beta_base(b) == Some(wanted)) {
                    let have = s
                        .beta
                        .iter()
                        .filter_map(|b| beta_base(b))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut f = Finding::new(
                        "manifest.beta_mismatch",
                        Severity::Critical,
                        "manifest",
                        format!(
                            "{module} '{version}' does not match this build's beta ({have}-beta)"
                        ),
                        format!("{module} {version}"),
                        Some(i),
                    );
                    f.extra_fixes.push(format!("This server provides {module} {have}-beta; set the dependency version to \"{have}-beta\" only if the pack's scripts work with it."));
                    out.push(f);
                }
            }
            if version.contains("beta") {
                if beta_apis == Some(false) {
                    out.push(Finding::new(
                        "manifest.beta_experiment",
                        Severity::Critical,
                        "manifest",
                        format!(
                            "{module} beta needs the Beta APIs experiment, which this world has off"
                        ),
                        format!("{module} {version}"),
                        Some(i),
                    ));
                }
                continue;
            }
            let (Some(want), Some(max)) = (
                parse_ver(&version),
                s.max_stable.as_deref().and_then(parse_ver),
            ) else {
                continue;
            };
            if want > max {
                let mut f = Finding::new(
                    "manifest.module_too_new",
                    Severity::Critical,
                    "manifest",
                    format!(
                        "Requires {module} {version}; this server supports up to {}",
                        fmt_ver(max)
                    ),
                    format!("{module} {version} > {}", fmt_ver(max)),
                    Some(i),
                );
                f.extra_fixes.push(format!(
                    "Server supports {module} ≤ {} (source: {}).",
                    fmt_ver(max),
                    s.source
                ));
                out.push(f);
            } else if module == "@minecraft/server" && want[0] < max[0] {
                out.push(Finding::new(
                    "manifest.legacy_scripts",
                    Severity::Info,
                    "manifest",
                    format!(
                        "Uses the legacy {module} {version} API (server is on {}.x)",
                        max[0]
                    ),
                    format!("{module} {version}"),
                    Some(i),
                ));
            }
        }
        // uuid dependencies inside the same world.
        if let Some(deps) = m["dependencies"].as_array() {
            for d in deps {
                let Some(uuid) = d["uuid"].as_str() else {
                    continue;
                };
                let uuid = uuid.to_ascii_lowercase();
                if !inv
                    .packs
                    .iter()
                    .any(|p| p.r.world == pack.r.world && p.r.uuid == uuid)
                {
                    out.push(Finding::new(
                        "manifest.missing_dependency",
                        Severity::Error,
                        "manifest",
                        format!("Depends on pack {uuid}, which is not installed"),
                        format!("dependency {uuid} version {}", d["version"]),
                        Some(i),
                    ));
                }
            }
        }
    }
    out
}
