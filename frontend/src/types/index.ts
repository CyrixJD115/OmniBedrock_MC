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
