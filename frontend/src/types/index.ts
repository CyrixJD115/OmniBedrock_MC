export interface ServerStatus {
  status: 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed';
  pid: number | null;
  uptime: number;
  version: string | null;
  backend?: 'endstone' | 'vanilla';
}

export interface ServerActionResponse {
  success: boolean;
  message: string;
}

export interface ConsoleLine {
  text: string;
  level: 'info' | 'warn' | 'error' | 'debug' | 'local';
  timestamp: number;
  sequence?: number;
  stream_id?: string;
  source?: 'stdout' | 'stderr' | 'panel';
  continued?: boolean;
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
  /** Where the usage numbers come from: process-tree or docker-stats. */
  usage_source?: string;
  limits?: { memory_mb: number | null; cpus: number | null; enforcement: string };
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
  manifest: Record<string, unknown> | null;
  size_bytes?: number;
  modified?: number;
  icon?: boolean;
  /** 'curseforge' or 'manual' (legacy packs without a sidecar count as manual). */
  source?: string;
  store?: Record<string, unknown> | null;
  /** Lang-resolved description (pack.description key or manifest string). */
  description?: string | null;
  /** Fingerprint of the pack's change-sensitive mtimes; busts icon caching. */
  icon_version?: string;
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
  roles: string[];
  created_at: string;
}

// --- Permission / Role types ---

export const ALL_PERMISSIONS = [
  'CONSOLE_VIEW', 'CONSOLE_SEND',
  'SERVER_VIEW', 'SERVER_MANAGE',
  'PLAYERS_VIEW', 'PLAYERS_KICK', 'PLAYERS_BAN', 'PLAYERS_OP',
  'PROPERTIES_VIEW', 'PROPERTIES_EDIT',
  'ADDONS_VIEW', 'ADDONS_MANAGE',
  'STORE_VIEW', 'STORE_INSTALL',
  'BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE',
  'USERS_VIEW', 'USERS_CREATE', 'USERS_EDIT', 'USERS_DELETE',
  'ROLES_MANAGE',
  'FILES_VIEW', 'FILES_EDIT',
  'AUDIT_VIEW',
  'SETTINGS_VIEW', 'SETTINGS_EDIT',
  'BROADCASTER_VIEW', 'BROADCASTER_MANAGE',
  'INSTANCES_CREATE', 'INSTANCES_VIEW_ALL', 'INSTANCES_SHARE', 'INSTANCES_DELETE',
  'INSTANCES_LIMITS',
] as const;

/** Permissions that mean something on a single shared instance. */
export const INSTANCE_PERMISSIONS = [
  'CONSOLE_VIEW', 'CONSOLE_SEND',
  'SERVER_VIEW', 'SERVER_MANAGE',
  'PLAYERS_VIEW', 'PLAYERS_KICK', 'PLAYERS_BAN', 'PLAYERS_OP',
  'PROPERTIES_VIEW', 'PROPERTIES_EDIT',
  'ADDONS_VIEW', 'ADDONS_MANAGE',
  'STORE_VIEW', 'STORE_INSTALL',
  'BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE',
  'FILES_VIEW', 'FILES_EDIT',
  'BROADCASTER_VIEW',
  'INSTANCES_SHARE', 'INSTANCES_DELETE', 'INSTANCES_LIMITS',
] as const;

export type Permission = typeof ALL_PERMISSIONS[number];

export interface RoleInfo {
  name: string;
  display_name: string;
  description: string;
  /** Hex digits without '#', e.g. "5865F2". */
  color: string;
  permissions: string[];
  /** Higher = more senior; the list sorts by this. */
  position: number;
  is_default: boolean;
  builtin: boolean;
  /** Carried by the Owner role; never settable through the API. */
  grants_owner: boolean;
  user_count: number;
  created_at: string;
}

export interface AccessEntry {
  user: string;
  /** Catalog role names granted on the instance. */
  roles: string[];
}

// --- Dual backend / BDS version types ---

export type BackendKind = 'endstone' | 'vanilla';

export interface BdsInstallState {
  /** 'idle' when no install is running; otherwise a step name (downloading, python, venv, …). */
  phase: string;
  version?: string;
  detail?: string;
  progress?: number | null;
}

export interface EndstoneToolchain {
  installed: {
    uv_version: string;
    python_version: string;
    endstone_version?: string | null;
    installed_at?: number;
  } | null;
  binary_present: boolean;
  install: BdsInstallState;
}

export interface BackendInfo {
  backend: BackendKind;
  vanilla_version: string;
  running: boolean;
  installed: { version: string; requested?: string; installed_at?: number } | null;
  latest: string | null;
  binary_present: boolean;
  install: BdsInstallState;
  endstone: EndstoneToolchain;
}

export interface VersionRegistry {
  release: { latest: string; versions: string[] };
  preview: { latest: string; versions: string[] };
}

// --- MCXboxBroadcast types ---

