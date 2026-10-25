<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import type { BackendInfo, VersionRegistry } from '$types/index';
  import { Settings as SettingsIcon, Server, Download, RefreshCw, Check, Save, ArrowRightLeft, LockKeyhole, LockOpen, Store as StoreIcon, Palette, Sun, Moon, Monitor } from '@lucide/svelte';
  import { setToken } from '$lib/api/client';
  import { currentUser, authToken as authTokenStore } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { THEMES, THEME_MODES, theme, themeMode } from '$lib/theme';

  let s = $state<Record<string, unknown>>({});
  let backend = $state<BackendInfo | null>(null);
  let registry = $state<VersionRegistry | null>(null);
  let channel = $state<'release' | 'preview'>('release');
  let selectedVersion = $state('latest');
  let busy = $state('');
  let serverSettings = $state<Record<string, unknown>>({});
  let panel = $state<{ login_enabled: boolean; default_credentials: boolean } | null>(null);
  let accessBusy = $state(false);
  let cfSettings = $state<{ configured: boolean; key_hint: string | null; source: 'settings' | 'env' } | null>(null);
  let cfKeyInput = $state('');
  let cfBusy = $state(false);
  let poll: ReturnType<typeof setInterval>;

  const canManage = $derived($userPermissions.includes('SERVER_MANAGE'));
  const canEditSettings = $derived($userPermissions.includes('SETTINGS_EDIT'));
  const installing = $derived(backend?.install && backend.install.phase !== 'idle');
  const endstoneInstalling = $derived(
    !!backend?.endstone.install && backend.endstone.install.phase !== 'idle'
  );
  const anyInstalling = $derived(!!installing || endstoneInstalling);
  const versions = $derived(channel === 'release' ? registry?.release.versions ?? [] : registry?.preview.versions ?? []);
  const updateAvailable = $derived(
    backend?.installed && backend.latest && backend.installed!.version !== backend.latest
  );

  async function refreshBackend() {
    try { backend = await api.getBackendInfo(); } catch { /* transient */ }
  }

  async function loadRegistry() {
    try { registry = await api.getVersionRegistry(); } catch (e: any) {
      addToast(`Version list unavailable: ${e.message}`, 'error');
    }
  }

  onMount(async () => {
    try { s = await api.getSettings(); } catch { /* silent */ }
    try { panel = await api.getPanelSettings(); } catch { /* silent */ }
    try { cfSettings = await api.getCurseForgeSettings(); } catch { /* silent */ }
    await loadRegistry();
    poll = setInterval(async () => {
      await refreshBackend();
      if (installing && backend) {
        // keep the selected version synced with what is being installed
        selectedVersion = backend.install.version ?? selectedVersion;
      }
    }, 800);
  });

  // The backend card edits the ACTIVE instance: reload its info and settings
  // when the user switches instances (same fix as the properties page).
  let backendLoadedFor = $state('');
  $effect(() => {
    const id = $activeInstance?.id ?? 'default';
    if (backendLoadedFor === id) return;
    backendLoadedFor = id;
    void (async () => {
      await refreshBackend();
      if (backend) selectedVersion = backend.vanilla_version;
      try { serverSettings = await api.getServerSettings(); } catch { /* silent */ }
    })();
  });

  onDestroy(() => clearInterval(poll));

  async function install() {
    if (!canManage || anyInstalling) return;
    busy = 'install';
    try {
      const version = selectedVersion || 'latest';
      const r = await api.installBds(version);
      addToast(r.message || 'Install started', 'success');
      await refreshBackend();
    } catch (e: any) { addToast(`Install failed: ${e.message}`, 'error'); }
    busy = '';
  }

  async function installEndstoneToolchain() {
    if (!canManage || anyInstalling) return;
    busy = 'endstone';
    try {
      const r = await api.installEndstone();
      addToast(r.message || 'Install started', 'success');
      await refreshBackend();
    } catch (e: any) { addToast(`Install failed: ${e.message}`, 'error'); }
    busy = '';
  }

  async function switchTo(kind: 'endstone' | 'vanilla') {
    if (!canManage || busy || backend?.backend === kind) return;
    busy = 'switch';
    try {
      const r = await api.switchBackend(kind, selectedVersion || undefined);
      addToast(r.message || `Switched to ${kind}`, 'success');
      await refreshBackend();
    } catch (e: any) { addToast(`Switch failed: ${e.message}`, 'error'); }
    busy = '';
  }

  async function saveServerSettings() {
    try {
      const r = await api.updateServerSettings(serverSettings);
      addToast(r.message || 'Saved', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  function pct(info: BackendInfo): string {
    const progress = info.install?.progress;
    return typeof progress === 'number' ? `${Math.round(progress * 100)}%` : '';
  }
  function pctEndstone(): string {
    const progress = backend?.endstone.install?.progress;
    return typeof progress === 'number' ? `${Math.round(progress * 100)}%` : '';
  }

  function num(field: string): number {
    return Number(serverSettings[field] ?? 0);
  }
  function setNum(field: string, value: number) {
    serverSettings = { ...serverSettings, [field]: value };
  }
  async function toggleLogin(event: Event) {
    const enabled = (event.target as HTMLInputElement).checked;
    accessBusy = true;
    try {
      const r = await api.updatePanelSettings({ login_enabled: enabled });
      addToast(r.message || 'Saved', 'success');
      if (panel) panel = { ...panel, login_enabled: enabled };
      if (enabled) {
        // Login is back on: the current token-less session ends at the login page.
        localStorage.removeItem('omb_token');
        localStorage.removeItem('omb_user');
        setToken('');
        authTokenStore.set('');
        currentUser.set(null as any);
        setTimeout(() => location.assign('/login'), 600);
      }
    } catch (e: any) {
      addToast(`Change failed: ${e.message}`, 'error');
      panel = panel ? { ...panel, login_enabled: !enabled } : panel;
    }
    accessBusy = false;
  }

  function toggleAutoRestart(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    serverSettings = { ...serverSettings, auto_restart: checked };
  }

  async function saveCurseForgeKey(clear = false) {
    if (cfBusy) return;
    const key = clear ? '' : cfKeyInput.trim();
    if (!clear && !key) {
      addToast('Paste your CurseForge API key first', 'error');
      return;
    }
    cfBusy = true;
    try {
      const r = await api.updateCurseForgeKey(key);
      addToast(r.message || 'Saved', 'success');
      cfKeyInput = '';
      cfSettings = {
        configured: r.configured,
        key_hint: clear ? null : cfSettings?.key_hint ?? '…saved',
        source: clear ? 'env' : 'settings',
      };
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
    }
    cfBusy = false;
  }
</script>

<div class="space-y-4">
  <div>
    <h1 class="page-title">Settings</h1>
    <div class="pixel-divider mt-2 w-24"></div>
  </div>

  <!-- Appearance: theme + light/dark, saved per browser -->
  <div class="card">
    <h2 class="card-header"><Palette size={13} /> Appearance</h2>
    <div class="space-y-4">
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Theme</p>
        <div class="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
          {#each THEMES as t (t.id)}
            <button onclick={() => theme.set(t.id)}
                    aria-pressed={$theme === t.id}
                    class={"text-left p-2.5 border-2 transition-all " + ($theme === t.id
                      ? 'border-bedrock-400 bg-bedrock-500/10'
                      : 'border-deep-600/50 bg-deep-900/40 hover:border-deep-500')}>
              <span class="flex items-center gap-2">
                <span class="w-6 h-6 shrink-0 border border-black/20 grid grid-cols-2 overflow-hidden" aria-hidden="true">
                  <i class="block" style="background:{t.swatch[0]}"></i><i class="block" style="background:{t.swatch[1]}"></i>
                </span>
                {#if $theme === t.id}
                  <span class="text-xs font-bold uppercase tracking-wider text-bedrock-300 truncate">{t.label}</span>
                  <Check size={13} class="text-bedrock-400 ml-auto shrink-0" />
                {:else}
                  <span class="text-xs font-bold uppercase tracking-wider text-deep-200 truncate">{t.label}</span>
                {/if}
              </span>
              <span class="block text-[10px] text-deep-400 mt-1.5 truncate">{t.hint}</span>
            </button>
          {/each}
        </div>
      </div>
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Mode</p>
        <div class="inline-flex border-2 border-deep-600/50 bg-deep-900/40 p-0.5 gap-0.5" role="group" aria-label="Color mode">
          {#each THEME_MODES as m (m.id)}
            <button onclick={() => themeMode.set(m.id)} aria-pressed={$themeMode === m.id}
                    class={"flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold uppercase tracking-wider transition-colors " + ($themeMode === m.id
                      ? 'bg-bedrock-600 text-white'
                      : 'text-deep-300 hover:bg-deep-700/60 hover:text-deep-100')}>
              {#if m.id === 'dark'}<Moon size={12} />
              {:else if m.id === 'light'}<Sun size={12} />
              {:else}<Monitor size={12} />{/if}
              {m.label}
            </button>
          {/each}
        </div>
        <p class="text-[10px] text-deep-400 mt-2">
          Applies instantly across the panel and is remembered in this browser. “System” follows your OS setting.
        </p>
      </div>
    </div>
  </div>

  <!-- Server backend -->
  <div class="card">
    <h2 class="card-header flex items-center gap-2">
      <Server size={13} /> Server backend
      {#if $activeInstance}
        <span class="ml-auto text-[10px] font-normal normal-case tracking-normal text-deep-400">
          editing <span class="text-strong font-bold">{$activeInstance.name}</span>
        </span>
      {/if}
    </h2>

    {#if backend}
      <div class="flex flex-wrap items-center gap-x-6 gap-y-1 text-xs">
        <span class="text-deep-400">Active runtime: <span class="text-strong capitalize font-bold">{backend.backend}</span></span>
        <span class="text-deep-400">Installed BDS:
          {#if backend.installed}<span class="font-mono text-strong">{backend.installed.version}</span>
          {:else}<span class="text-red-400">none</span>{/if}
        </span>
        {#if backend.latest}
          <span class="text-deep-400">Latest: <span class="font-mono text-strong">{backend.latest}</span></span>
          {#if updateAvailable}<span class="text-yellow-400">update available</span>{/if}
        {/if}
      </div>

      {#if installing}
        <div class="mt-4 border-2 border-bedrock-500/40 bg-bedrock-950/30 p-3">
          <div class="flex items-center justify-between text-xs mb-2">
            <span class="text-bedrock-300 uppercase tracking-wider">
              BDS {backend.install.phase} {backend.install.version ?? ''}
            </span>
            <span class="font-mono text-strong">{backend.install.detail ?? ''} {pct(backend)}</span>
          </div>
          <div class="h-2 bg-deep-800 overflow-hidden">
            <div class="h-full bg-bedrock-500 transition-all duration-300"
                 style="width: {typeof backend.install.progress === 'number' ? Math.round(backend.install.progress * 100) : 10}%"></div>
          </div>
          <p class="text-[10px] text-deep-400 mt-2">User data (worlds, server.properties, allowlist) is preserved; progress is also logged to the console.</p>
        </div>
      {/if}

      {#if endstoneInstalling}
        <div class="mt-4 border-2 border-teal-500/40 bg-teal-950/20 p-3">
          <div class="flex items-center justify-between text-xs mb-2">
            <span class="text-teal-300 uppercase tracking-wider">
              Endstone toolchain: {backend.endstone.install.phase}
            </span>
            <span class="font-mono text-strong">{backend.endstone.install.detail ?? ''} {pctEndstone()}</span>
          </div>
          <div class="h-2 bg-deep-800 overflow-hidden">
            <div class="h-full bg-teal-500 transition-all duration-300"
                 style="width: {typeof backend.endstone.install.progress === 'number' ? Math.round(backend.endstone.install.progress * 100) : 10}%"></div>
          </div>
          <p class="text-[10px] text-deep-400 mt-2">
            Downloading uv, a managed CPython, and Endstone into the panel data dir — no system Python needed. Progress is also logged to the console.
          </p>
        </div>
      {/if}

      {#if canManage && !anyInstalling}
        {#if backend.backend === 'vanilla'}
          <div class="mt-4 flex flex-wrap items-center gap-2">
            <select bind:value={channel} class="input text-xs w-auto">
              <option value="release">release</option>
              <option value="preview">preview</option>
            </select>
            <select bind:value={selectedVersion} class="input text-xs w-auto font-mono">
              <option value="latest">latest{#if registry} ({registry[channel].latest}){/if}</option>
              {#each versions as v (v)}
                <option value={v}>{v}</option>
              {/each}
            </select>
            <button onclick={install} disabled={!registry || !!busy}
                    class="btn-primary flex items-center gap-1.5 text-xs">
              <Download size={13} />
              {backend.installed ? 'Install / upgrade' : 'Install'} BDS
            </button>
            {#if backend.installed}
              <button onclick={() => switchTo('vanilla')} disabled={!!busy}
                      class="btn-ghost flex items-center gap-1.5 text-xs">
                <ArrowRightLeft size={13} /> Restart on selected version
              </button>
            {/if}
            <button onclick={loadRegistry} class="btn-ghost p-2" title="Refresh version list">
              <RefreshCw size={13} />
            </button>
          </div>
          <p class="text-[10px] text-deep-400 mt-2">
            Archives come from the official Minecraft CDN with SHA256 verification; the version registry is
            EndstoneMC/bedrock-server-data.
          </p>
        {:else}
          <div class="mt-4 flex flex-wrap items-center gap-2">
            {#if backend.endstone.installed}
              <span class="text-[10px] text-green-400 font-mono">
                toolchain ✓ endstone {backend.endstone.installed.endstone_version ?? '?'} · py {backend.endstone.installed.python_version}
              </span>
              <button onclick={installEndstoneToolchain} disabled={!!busy}
                      class="btn-primary flex items-center gap-1.5 text-xs !bg-teal-600 hover:!bg-teal-500">
                <Download size={13} /> Update Endstone toolchain
              </button>
            {:else}
              <span class="text-[10px] text-yellow-400">Endstone toolchain not installed</span>
              <button onclick={installEndstoneToolchain} disabled={!!busy}
                      class="btn-primary flex items-center gap-1.5 text-xs !bg-teal-600 hover:!bg-teal-500"
                      title="Downloads uv, a managed CPython, and Endstone into the panel data dir — no system Python required">
                <Download size={13} /> Install Endstone toolchain
              </button>
            {/if}
          </div>
          <p class="text-[10px] text-deep-400 mt-2">
            The toolchain (uv + managed CPython + Endstone) lives in the panel data dir — no system Python needed.
            Starting without it also works: the panel installs it automatically first.
          </p>
        {/if}
      {/if}

      <!-- Switch affordance: the only place the other runtime appears -->
      <div class="mt-4 border-2 border-dashed border-deep-600/40 p-3">
        <p class="text-[10px] text-deep-400 uppercase tracking-wider mb-2">Switch runtime</p>
        {#if backend.backend === 'vanilla'}
          <button onclick={() => switchTo('endstone')} disabled={!canManage || !!busy}
                  class="btn-secondary flex items-center gap-1.5 text-xs">
            <ArrowRightLeft size={13} /> Switch to Endstone
          </button>
          <p class="text-[10px] text-deep-400 mt-2">
            {#if backend.endstone.installed}Endstone toolchain is ready — the switch applies immediately.{:else}The Endstone toolchain installs automatically as part of the switch.{/if}
          </p>
        {:else}
          <button onclick={() => switchTo('vanilla')} disabled={!canManage || !!busy}
                  class="btn-secondary flex items-center gap-1.5 text-xs">
            <ArrowRightLeft size={13} /> Switch to Vanilla BDS
          </button>
          <p class="text-[10px] text-deep-400 mt-2">
            {#if backend.installed}BDS {backend.installed.version} is installed — the switch applies immediately.{:else}Vanilla BDS is not installed yet; it downloads automatically as part of the switch.{/if}
          </p>
        {/if}
        <p class="text-[10px] text-deep-400 mt-1">
          Worlds, allowlist and player permissions carry over — both runtimes share the same server directory.
          Switching restarts the server if it is running.
        </p>
      </div>
    {:else}
      <p class="text-xs text-deep-400">Backend information unavailable — retrying…</p>
    {/if}
  </div>

  <!-- Server supervisor settings -->
  <div class="card">
    <div class="flex items-center justify-between">
      <h2 class="card-header">Supervisor</h2>
      {#if canEditSettings}
        <button onclick={saveServerSettings} class="btn-primary flex items-center gap-1.5 text-xs">
          <Save size={13} /> Save
        </button>
      {/if}
    </div>
    <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
      <label class="flex items-center justify-between gap-3 text-xs text-deep-300">
        Auto-restart
        <input type="checkbox" checked={!!serverSettings.auto_restart}
               onchange={toggleAutoRestart}
               class="accent-bedrock-500 w-4 h-4" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Restart delay (s)</span>
        <input type="number" min="1" max="3600" value={num('auto_restart_delay')}
               onchange={(e) => setNum('auto_restart_delay', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Max crashes before giving up</span>
        <input type="number" min="1" max="100" value={num('max_crashes')}
               onchange={(e) => setNum('max_crashes', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Graceful stop timeout (s)</span>
        <input type="number" min="1" max="600" value={num('stop_timeout')}
               onchange={(e) => setNum('stop_timeout', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Kill timeout (s)</span>
        <input type="number" min="1" max="600" value={num('kill_timeout')}
               onchange={(e) => setNum('kill_timeout', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
    </div>
  </div>

  <!-- Access / login -->
  <div class="card">
    <h2 class="card-header flex items-center gap-2"><LockKeyhole size={13} /> Access</h2>
    <div class="space-y-3">
      <label class="flex items-center justify-between gap-3 text-xs text-deep-300">
        <span>
          Require login
          <span class="block text-[10px] text-deep-400 mt-0.5">
            {#if panel?.login_enabled === false}
              Login is disabled — everyone opening the panel acts as Admin.
            {:else}
              Show the login page and require an account.
            {/if}
          </span>
        </span>
        <input type="checkbox" checked={panel?.login_enabled ?? true} disabled={accessBusy || !canEditSettings}
               onchange={toggleLogin} class="accent-bedrock-500 w-4 h-4" />
      </label>
      {#if panel?.default_credentials}
        <p class="text-[10px] text-yellow-400/90 uppercase tracking-wider">
          <LockOpen size={10} class="inline mr-1" />
          Default credentials (Admin / 123456) are still active — change the password in Users.
        </p>
      {/if}
    </div>
  </div>

  <!-- CurseForge Store -->
  <div class="card">
    <div class="flex items-center justify-between">
      <h2 class="card-header flex items-center gap-2"><StoreIcon size={13} /> CurseForge Store</h2>
      {#if cfSettings}
        <span class="text-[10px] uppercase tracking-wider {cfSettings.configured ? 'text-green-400' : 'text-yellow-400'}">
          {cfSettings.configured ? `Configured (${cfSettings.source === 'settings' ? `settings ${cfSettings.key_hint}` : '.env'})` : 'Not configured'}
        </span>
      {/if}
    </div>
    <div class="space-y-3">
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">
          API key
          {#if cfSettings?.source === 'settings'}
            <span class="text-deep-400">— saved key {cfSettings.key_hint}; paste a new one to replace it</span>
          {:else}
            <span class="text-deep-400">— currently using CURSEFORGE_API_KEY from the .env file</span>
          {/if}
        </span>
        <div class="flex flex-wrap items-center gap-2">
          <input type="password" bind:value={cfKeyInput} disabled={!canEditSettings || cfBusy}
                 placeholder="Paste key from console.curseforge.com…"
                 autocomplete="off" class="input flex-1 min-w-56 text-xs font-mono py-2" />
          <button onclick={() => saveCurseForgeKey(false)} disabled={!canEditSettings || cfBusy || !cfKeyInput.trim()}
                  class="btn-primary text-xs px-3 py-2 flex items-center gap-1.5 disabled:opacity-40">
            <Save size={13} /> {cfBusy ? 'Saving…' : 'Save key'}
          </button>
          {#if cfSettings?.source === 'settings'}
            <button onclick={() => saveCurseForgeKey(true)} disabled={!canEditSettings || cfBusy}
                    class="btn-ghost text-xs px-3 py-2 disabled:opacity-40" title="Remove the saved key and fall back to .env">
              Use .env key
            </button>
          {/if}
        </div>
      </label>
      <p class="text-[10px] text-deep-400">
        Applies immediately — no restart needed. Get a free key at
        <a href="https://console.curseforge.com" target="_blank" rel="noopener noreferrer" class="text-bedrock-300">console.curseforge.com</a>.
        The key is stored in the panel's data dir and never shown in full again.
        Search access is approved by CurseForge per key and can take up to an hour.
      </p>
    </div>
  </div>

  <!-- Application info -->
  <div class="card">
    <h2 class="card-header flex items-center gap-2"><SettingsIcon size={13} /> Application</h2>
    <div class="space-y-3">
      {#each Object.entries(s) as [k, v]}
        <div class="flex items-center justify-between py-2 border-b border-deep-600/20 last:border-0">
          <span class="text-xs font-mono text-deep-300">{k}</span>
          <span class="text-xs text-deep-400">{String(v)}</span>
        </div>
      {/each}
    </div>
  </div>
</div>
