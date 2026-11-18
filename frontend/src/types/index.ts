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

/** One Endstone plugin file in the instance's server/plugins directory. */
export interface PluginFile {
  filename: string;
  size: number;
  modified: number;
}

/** One entry of an explorer directory listing (symlinks are inert files). */
export interface ExplorerEntry {
  name: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

export interface ExplorerListing {
  path: string;
  entries: ExplorerEntry[];
  truncated: boolean;
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
  'PLUGIN_MANAGE',
  'STORE_VIEW', 'STORE_INSTALL',
  'BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE',
  'USERS_VIEW', 'USERS_CREATE', 'USERS_EDIT', 'USERS_DELETE',
  'ROLES_MANAGE',
  'FILES_VIEW', 'FILES_WRITE',
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
  'PLUGIN_MANAGE',
  'STORE_VIEW', 'STORE_INSTALL',
  'BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE',
  'FILES_VIEW', 'FILES_WRITE',
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
  /** Newest stable release per PyPI; null until the first registry fetch. */
  latest?: string | null;
  /** Installed endstone version differs from the registry latest. */
  update_available?: boolean;
}

export interface EndstoneRegistry {
  latest: string;
  /** Every published version, newest first. */
  versions: string[];
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
  /** config.yml relay targets with a co-location (public IP) verdict. */
  relay: RelayAnalysis;
  /** Panel-side query-ping health against the relay target. */
  ping: PingHealth;
  /** A broadcaster java for this instance the panel did not spawn. */
  external: ExternalBroadcaster | null;
  /** Version tag of config.yml on disk; moves when the file changes. */
  config_etag?: string;
  /** server.properties facts the broadcaster session should mirror. */
  server?: ServerFacts;
  /** The instance's game server state (the ping only means something while running). */
  server_status?: string;
}

export interface ServerFacts {
  port: number;
  transport: string;
  server_name: string | null;
  level_name: string | null;
  max_players: number | null;
  gamemode: string | null;
}

export interface RelayTarget {
  text: string;
  port?: number;
  public: boolean;
}

export interface RelayAnalysis {
  query_server: boolean;
  /** session.remote-address / remote-port — the operational offer-relay target. */
  relay: RelayTarget | null;
  /** session-info.ip / port — legacy pre-v7 keys the jar migrates on load. */
  legacy: RelayTarget | null;
  /** The instance's server-port. */
  server_port?: number;
  /** Relay port differs from the instance's server-port. */
  port_mismatch?: boolean;
}

export interface PongInfo {
  name: string;
  version: string;
  level: string;
  players: number | null;
  max_players: number | null;
}

export interface PingHealth {
  target: string | null;
  /** null = no verdict (stopped, server offline/booting, or query disabled). */
  ok: boolean | null;
  failing_since: number | null;
  last_checked: number | null;
  /** 'nethernet' (TCP signaling) or 'raknet' (UDP) — whichever answered. */
  protocol?: string | null;
  transport?: string | null;
  latency_ms?: number | null;
  server?: PongInfo | null;
  error?: string | null;
  server_running?: boolean | null;
  consecutive_failures?: number;
}

export interface ApiKeyRecord {
  id: string;
  user: string;
  name: string;
  scope: 'full' | 'read_only';
  prefix: string;
  created_at: string;
  last_used: string;
  last_ip: string;
  expires_at: string;
  expired: boolean;
}

export interface ExternalBroadcaster {
  pid: number;
  /** True when it was spawned by a previous panel run (pidfile match). */
  orphaned: boolean;
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

export interface DeliveredPack {
  uuid: string;
  name: string;
  folder: string;
  version: number[];
  bytes: number;
  files: number;
  subpacks: boolean;
  unpaired: boolean;
  version_mismatch: boolean;
}

/** GET /addons/delivery — what a joining client downloads for one world. */
export interface PackDelivery {
  world: string;
  packs: DeliveredPack[];
  total_bytes: number;
  missing: string[];
  console_heavy: boolean;
  texturepack_required: boolean | null;
}

/** GET /broadcaster/jar */
export interface BroadcasterJarInfo {
  jar: string;
  jar_exists: boolean;
  running_build: string | null;
  running_bedrock: string | null;
  installed_build: string | null;
  latest: { tag: string; published_at: string; size: number } | null;
  latest_error: string | null;
  update_available: boolean;
}

/** GET /addons/cdn — optional pack CDN (cdn_config.json) status. */
export interface PackCdnStatus {
  state: {
    settings: { enabled: boolean; base_url: string };
    bucket: string;
    world: string;
    config_present: boolean;
    config_owned: boolean;
    uncovered: string[];
    files: number;
    total_bytes: number;
  };
  /** What BDS logged about cdn_config.json during this run. */
  bds: { status: 'loaded' | 'mismatch' | 'parse_error' | 'absent'; line: string } | null;
  suggestions: string[];
  panel_port: number;
  message?: string;
}

export interface PackCdnTest {
  url: string;
  ok: boolean;
  status: number | null;
  content_type: string | null;
  content_length: number | null;
  expected_length: number;
  detail: string;
}
