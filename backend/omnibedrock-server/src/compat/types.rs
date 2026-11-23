//! Shared report types for the add-on compatibility checker.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Where a finding came from.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Server log lines (latest.log, rotated logs, or the live console).
    Log,
    /// manifest.json / world pack-order inspection.
    Manifest,
    /// Heuristic scan of a pack's JavaScript.
    ScriptScan,
    /// Disk housekeeping (content logs, log growth).
    Disk,
}

/// What a raw finding points at before it is matched to an installed pack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hint {
    /// Pack display name as printed by `[Scripting] [<name>]`.
    Name(String),
    /// Pack folder, from a content-log path segment.
    Folder(String),
    /// Identifier namespace (`lv_ce` in `lv_ce:slimy_grass`).
    Namespace(String),
    /// Endstone plugin name.
    Plugin(String),
    /// Exact pack uuid (static checks).
    Uuid(String),
}

impl Hint {
    pub fn label(&self) -> String {
        match self {
            Self::Name(v) => v.clone(),
            Self::Folder(v) => format!("folder {v}"),
            Self::Namespace(v) => format!("namespace {v}:"),
            Self::Plugin(v) => format!("plugin {v}"),
            Self::Uuid(v) => format!("pack {v}"),
        }
    }
}

/// A script stack frame: `at fn (file:line)`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash, Default)]
pub struct Location {
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PackRef {
    pub uuid: String,
    pub name: String,
    pub folder: String,
    pub pack_type: String,
    pub world: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocLink {
    pub title: &'static str,
    pub url: &'static str,
}

/// Per-file occurrence count, for "when did this start".
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FileCount {
    pub label: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Issue {
    /// Stable signature id; the same problem keeps its id across scans.
    pub id: String,
    pub rule: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub source: Source,
    pub title: String,
    pub detail: String,
    pub explanation: String,
    pub fixes: Vec<String>,
    pub docs: Vec<DocLink>,
    pub pack: Option<PackRef>,
    /// Raw attribution when no installed pack matched.
    pub hint: Option<String>,
    pub count: u64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
    pub by_file: Vec<FileCount>,
    pub samples: Vec<String>,
    pub location: Option<Location>,
    /// Source excerpt around the failing function, when it could be found.
    pub context: Option<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct PackHealth {
    pub uuid: String,
    pub name: String,
    pub folder: String,
    pub pack_type: String,
    pub world: String,
    pub active: bool,
    pub status: &'static str,
    pub errors: u64,
    pub warnings: u64,
    pub events: u64,
    pub issues: usize,
    pub min_engine: Option<String>,
    pub script_modules: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct TimelineEntry {
    pub label: String,
    pub file: String,
    pub server_version: Option<String>,
    pub endstone_version: Option<String>,
    pub lines: u64,
    pub errors: u64,
    pub warnings: u64,
    pub truncated: bool,
    /// The version changed at this file (vs. the previous one).
    pub version_changed: bool,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct ModuleSupport {
    pub module: String,
    pub max_stable: Option<String>,
    pub beta: Vec<String>,
    /// "npm" (live registry, cached), "builtin" (offline table) or "unknown".
    pub source: &'static str,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct ServerInfo {
    pub version: Option<String>,
    pub engine: Option<String>,
    pub endstone_version: Option<String>,
    pub experiments: Vec<String>,
    pub beta_apis: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Housekeeping {
    pub content_log_files: u64,
    pub content_log_bytes: u64,
    pub logs_dir_bytes: u64,
    pub content_log_file_enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Summary {
    pub critical: usize,
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub events: u64,
    pub packs_affected: usize,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct CompatReport {
    pub generated_at: String,
    pub world: Option<String>,
    pub server: ServerInfo,
    pub script_api: Vec<ModuleSupport>,
    pub summary: Summary,
    pub packs: Vec<PackHealth>,
    pub issues: Vec<Issue>,
    pub timeline: Vec<TimelineEntry>,
    pub housekeeping: Housekeeping,
}
