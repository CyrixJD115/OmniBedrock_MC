// Add-on compatibility checker report (GET /addons/compat).

export type CompatSeverity = 'info' | 'warning' | 'error' | 'critical';
export type CompatSource = 'log' | 'manifest' | 'script-scan' | 'disk';

export interface CompatPackRef {
  uuid: string;
  name: string;
  folder: string;
  pack_type: string;
  world: string;
}

export interface CompatLocation {
  function: string | null;
  file: string | null;
  line: number | null;
}

export interface CompatIssue {
  id: string;
  rule: string;
  severity: CompatSeverity;
  category: string;
  source: CompatSource;
  title: string;
  detail: string;
  explanation: string;
  fixes: string[];
  docs: { title: string; url: string }[];
  pack: CompatPackRef | null;
  hint: string | null;
  count: number;
  first_seen: string | null;
  last_seen: string | null;
  by_file: { label: string; count: number }[];
  samples: string[];
  location: CompatLocation | null;
  context: string | null;
}

export interface CompatPackHealth {
  uuid: string;
  name: string;
  folder: string;
  pack_type: string;
  world: string;
  active: boolean;
  status: 'ok' | 'warning' | 'error' | 'inactive';
  errors: number;
  warnings: number;
  events: number;
  issues: number;
  min_engine: string | null;
  script_modules: string[];
}

export interface CompatTimelineEntry {
  label: string;
  file: string;
  server_version: string | null;
  endstone_version: string | null;
  lines: number;
  errors: number;
  warnings: number;
  truncated: boolean;
  version_changed: boolean;
}

export interface CompatReport {
  generated_at: string;
  world: string | null;
  server: {
    version: string | null;
    engine: string | null;
    endstone_version: string | null;
    experiments: string[];
    beta_apis: boolean | null;
  };
  script_api: { module: string; max_stable: string | null; beta: string[]; source: string }[];
  summary: {
    critical: number;
    errors: number;
    warnings: number;
    info: number;
    events: number;
    packs_affected: number;
    files_scanned: number;
    bytes_scanned: number;
    truncated: boolean;
    elapsed_ms: number;
  };
  packs: CompatPackHealth[];
  issues: CompatIssue[];
  timeline: CompatTimelineEntry[];
  housekeeping: {
    content_log_files: number;
    content_log_bytes: number;
    logs_dir_bytes: number;
    content_log_file_enabled: boolean | null;
  };
}

/** GET /addons/compat/preview?version=… */
export interface VersionPreview {
  current: string | null;
  target: string;
  world: string | null;
  target_script_api: string | null;
  issues: { pack: string; folder: string; severity: CompatSeverity; title: string; new: boolean }[];
  blocking: number;
  resolved: number;
}