export interface BroadcasterSnapshot {
  status: 'stopped' | 'running' | 'stopping' | 'crashed';
  pid: number | null;
  uptime: number;
}

export interface BroadcasterStatus {
  status: BroadcasterSnapshot;
  jar: string;
  jar_exists: boolean;
  java: string | null;
  config_exists: boolean;
  dir: string;
  accounts: BroadcasterAccount[];
  device_codes: Record<string, DeviceCode>;
}

export interface BroadcasterAccount {
  id: string;
  main: boolean;
}

export interface DeviceCode {
  account: string;
  code: string;
  url: string;
  at: number;
}

// --- CurseForge Store types ---

export interface CfAuthor {
  id?: number;
  name: string;
  url?: string;
  avatarUrl?: string;
}

export interface CfLogo {
  title?: string;
  thumbnailUrl?: string;
  url?: string;
}

export interface CfCategoryRef {
  id: number;
  name: string;
  slug?: string;
}

export interface CfLinks {
  websiteUrl?: string;
  wikiUrl?: string;
  issuesUrl?: string;
  sourceUrl?: string;
}

export interface CfScreenshot {
  id?: number;
  title?: string;
  thumbnailUrl?: string;
  url?: string;
}

export interface CfLatestFileIndex {
  gameVersion?: string;
  fileId: number;
  filename: string;
  releaseType?: number;
}

export interface CfProject {
  id: number;
  name: string;
  slug: string;
  summary: string;
  downloadCount: number;
  thumbsUpCount?: number;
  dateCreated?: string;
  dateModified?: string;
  dateReleased?: string;
  authors: CfAuthor[];
  logo?: CfLogo | null;
  categories: CfCategoryRef[];
  links?: CfLinks | null;
  classId?: number;
  mainFileId?: number;
  latestFilesIndexes: CfLatestFileIndex[];
  screenshots?: CfScreenshot[];
  isAvailable?: boolean;
}

export interface CfDependency {
  modId: number;
  relationType: number;
}

export interface CfFile {
  id: number;
  modId: number;
  displayName: string;
  fileName: string;
  /** 1 release, 2 beta, 3 alpha. */
  releaseType: number;
  fileDate?: string;
  fileLength: number;
  downloadCount: number;
  downloadUrl?: string | null;
  gameVersions: string[];
  isAvailable?: boolean;
  dependencies?: CfDependency[];
}

export interface CfPagination {
  index: number;
  pageSize: number;
  totalCount: number;
}

export interface StoreSearchResponse {
  results: CfProject[];
  pagination: CfPagination;
  page: number;
  pageSize: number;
}

export interface StoreClassInfo {
  slug: string;
  id: number;
}

export interface StoreMeta {
  classes: StoreClassInfo[];
  /** Sub-category options keyed by class id. */
  categories: Record<string, Array<{ id: number; name: string }>>;
  versions: string[];
}

export interface StoreFilesResponse {
  files: CfFile[];
  total: number;
  page: number;
  pageSize: number;
}

export interface StoreInstallResponse {
  success: boolean;
  message: string;
  as_world?: boolean;
  installed: Array<{ name: string; pack_type: string; uuid: string; version: number[] }>;
}



export interface ResourcePolicy {
  memory_mb: number | null;
  cpus: number | null;
}

export interface Instance {
  id: string;
  name: string;
  owner: string;
  created_at: string;
  port: number;
  deployment: 'native' | 'systemd' | 'docker';
  /** docker deployment only: image override + extra bind mounts. */
  docker?: { image?: string | null; extra_mounts?: string[] } | null;
  enforcement: 'advisory' | 'cgroup' | 'container';
  resources: ResourcePolicy;
  access: AccessEntry[];
  /** systemd instances: the user unit is enabled for boot-time autostart. */
  autostart?: boolean;
  backend: string;
  status: string;
  uptime: number;
  pid: number | null;
  usage?: { cpu_percent: number; memory_mb: number; disk_mb: number; usage_source: string };
}

export interface DeployCapabilities {
  docker: string | null;
  systemd: boolean;
  linger: boolean;
  /** Host OS: "linux" | "windows" | "macos" — drives the per-OS deployment default. */
  platform?: string;
}

export interface HostInfo {
  memory_total_mb: number;
  memory_used_mb: number;
  memory_percent: number;
  cpu_count: number;
  load_1: number;
  disk_total_mb?: number;
  disk_used_mb?: number;
  disk_percent?: number;
  capabilities: DeployCapabilities;
  allocated: { memory_mb: number; cpus: number; instances: number };
  reserve_mb: number;
  overcommit: {
    memory_exceeded: boolean;
    memory_warning: string | null;
    cpu_exceeded: boolean;
    cpu_warning: string | null;
  };
}

export interface UserQuotas {
  max_instances: number | null;
  memory_mb: number | null;
  cpus: number | null;
}
