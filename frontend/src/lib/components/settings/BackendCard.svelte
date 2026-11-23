<script lang="ts">
  /**
   * Runtime of the active instance: vanilla BDS version switcher (with a
   * pack-compatibility preview for the selected version), the Endstone
   * toolchain, and switching between the two runtimes.
   */
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { activeInstance } from '$stores/instances';
  import type { BackendInfo, VersionRegistry, EndstoneRegistry } from '$types/index';
  import { Server, Download, RefreshCw, ArrowRightLeft } from '@lucide/svelte';
  import VersionCompatPreview from './VersionCompatPreview.svelte';

  let { canManage }: { canManage: boolean } = $props();

  let backend = $state<BackendInfo | null>(null);
  let registry = $state<VersionRegistry | null>(null);
  let endstoneRegistry = $state<EndstoneRegistry | null>(null);
  let channel = $state<'release' | 'preview'>('release');
  let selectedVersion = $state('latest');
  let selectedEndstone = $state('latest');
  let busy = $state('');
  let poll: ReturnType<typeof setInterval>;

  const installing = $derived(backend?.install && backend.install.phase !== 'idle');
  const endstoneInstalling = $derived(
    !!backend?.endstone.install && backend.endstone.install.phase !== 'idle'
  );
  const anyInstalling = $derived(!!installing || endstoneInstalling);
  const versions = $derived(channel === 'release' ? registry?.release.versions ?? [] : registry?.preview.versions ?? []);
  const updateAvailable = $derived(
    backend?.installed && backend.latest && backend.installed!.version !== backend.latest
  );
  const endstoneUpdateAvailable = $derived(backend?.endstone.update_available ?? false);
  /** The concrete version "latest" resolves to, for the preview. */
  const resolvedTarget = $derived(selectedVersion === 'latest' ? registry?.[channel].latest ?? '' : selectedVersion);

  async function refreshBackend() {
    try { backend = await api.getBackendInfo(); } catch { /* transient */ }
  }

  async function loadRegistry() {
    try { registry = await api.getVersionRegistry(); } catch (e: any) {
      addToast(`Version list unavailable: ${e.message}`, 'error');
    }
  }

  async function loadEndstoneRegistry() {
    try { endstoneRegistry = await api.getEndstoneVersions(); } catch { /* dropdown stays on 'latest' */ }
  }

  onMount(async () => {
    await loadRegistry();
    void loadEndstoneRegistry();
    // Fast (800 ms) only while an install runs; otherwise every ~5 s, and
    // never from a background tab.
    let tick = 0;
    poll = setInterval(async () => {
      tick += 1;
      if (document.hidden) return;
      if (!anyInstalling && tick % 6 !== 0) return;
      await refreshBackend();
      if (installing && backend) selectedVersion = backend.install.version ?? selectedVersion;
    }, 800);
  });

  // Reload when the active instance changes.
  let backendLoadedFor = $state('');
  $effect(() => {
    const id = $activeInstance?.id ?? 'default';
    if (backendLoadedFor === id) return;
    backendLoadedFor = id;
    void (async () => {
      await refreshBackend();
      if (backend) selectedVersion = backend.vanilla_version;
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
      const version = selectedEndstone === 'latest' ? undefined : selectedEndstone;
      const r = await api.installEndstone(version);
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
  function pct(info: BackendInfo): string {
    const progress = info.install?.progress;
    return typeof progress === 'number' ? `${Math.round(progress * 100)}%` : '';
  }
  function pctEndstone(): string {
    const progress = backend?.endstone.install?.progress;
    return typeof progress === 'number' ? `${Math.round(progress * 100)}%` : '';
  }
</script>

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
          <div class="mc-progress">
            <span style="width: {typeof backend.install.progress === 'number' ? Math.round(backend.install.progress * 100) : 10}%"></span>
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
          <div class="mc-progress">
            <span style="width: {typeof backend.endstone.install.progress === 'number' ? Math.round(backend.endstone.install.progress * 100) : 10}%"></span>
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
          <VersionCompatPreview version={resolvedTarget} installed={backend.installed?.version ?? null} />
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
              {#if backend.endstone.latest}
                <span class="text-[10px] text-deep-400 font-mono">latest {backend.endstone.latest}</span>
              {/if}
              {#if endstoneUpdateAvailable}
                <span class="text-[10px] text-yellow-400 uppercase tracking-wider">update available</span>
              {/if}
            {:else}
              <span class="text-[10px] text-yellow-400">Endstone toolchain not installed</span>
            {/if}
            <select bind:value={selectedEndstone} class="input text-xs w-auto font-mono">
              <option value="latest">latest{#if endstoneRegistry} ({endstoneRegistry.latest}){/if}</option>
              {#each endstoneRegistry?.versions ?? [] as v (v)}
                <option value={v}>{v}</option>
              {/each}
            </select>
            <button onclick={installEndstoneToolchain} disabled={!!busy}
                    class="btn-primary flex items-center gap-1.5 text-xs !bg-teal-600 hover:!bg-teal-500"
                    title="Downloads uv, a managed CPython, and Endstone into the panel data dir — no system Python required">
              <Download size={13} />
              {backend.endstone.installed ? 'Install / update' : 'Install Endstone toolchain'}
            </button>
            <button onclick={loadEndstoneRegistry} class="btn-ghost p-2" title="Refresh Endstone version list">
              <RefreshCw size={13} />
            </button>
          </div>
          <p class="text-[10px] text-deep-400 mt-2">
            The toolchain (uv + managed CPython + Endstone) lives in the panel data dir — no system Python needed.
            Starting without it also works: the panel installs it automatically first.
            Versions come from PyPI; <span class="font-mono">latest</span> updates the existing venv in place when a
            newer release exists, while a pinned version rebuilds the venv from scratch (also the repair path).
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
