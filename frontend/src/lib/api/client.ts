import type { ServerStatus, ServerActionResponse, Backup, AddonList, Addon, Player, PropertyEntry, IniFile, WorldInfo, BackupSettings, SchedulerConfig, CommandEntry, IncludeItem, FolderEntry } from '$types/index';

const API_BASE = '/api/v1';
let authToken = '';

export function setToken(token: string) {
  authToken = token;
}

export function getToken(): string {
  return authToken;
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string>),
  };
  if (authToken) {
    headers['Authorization'] = `Bearer ${authToken}`;
  }

  const res = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers,
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`API Error ${res.status}: ${text}`);
  }

  return res.json();
}

export const api = {
  // Auth
  login: (username: string, password: string) =>
    request<{ token: string; user: Record<string, unknown> }>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    }),
  getMe: () => request<Record<string, unknown>>('/auth/me'),
  listUsers: () => request<Array<Record<string, unknown>>>('/auth/users'),
  createUser: (data: { username: string; password: string; role: string; display_name?: string }) =>
    request<Record<string, unknown>>('/auth/users', {
      method: 'POST',
      body: JSON.stringify(data),
    }),
  updateUser: (username: string, data: Record<string, unknown>) =>
    request<Record<string, unknown>>(`/auth/users/${username}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    }),
  deleteUser: (username: string) =>
    request<{ success: boolean }>(`/auth/users/${username}`, { method: 'DELETE' }),

  // Server
  getServerStatus: () => request<ServerStatus>('/server/status'),
  serverAction: (action: string) =>
    request<ServerActionResponse>('/server/action', {
      method: 'POST',
      body: JSON.stringify({ action }),
    }),

  // Console
  sendCommand: (command: string) =>
    request<{ success: boolean }>('/console/command', {
      method: 'POST',
      body: JSON.stringify({ command }),
    }),

  // Properties
  getProperties: () => request<PropertyEntry[]>('/properties/'),
  getPropertiesRaw: () => fetch(`${API_BASE}/properties/raw`, { headers: { Authorization: `Bearer ${authToken}` } }).then(r => r.text()),
  updateProperty: (key: string, value: string) =>
    request<{ success: boolean }>(`/properties/${encodeURIComponent(key)}`, {
      method: 'PUT',
      body: JSON.stringify({ value }),
    }),
  savePropertiesRaw: (text: string) =>
    request<{ success: boolean }>('/properties/raw', {
      method: 'PUT',
      body: JSON.stringify({ text }),
    }),

  // Backups
  listWorlds: () => request<string[]>('/backups/worlds'),
  listBackups: (world?: string) => request<{ backups: Backup[]; total: number }>(`/backups/${world ? `?world=${world}` : ''}`),
  createBackup: (data: {
    world: string; tag?: string; full_backup?: boolean; zip_prefix?: string;
    export_folder?: string; compression?: string; include_items?: string[]; dry_run?: boolean;
  }) =>
    request<{ success: boolean; filename?: string }>('/backups/create', {
      method: 'POST',
      body: JSON.stringify(data),
    }),
  deleteBackup: (world: string, filename: string) =>
    request<{ success: boolean }>(`/backups/${world}/${filename}`, { method: 'DELETE' }),
  restoreBackup: (world: string, filename: string) =>
    request<{ success: boolean }>(`/backups/restore/${world}/${filename}`, { method: 'POST' }),
  listTrash: (world?: string) => request<Array<any>>(`/backups/trash${world ? `?world=${world}` : ''}`),
  downloadBackup: (world: string, filename: string) => {
    const url = `${API_BASE}/backups/${encodeURIComponent(world)}/${encodeURIComponent(filename)}/download?token=${authToken}`;
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  },
  restoreToWorld: (world: string, filename: string) =>
    request<{ success: boolean; message: string }>(`/backups/${encodeURIComponent(world)}/${encodeURIComponent(filename)}/restore-to-world`, {
      method: 'POST',
    }),
  getBackupSettings: () => request<BackupSettings>('/backups/settings'),
  updateBackupSettings: (data: Partial<BackupSettings>) =>
    request<{ success: boolean }>('/backups/settings', {
      method: 'PUT',
      body: JSON.stringify(data),
    }),
  getSchedulerConfig: () => request<SchedulerConfig>('/backups/scheduler'),
  updateScheduler: (cfg: SchedulerConfig) =>
    request<{ success: boolean }>('/backups/scheduler', {
      method: 'PUT',
      body: JSON.stringify(cfg),
    }),
  testCommand: (entry: CommandEntry) =>
    request<{ kind: string; output: string; exit_code: number }>('/backups/test-command', {
      method: 'POST',
      body: JSON.stringify({ entry }),
    }),
  getIncludeItems: (world: string) =>
    request<{ items: IncludeItem[] }>(`/backups/include-items?world=${encodeURIComponent(world)}`),
  listFolders: (base?: string) =>
    request<{ base: string; dirs: FolderEntry[] }>(`/backups/folders${base ? `?base=${encodeURIComponent(base)}` : ''}`),

  // Addons
  listAddons: () => request<AddonList>('/addons/'),
  getManifest: (path: string) => request<Record<string, unknown>>(`/addons/manifest?path=${encodeURIComponent(path)}`),
  updateManifest: (path: string, manifest: Record<string, unknown>) =>
    request<{ success: boolean }>('/addons/manifest', {
      method: 'PUT',
      body: JSON.stringify({ path, manifest }),
    }),
  getPackOrder: (world: string, packType: string) => request<Record<string, unknown>[]>(`/addons/order/${world}/${packType}`),
  setPackOrder: (world: string, packType: string, uuids: string[]) =>
    request<{ success: boolean }>(`/addons/order/${world}/${packType}`, {
      method: 'PUT',
      body: JSON.stringify({ pack_type: packType, uuids }),
    }),
  renameAddon: (path: string, newName: string) =>
    request<{ success: boolean; new_path: string }>('/addons/rename', {
      method: 'PUT',
      body: JSON.stringify({ path, new_name: newName }),
    }),
  randomizeUuid: (path: string) =>
    request<{ success: boolean; uuid: string }>('/addons/randomize-uuid', {
      method: 'POST',
      body: JSON.stringify({ path }),
    }),
  changeUuid: (path: string, uuid: string) =>
    request<{ success: boolean }>('/addons/change-uuid', {
      method: 'PUT',
      body: JSON.stringify({ path, uuid }),
    }),

  // Players
  listPlayers: () => request<{ players: Player[]; count: number }>('/players/'),
  playerAction: (action: string, target: string, reason?: string) =>
    request<{ success: boolean }>('/players/action', {
      method: 'POST',
      body: JSON.stringify({ action, target, reason: reason ?? '' }),
    }),

  // Worlds
  listWorldsInfo: () => request<string[]>('/worlds/'),
  getWorldInfo: (world: string) => request<WorldInfo>(`/worlds/${world}`),

  // Files/INI
  listIniFiles: () => request<IniFile[]>('/files/'),
  readIniFile: (filename: string) => request<{ name: string; content: string }>(`/files/${filename}`),
  writeIniFile: (filename: string, content: string) =>
    request<{ success: boolean }>(`/files/${filename}`, {
      method: 'PUT',
      body: JSON.stringify({ content }),
    }),
  deleteIniFile: (filename: string) =>
    request<{ success: boolean }>(`/files/${filename}`, { method: 'DELETE' }),

  // Roles
  listRoles: () => request<Array<Record<string, unknown>>>('/roles/'),
  createRole: (data: { name: string; display_name?: string; permissions: string[]; is_default?: boolean }) =>
    request<Record<string, unknown>>('/roles/', {
      method: 'POST',
      body: JSON.stringify(data),
    }),
  updateRole: (name: string, data: Record<string, unknown>) =>
    request<Record<string, unknown>>(`/roles/${encodeURIComponent(name)}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    }),
  deleteRole: (name: string) =>
    request<{ success: boolean }>(`/roles/${encodeURIComponent(name)}`, { method: 'DELETE' }),

  // Settings
  getSettings: () => request<Record<string, unknown>>('/settings/'),
};
