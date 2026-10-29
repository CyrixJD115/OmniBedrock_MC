import type { ServerStatus, ServerActionResponse, Backup, AddonList, Addon, Player, PropertyEntry, IniFile, WorldInfo, BackupSettings, SchedulerConfig, CommandEntry, IncludeItem, FolderEntry, BackendInfo, VersionRegistry, EndstoneRegistry, BroadcasterStatus, CfProject, StoreSearchResponse, StoreMeta, StoreFilesResponse, StoreInstallResponse, Instance, HostInfo } from '$types/index';

const API_BASE = '/api/v1';
let authToken = '';
let activeInstance = '';
// Pages can mount and fetch before the layout finishes booting the instance
// list, so seed the scope from the persisted selection (same key as
// stores/instances.ts activeInstanceId) — otherwise the very first requests
// carry no X-Instance header and the backend silently scopes them to default.
if (typeof localStorage !== 'undefined') {
  try {
    const raw = localStorage.getItem('omnibedrock.active-instance');
    const id = raw ? (JSON.parse(raw) as unknown) : null;
    if (typeof id === 'string' && id) activeInstance = id;
  } catch { /* keep unscoped */ }
}

export function setToken(token: string) {
  authToken = token;
}

export function getToken(): string {
  return authToken;
}

/** Scope every request (and instance-aware WS URLs) to one instance. */
export function setInstance(id: string) {
  activeInstance = id;
}

export function getInstance(): string {
  return activeInstance;
}

