//! "What breaks if I switch to version X?": the static checks (manifest
//! engine versions, script-module support, beta pins) evaluated against a
//! target engine and diffed with the current one. No logs are read, so it
//! is fast enough to run while the user picks a version.

use super::housekeeping;
use super::packs::Inventory;
use super::script_api;
use super::statics::{self, Finding};
use super::types::Severity;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize)]
pub struct PreviewIssue {
    pub pack: String,
    pub folder: String,
    pub severity: Severity,
    pub title: String,
    /// Only appears on the target version (a regression of the switch).
    pub new: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionPreview {
    pub current: Option<String>,
    pub target: String,
    pub world: Option<String>,
    /// `@minecraft/server` support on the target build, when known.
    pub target_script_api: Option<String>,
    pub issues: Vec<PreviewIssue>,
    /// New critical/error issues the switch would introduce.
    pub blocking: usize,
    /// Issues that exist now and go away on the target.
    pub resolved: usize,
}

fn key(f: &Finding, inv: &Inventory) -> String {
    let folder = f
        .pack
        .and_then(|i| inv.packs.get(i))
        .map(|p| p.r.folder.as_str())
        .unwrap_or("");
    format!("{}|{folder}|{}", f.rule, f.detail)
}

/// Preview static compatibility of the installed packs on `target`.
pub async fn preview(
    server_dir: PathBuf,
    cache_dir: PathBuf,
    world: Option<String>,
    current: Option<String>,
    target: String,
) -> anyhow::Result<VersionPreview> {
    let target_engine = script_api::engine_of(&target)
        .ok_or_else(|| anyhow::anyhow!("'{target}' is not a game version"))?;
    let current_engine = current.as_deref().and_then(script_api::engine_of);
    let worlds: Vec<String> = match world
        .clone()
        .or_else(|| housekeeping::level_name(&server_dir))
    {
        Some(w) => vec![w],
        None => Vec::new(),
    };
    let dir = server_dir.clone();
    let inv =
        tokio::task::spawn_blocking(move || Inventory::load(&dir.join("worlds"), &worlds)).await?;
    let mut modules: Vec<String> = inv
        .packs
        .iter()
        .filter(|p| p.active)
        .flat_map(|p| p.script_modules().into_iter().map(|(m, _)| m))
        .filter(|m| m.starts_with("@minecraft/"))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    modules.sort();
    let target_support =
        script_api::resolve(&cache_dir, &modules, Some(target_engine), false).await;
    let current_support = script_api::resolve(&cache_dir, &modules, current_engine, false).await;
    // Beta APIs state is not known without logs; assume unchanged (None).
    let on_target = statics::check(&inv, Some(target_engine), &target_support, None);
    let now: HashSet<String> = statics::check(&inv, current_engine, &current_support, None)
        .iter()
        .map(|f| key(f, &inv))
        .collect();
    let target_keys: HashSet<String> = on_target.iter().map(|f| key(f, &inv)).collect();
    let resolved = now.iter().filter(|k| !target_keys.contains(*k)).count();
    let mut issues: Vec<PreviewIssue> = on_target
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .map(|f| {
            let pack = f.pack.and_then(|i| inv.packs.get(i));
            PreviewIssue {
                pack: pack.map(|p| p.r.name.clone()).unwrap_or_default(),
                folder: pack.map(|p| p.r.folder.clone()).unwrap_or_default(),
                severity: f.severity,
                title: f.title.clone(),
                new: !now.contains(&key(f, &inv)),
            }
        })
        .collect();
    issues.sort_by_key(|i| std::cmp::Reverse((i.new, i.severity)));
    let blocking = issues
        .iter()
        .filter(|i| i.new && i.severity >= Severity::Error)
        .count();
    Ok(VersionPreview {
        current,
        target,
        world: world.or_else(|| housekeeping::level_name(&server_dir)),
        target_script_api: target_support
            .iter()
            .find(|s| s.module == "@minecraft/server")
            .and_then(|s| s.max_stable.clone()),
        issues,
        blocking,
        resolved,
    })
}
