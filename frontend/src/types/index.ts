export interface ServerStatus {
  status: 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed';
  pid: number | null;
  uptime: number;
  version: string | null;
}

export interface ServerActionResponse {
  success: boolean;
  message: string;
}

export interface ConsoleLine {
  text: string;
  level: 'info' | 'warn' | 'error' | 'debug' | 'local';
  timestamp: number;
}

export interface ErrorStat {
  signature: string;
  count: number;
  first_seen: number;
  last_seen: number;
}

export interface Metrics {
  timestamp: number;
  status: string;
  cpu_percent: number;
  memory_mb: number;
  memory_percent: number;
  tps: number;
}

export interface Backup {
  filename: string;
  world: string;
  size_bytes: number;
  modified: string;
}

export interface AddonList {
  behavior_packs: Addon[];
  resource_packs: Addon[];
}

export interface Addon {
  name: string;
  path: string;
  world: string;
  pack_type: string;
  uuid: string;
  version: number[];
  valid: boolean;
}

export interface Player {
  name: string;
  uuid?: string;
  xuid?: string;
  ip?: string;
}

export interface PropertyEntry {
  key: string;
  value: string;
  comment: string;
  inline_comment: string;
}

export interface IniFile {
  name: string;
  path: string;
  size: number;
  modified: number;
}

export interface WorldInfo {
  name: string;
  path: string;
  size_bytes: number;
  files: number;
}

// --- Backup types ---

export interface CommandEntry {
  type: 'command' | 'wait' | 'comment' | 'send';
  value: string | number;
}

export interface PrePostConfig {
  before: CommandEntry[];
  after: CommandEntry[];
}

export interface BackupSettings {
  manual: Record<string, unknown>;
  auto: Record<string, unknown>;
  pre_post: PrePostConfig;
}

export interface BackupEvent {
  type: string;
  phase?: string;
  message?: string;
  percent?: number;
  stream?: string;
  line?: string;
  success?: boolean;
  filename?: string | null;
  active?: boolean;
}

export interface IncludeItem {
  name: string;
  is_dir: boolean;
}

export interface FolderEntry {
  name: string;
  path: string;
}

export interface SchedulerConfig {
  enabled: boolean;
  interval_minutes?: number;
  keep_count?: number;
  full_backup?: boolean;
  compression?: string;
  worlds?: string[];
}

export interface UserInfo {
  username: string;
  display_name: string;
  role: string;
  created_at: string;
}

// --- Permission / Role types ---

export const ALL_PERMISSIONS = [
  'CONSOLE_VIEW', 'CONSOLE_SEND',
  'SERVER_VIEW', 'SERVER_MANAGE',
  'PLAYERS_VIEW', 'PLAYERS_KICK', 'PLAYERS_BAN', 'PLAYERS_OP',
  'PROPERTIES_VIEW', 'PROPERTIES_EDIT',
  'ADDONS_VIEW', 'ADDONS_MANAGE',
  'BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE',
  'USERS_VIEW', 'USERS_CREATE', 'USERS_EDIT', 'USERS_DELETE',
  'FILES_VIEW', 'FILES_EDIT',
  'AUDIT_VIEW',
  'SETTINGS_VIEW', 'SETTINGS_EDIT',
] as const;

export type Permission = typeof ALL_PERMISSIONS[number];

export interface RoleInfo {
  name: string;
  display_name: string;
  permissions: string[];
  is_default: boolean;
  created_at: string;
}