/** Headers for raw (non-JSON) fetches: auth + active-instance scoping. */
function rawHeaders(extra: Record<string, string> = {}): Record<string, string> {
  const headers: Record<string, string> = { ...extra };
  if (authToken) headers['Authorization'] = `Bearer ${authToken}`;
  if (activeInstance) headers['X-Instance'] = activeInstance;
  return headers;
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
  if (activeInstance) {
    headers['X-Instance'] = activeInstance;
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
  createUser: (data: { username: string; password: string; roles?: string[]; role?: string; display_name?: string }) =>
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

  // Instances & host
  listInstances: () => request<Instance[]>('/instances'),
  getInstanceDetail: (id: string) => request<Instance>(`/instances/${encodeURIComponent(id)}`),
  createInstance: (data: {
    name: string; backend: 'endstone' | 'vanilla'; deployment?: string;
    memory_mb?: number | null; cpus?: number | null; port?: number; vanilla_version?: string;
    endstone_version?: string;
  }) =>
    request<Instance>('/instances', {
      method: 'POST',
      body: JSON.stringify(data),
    }),
  updateInstance: (id: string, data: Record<string, unknown>) =>
    request<Instance>(`/instances/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify(data),
    }),
  deleteInstance: (id: string, keepData: boolean) =>
    request<{ success: boolean; message: string }>(
      `/instances/${encodeURIComponent(id)}?confirm=yes${keepData ? '&keep_data=1' : ''}`,
      { method: 'DELETE' },
    ),
  instanceAction: (id: string, action: string) =>
    request<ServerActionResponse>(`/instances/${encodeURIComponent(id)}/action`, {
      method: 'POST',
      body: JSON.stringify({ action }),
    }),
  getHost: () => request<HostInfo>('/host'),

  // Server
  getServerStatus: () => request<ServerStatus>('/server/status'),
  serverAction: (action: string) =>
    request<ServerActionResponse>('/server/action', {
      method: 'POST',
      body: JSON.stringify({ action }),
    }),

  // Server backend (Endstone / Vanilla BDS)
  getBackendInfo: () => request<BackendInfo>('/server/backend'),
  getVersionRegistry: () => request<VersionRegistry>('/server/backend/versions'),
  getEndstoneVersions: () => request<EndstoneRegistry>('/server/backend/endstone/versions'),
  installBds: (version: string) =>
    request<{ success: boolean; message: string }>('/server/backend/install', {
      method: 'POST',
      body: JSON.stringify({ version }),
    }),
  installEndstone: (version?: string) =>
    request<{ success: boolean; message: string }>('/server/backend/endstone/install', {
      method: 'POST',
      body: JSON.stringify(version ? { version } : {}),
    }),
  switchBackend: (backend: string, version?: string) =>
    request<{ success: boolean; message: string }>('/server/backend/switch', {
      method: 'POST',
      body: JSON.stringify(version ? { backend, version } : { backend }),
    }),
  getServerSettings: () => request<Record<string, unknown>>('/settings/server'),
  updateServerSettings: (data: Record<string, unknown>) =>
    request<{ success: boolean; message: string }>('/settings/server', {
      method: 'PUT',
      body: JSON.stringify(data),
    }),

  // MCXboxBroadcast
  getBroadcasterStatus: () => request<BroadcasterStatus>('/broadcaster/status'),
  broadcasterAction: (action: 'start' | 'stop' | 'restart' | 'takeover') =>
    request<{ success: boolean; message: string }>('/broadcaster/action', {
      method: 'POST',
      body: JSON.stringify({ action }),
    }),
  sendBroadcasterCommand: (command: string) =>
    request<{ success: boolean }>('/broadcaster/command', {
      method: 'POST',
      body: JSON.stringify({ command }),
    }),
  getBroadcasterConfig: () =>
    fetch(`${API_BASE}/broadcaster/config`, { headers: rawHeaders() }).then(r => {
      if (!r.ok) throw new Error(`API Error ${r.status}`);
      return r.text();
    }),
  saveBroadcasterConfig: (text: string) =>
    request<{ success: boolean }>('/broadcaster/config', {
      method: 'PUT',
      body: JSON.stringify({ text }),
    }),
  broadcasterAccount: (action: 'add' | 'remove', id: string) =>
    request<{ success: boolean; message: string }>('/broadcaster/accounts', {
      method: 'POST',
      body: JSON.stringify({ action, id }),
    }),
  uploadBroadcasterScreenshot: (account: string, file: File) => {
    return fetch(`${API_BASE}/broadcaster/screenshot?account=${encodeURIComponent(account)}`, {
      method: 'PUT',
      headers: rawHeaders(),
      body: file,
    }).then(async r => {
      if (!r.ok) throw new Error(await r.text());
      return r.json() as Promise<{ success: boolean; message: string }>;
    });
  },

  // Console
  sendCommand: (command: string) =>
    request<{ success: boolean }>('/console/command', {
      method: 'POST',
      body: JSON.stringify({ command }),
    }),

  // Properties
  getProperties: () => request<PropertyEntry[]>('/properties/'),
  getPropertiesRaw: () => fetch(`${API_BASE}/properties/raw`, { headers: rawHeaders() }).then(r => r.text()),
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
    const url = `${API_BASE}/backups/${encodeURIComponent(world)}/${encodeURIComponent(filename)}/download?token=${authToken}&instance=${encodeURIComponent(activeInstance || 'default')}`;
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
  listAddons: (refresh = false) => request<AddonList>(refresh ? '/addons/?refresh=1' : '/addons/'),
  uploadAddon: (world: string, file: File, packType: 'auto' | 'behavior_packs' | 'resource_packs' = 'auto') => {
    return fetch(`${API_BASE}/addons/upload?world=${encodeURIComponent(world)}&type=${packType}`, {
      method: 'POST',
      headers: rawHeaders(),
      body: file,
    }).then(async r => {
      const body = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(body.detail ?? `Upload failed (${r.status})`);
      return body as { success: boolean; message: string; installed: Array<{ name: string; pack_type: string; uuid: string; version: number[] }> };
    });
  },
  deleteAddon: (path: string) =>
    request<{ success: boolean; message: string }>('/addons/delete', {
      method: 'POST',
      body: JSON.stringify({ path }),
    }),
  addonIconUrl: (path: string, version?: string) =>
    `${API_BASE}/addons/icon?path=${encodeURIComponent(path)}&token=${authToken}&instance=${encodeURIComponent(activeInstance || 'default')}${version ? `&v=${encodeURIComponent(version)}` : ''}`,
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

  // Store (CurseForge)
  storeSearch: (params: { class?: string; q?: string; page?: number; pageSize?: number; sort?: string; version?: string; category?: string }, signal?: AbortSignal) => {
    const usp = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined && value !== null && `${value}` !== '') usp.set(key, `${value}`);
    }
    return request<StoreSearchResponse>(`/store/search?${usp.toString()}`, { signal });
  },
  storeMeta: () => request<StoreMeta>('/store/meta'),
  storeProject: (id: number) => request<CfProject>(`/store/project/${id}`),
  storeDescription: (id: number) => request<{ description: string }>(`/store/project/${id}/description`),
  storeFiles: (id: number, page = 1, pageSize = 20, version?: string) => {
    const usp = new URLSearchParams({ page: String(page), pageSize: String(pageSize) });
    if (version) usp.set('version', version);
    return request<StoreFilesResponse>(`/store/project/${id}/files?${usp.toString()}`);
  },
  storeInstall: (payload: { project_id: number; project_slug: string; file_id: number; world: string; target?: 'auto' | 'behavior_packs' | 'resource_packs' | 'world' }) =>
    request<StoreInstallResponse>('/store/install', {
      method: 'POST',
      body: JSON.stringify(payload),
    }),
  storeImageUrl: (url: string) =>
    `${API_BASE}/store/image?url=${encodeURIComponent(url)}&token=${authToken}`,

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
  createRole: (data: {
    name: string; display_name?: string; permissions: string[]; is_default?: boolean;
    description?: string; color?: string; position?: number; duplicate_of?: string;
  }) =>
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
  getPanelSettings: () => request<{ login_enabled: boolean; default_credentials: boolean }>('/settings/panel'),
  updatePanelSettings: (data: { login_enabled: boolean }) =>
    request<{ success: boolean; message: string }>('/settings/panel', {
      method: 'PUT',
      body: JSON.stringify(data),
    }),
  getCurseForgeSettings: () =>
    request<{ configured: boolean; key_hint: string | null; source: 'settings' | 'env' }>('/settings/curseforge'),
  updateCurseForgeKey: (api_key: string) =>
    request<{ success: boolean; message: string; configured: boolean }>('/settings/curseforge', {
      method: 'PUT',
      body: JSON.stringify({ api_key }),
    }),
};
