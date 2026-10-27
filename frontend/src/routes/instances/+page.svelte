<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, userPermissions } from '$stores/auth';
  import { instances, hostInfo, activeInstanceId, loadHost, refreshInstances, switchInstance } from '$stores/instances';
  import type { Instance, AccessEntry, RoleInfo } from '$types/index';
  import {
    Layers, Plus, Play, Square, RotateCw, Terminal, Settings2, Trash2, RefreshCw,
    Cpu, MemoryStick, ServerCog, ShieldCheck, ShieldAlert, Download, Users2, HardDrive
  } from '@lucide/svelte';
  import { goto } from '$app/navigation';

  let loading = $state(true);
  let acting = $state<string | null>(null);
  let showCreate = $state(false);
  let editing = $state<Instance | null>(null);
  let deleting = $state<Instance | null>(null);
  let deleteKeepData = $state(false);
  let poll: ReturnType<typeof setInterval> | undefined;

  let form = $state({
    name: '',
    backend: 'vanilla' as 'vanilla' | 'endstone',
    deployment: 'native',
    dockerImage: '',
    dockerMounts: '',
    memory: '' as string | number,
    cpus: '' as string | number,
    port: '' as string | number,
    vanillaVersion: 'latest',
    endstoneVersion: '',
    installAfter: true,
    autostart: false,
  });
  /** Per-OS deployment default: no official macOS bedrock_server binary, so
   *  macOS prefers docker; Windows and Linux stay native (opt-in docker). */
  function defaultDeployment(): 'native' | 'docker' {
    return $hostInfo?.capabilities?.platform === 'macos' ? 'docker' : 'native';
  }
  function emptyForm() {
    return {
      name: '',
      backend: 'vanilla' as 'vanilla' | 'endstone',
      deployment: defaultDeployment(),
      dockerImage: '',
      dockerMounts: '',
      memory: '',
      cpus: '',
      port: '',
      vanillaVersion: 'latest',
      endstoneVersion: '',
      installAfter: true,
      autostart: false,
    };
  }
  function parseMounts(raw: string): string[] {
    return raw
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
  }
  let vanillaVersions = $state<string[]>([]);
  let versionsLoading = $state(false);

  async function loadVersions() {
    if (vanillaVersions.length > 0 || versionsLoading) return;
    versionsLoading = true;
    try {
      const registry = await api.getVersionRegistry();
      // Release channel only; previews stay available via Settings → Backend.
      vanillaVersions = (registry.release?.versions ?? []).slice(0, 40);
    } catch {
      vanillaVersions = [];
    }
    versionsLoading = false;
  }
  let editForm = $state({
    name: '',
    port: '' as string | number,
    memory: '' as string | number,
    cpus: '' as string | number,
    deployment: 'native',
    dockerImage: '',
    dockerMounts: '',
    autostart: false,
    access: [] as AccessEntry[],
  });
  let accessUser = $state('');
  let accessRoles = $state<Set<string>>(new Set(['viewer']));
  let users = $state<string[]>([]);
  let roleCatalog = $state<RoleInfo[]>([]);
  let creating = $state(false);

  function roleInfo(name: string): RoleInfo | undefined {
    return roleCatalog.find((r) => r.name === name);
  }
  /** Frontend mirror of the backend rule; the API stays the boundary. */
  function entryPerms(i: Instance): Set<string> {
    const perms = new Set<string>();
    const entry = i.access.find((a) => a.user === $currentUser?.username);
    if (entry) {
      for (const r of entry.roles) {
        for (const p of roleInfo(r)?.permissions ?? []) perms.add(p);
      }
    }
    if ($userPermissions.includes('INSTANCES_VIEW_ALL')) {
      for (const p of $userPermissions) perms.add(p);
    }
    return perms;
  }

  onMount(async () => {
    await refreshInstances();
    void loadHost();
    if ($userPermissions.includes('USERS_VIEW')) {
      try { users = ((await api.listUsers()) as { username: string }[]).map((u) => u.username); } catch { /* ignore */ }
    }
    try { roleCatalog = (await api.listRoles()) as never; } catch { /* chips degrade gracefully */ }
    loading = false;
    poll = setInterval(() => void refreshInstances(), 5000);
  });
  onDestroy(() => { if (poll) clearInterval(poll); });

  function isPanelOwner(): boolean {
    return $currentUser?.role === 'owner' || $userPermissions.includes('INSTANCES_VIEW_ALL');
  }
  function owns(i: Instance): boolean {
    return isPanelOwner() || i.owner === $currentUser?.username;
  }
  function canControl(i: Instance): boolean {
    if (owns(i)) return true;
    return entryPerms(i).has('SERVER_MANAGE');
  }
  function canEdit(i: Instance): boolean {
    if (owns(i)) return true;
    return entryPerms(i).has('INSTANCES_LIMITS') || $userPermissions.includes('INSTANCES_SHARE');
  }

  function deploymentBadge(d: string): string {
    if (d === 'docker') return 'text-sky-400 border-sky-500/40 bg-sky-500/10';
    if (d === 'systemd') return 'text-violet-400 border-violet-500/40 bg-violet-500/10';
    return 'text-deep-300 border-deep-500/40 bg-deep-600/20';
  }
  function enforcementNote(i: Instance): string {
    if (i.enforcement === 'container') return 'Limits enforced by the container runtime (hard cgroup limits; OOM kills the container).';
    if (i.enforcement === 'cgroup') return 'Runs as a systemd user service: MemoryMax/CPUQuota enforced, and the server keeps running through panel restarts.';
    return 'Advisory only: usage is reported, but the OS does not enforce these limits on native processes.';
  }
  function memBar(i: Instance): number {
    const limit = i.resources.memory_mb;
    if (!limit || !i.usage) return 0;
    return Math.min(100, (i.usage.memory_mb / limit) * 100);
  }
  function fmtMb(n: number): string {
    return n >= 1024 ? `${(n / 1024).toFixed(1)} GB` : `${Math.round(n)} MB`;
  }
  function fmtUptime(s: number): string {
    if (!s) return '—';
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }

  async function act(i: Instance, action: string) {
    acting = `${i.id}:${action}`;
    try {
      const res = await api.instanceAction(i.id, action);
      addToast(res.message, res.success ? 'success' : 'error');
      setTimeout(() => void refreshInstances(), 1500);
    } catch (e) {
      addToast(`Action failed: ${(e as Error).message}`, 'error');
    }
    acting = null;
  }

  async function create() {
    if (!form.name.trim()) { addToast('Name required', 'error'); return; }
    creating = true;
    try {
      const body: Record<string, unknown> = { name: form.name.trim(), backend: form.backend, deployment: form.deployment };
      if (form.deployment === 'systemd') body.autostart = form.autostart;
      if (form.deployment === 'docker') {
        body.docker = {
          image: form.dockerImage.trim() || null,
          extra_mounts: parseMounts(form.dockerMounts),
        };
      }
      if (form.memory !== '' && form.memory !== null) body.memory_mb = Number(form.memory) || null;
      if (form.cpus !== '' && form.cpus !== null) body.cpus = Number(form.cpus) || null;
      if (form.port !== '' && form.port !== null) body.port = Number(form.port);
      if (form.backend === 'vanilla' && form.vanillaVersion) body.vanilla_version = form.vanillaVersion;
      const created = await api.createInstance(body as never);
      addToast(`Instance '${created.name}' created`, 'success');
      showCreate = false;
      await refreshInstances();
      if (form.installAfter && created.id !== 'default') {
        // Seed the runtime the instance will launch with; progress streams
        // into that instance's console.
        switchInstance(created.id);
        try {
          if (form.backend === 'vanilla') {
            await api.installBds(form.vanillaVersion || 'latest');
            addToast(`Vanilla BDS ${form.vanillaVersion || 'latest'} install started — open the console to watch`, 'success');
          } else {
            const version = form.endstoneVersion.trim();
            await api.installEndstone(version || undefined);
            addToast(`Endstone ${version || 'latest'} install started — open the console to watch`, 'success');
          }
        } catch (e) {
          addToast(`Install not started: ${(e as Error).message}`, 'error');
        }
        goto('/console');
      }
    } catch (e) {
      addToast(`Create failed: ${(e as Error).message}`, 'error');
    }
    creating = false;
  }

  function openEdit(i: Instance) {
    editing = i;
    editForm = {
      name: i.name,
      port: i.port,
      memory: i.resources.memory_mb ?? '',
      cpus: i.resources.cpus ?? '',
      deployment: i.deployment,
      dockerImage: i.docker?.image ?? '',
      dockerMounts: (i.docker?.extra_mounts ?? []).join('\n'),
      autostart: i.autostart ?? false,
      access: [...i.access],
    };
  }

  function addAccess() {
    const user = accessUser.trim();
    if (!user) return;
    if (editForm.access.some((a) => a.user === user)) return;
    const roles = [...accessRoles];
    if (roles.length === 0) return;
    editForm.access = [...editForm.access, { user, roles }];
    accessUser = '';
    accessRoles = new Set(['viewer']);
  }

  function toggleEntryRole(entry: AccessEntry, role: string) {
    const roles = entry.roles.includes(role)
      ? entry.roles.filter((r) => r !== role)
      : [...entry.roles, role];
    editForm.access = editForm.access.map((a) =>
      a.user === entry.user ? { ...a, roles } : a
    );
  }

  async function saveEdit() {
    if (!editing) return;
    const body: Record<string, unknown> = {
      name: editForm.name.trim(),
      deployment: editForm.deployment,
      access: editForm.access,
      port: Number(editForm.port) || 0,
    };
    body.autostart = editForm.deployment === 'systemd' && editForm.autostart;
    if (editForm.deployment === 'docker') {
      body.docker = {
        image: editForm.dockerImage.trim() || null,
        extra_mounts: parseMounts(editForm.dockerMounts),
      };
    }
    body.memory_mb = editForm.memory === '' ? null : Number(editForm.memory);
    body.cpus = editForm.cpus === '' ? null : Number(editForm.cpus);
    try {
      await api.updateInstance(editing.id, body);
      addToast('Instance updated', 'success');
      editing = null;
      await refreshInstances();
    } catch (e) {
      addToast(`Update failed: ${(e as Error).message}`, 'error');
    }
  }

  async function remove() {
    if (!deleting) return;
    try {
      const res = await api.deleteInstance(deleting.id, deleteKeepData);
      addToast(res.message ?? 'Instance deleted', 'success');
      if ($activeInstanceId === deleting.id) switchInstance('default');
      deleting = null;
      deleteKeepData = false;
      await refreshInstances();
    } catch (e) {
      addToast(`Delete failed: ${(e as Error).message}`, 'error');
    }
  }

  function open(i: Instance) {
    switchInstance(i.id);
    goto('/console');
  }

  let hostPercent = $derived($hostInfo ? Math.min(100, $hostInfo.memory_percent) : 0);
  let diskPercent = $derived($hostInfo?.disk_percent ? Math.min(100, $hostInfo.disk_percent) : 0);
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Instances</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={() => { void refreshInstances(); void loadHost(); }} class="btn-secondary text-xs flex items-center gap-1.5">
        <RefreshCw size={12} /> Refresh
      </button>
      {#if $userPermissions.includes('INSTANCES_CREATE')}
        <button onclick={() => { form = emptyForm(); void loadVersions(); showCreate = true; }}
                class="btn-primary text-xs flex items-center gap-1.5">
          <Plus size={12} /> New Instance
        </button>
      {/if}
    </div>
  </div>

  {#if $userPermissions.includes('SERVER_VIEW') && $hostInfo && ($hostInfo.overcommit.memory_exceeded || $hostInfo.overcommit.cpu_exceeded)}
    <div class="card border-l-4 {($hostInfo.overcommit.memory_exceeded && $hostInfo.memory_percent > 90) ? 'border-red-500' : 'border-amber-500'}">
      <h2 class="card-header flex items-center gap-2 text-amber-400">Overcommitment warning</h2>
      <ul class="text-xs text-deep-200 space-y-1 list-disc pl-4">
        {#if $hostInfo.overcommit.memory_warning}<li>{$hostInfo.overcommit.memory_warning}</li>{/if}
        {#if $hostInfo.overcommit.cpu_warning}<li>{$hostInfo.overcommit.cpu_warning}</li>{/if}
      </ul>
      <p class="text-[10px] text-deep-400 mt-2">Warnings are advisory; per-instance limits stay enforced. Lower allocations or raise the host reserve (HOST_RESERVE_MB / config.toml).</p>
    </div>
  {/if}

  {#if $userPermissions.includes('SERVER_VIEW') && $hostInfo}
    <div class="card">
      <h2 class="card-header flex items-center gap-2"><ServerCog size={14} /> Host Capacity</h2>
      <div class="grid grid-cols-2 md:grid-cols-4 gap-4 text-xs">
        <div>
          <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Memory used</p>
          <p class="font-mono text-strong">{fmtMb($hostInfo.memory_used_mb)} / {fmtMb($hostInfo.memory_total_mb)}</p>
          <div class="h-1.5 bg-deep-700 mt-1.5">
            <div class="h-full {hostPercent > 90 ? 'bg-red-500' : hostPercent > 75 ? 'bg-amber-500' : 'bg-teal-500'}" style="width: {hostPercent}%"></div>
          </div>
        </div>
        <div>
          <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">CPU cores</p>
          <p class="font-mono text-strong">{$hostInfo.cpu_count} <span class="text-deep-400">(load {$hostInfo.load_1.toFixed(2)})</span></p>
        </div>
        {#if $hostInfo.disk_total_mb}
          <div>
            <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Disk used</p>
            <p class="font-mono text-strong">{fmtMb($hostInfo.disk_used_mb ?? 0)} / {fmtMb($hostInfo.disk_total_mb)}</p>
            <div class="h-1.5 bg-deep-700 mt-1.5">
              <div class="h-full {diskPercent > 90 ? 'bg-red-500' : diskPercent > 75 ? 'bg-amber-500' : 'bg-teal-500'}" style="width: {diskPercent}%"></div>
            </div>
          </div>
        {/if}
        <div>
          <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Allocated to instances</p>
          <p class="font-mono text-strong">{fmtMb($hostInfo.allocated.memory_mb)} · {$hostInfo.allocated.cpus.toFixed(1)} CPU · {$hostInfo.allocated.instances} instances</p>
          <p class="text-[10px] text-deep-400 mt-0.5">host reserve {fmtMb($hostInfo.reserve_mb)} (monitoring-only budget)</p>
        </div>
        <div>
          <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Enforcement backends</p>
          <div class="flex flex-wrap gap-1.5">
            <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('native')}">native</span>
            <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('systemd')} {$hostInfo.capabilities.systemd ? '' : 'opacity-40'}">
              systemd {#if !$hostInfo.capabilities.systemd}(unavailable){/if}
            </span>
            <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('docker')} {$hostInfo.capabilities.docker ? '' : 'opacity-40'}">
              docker {#if !$hostInfo.capabilities.docker}(unavailable){/if}
            </span>
          </div>
          {#if $hostInfo.capabilities.systemd && !$hostInfo.capabilities.linger}
            <p class="text-[10px] text-amber-400 mt-1">
              linger off — systemd units stop at logout; run <code>loginctl enable-linger $USER</code> for boot-persistent servers
            </p>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    {#each $instances as i (i.id)}
      <div class="card flex flex-col gap-3">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <span class="status-dot {i.status}"></span>
              <h3 class="text-sm font-bold text-strong truncate">{i.name}</h3>
              {#if $activeInstanceId === i.id}
                <span class="px-1.5 py-0.5 border border-bedrock-500/40 bg-bedrock-500/10 text-bedrock-300 text-[10px] uppercase tracking-wider">active</span>
              {/if}
            </div>
            <p class="text-[10px] uppercase tracking-wider text-deep-400 mt-1">
              {i.owner} · {i.backend} · port {i.port} · up {fmtUptime(i.uptime)}
            </p>
          </div>
          <div class="flex flex-col items-end gap-1 shrink-0">
            <span class="px-1.5 py-0.5 border text-[10px] uppercase tracking-wider {deploymentBadge(i.deployment)}" title={enforcementNote(i)}>
              {i.deployment}
            </span>
            {#if i.enforcement === 'advisory'}
              <span class="flex items-center gap-1 text-[10px] text-amber-400" title={enforcementNote(i)}>
                <ShieldAlert size={10} /> advisory limits
              </span>
            {:else}
              <span class="flex items-center gap-1 text-[10px] text-teal-400" title={enforcementNote(i)}>
                <ShieldCheck size={10} /> enforced
              </span>
            {/if}
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3 text-xs">
          <div>
            <div class="flex justify-between text-deep-400 mb-1">
              <span class="uppercase tracking-wider text-[10px] flex items-center gap-1"><MemoryStick size={10} /> Memory</span>
              <span class="font-mono">
                {#if i.usage}{fmtMb(i.usage.memory_mb)}{/if}
                {#if i.resources.memory_mb}/ {fmtMb(i.resources.memory_mb)}{:else}(no limit){/if}
              </span>
            </div>
            <div class="h-1.5 bg-deep-700">
              <div class="h-full bg-bedrock-500" style="width: {memBar(i)}%"></div>
            </div>
          </div>
          <div>
            <div class="flex justify-between text-deep-400 mb-1">
              <span class="uppercase tracking-wider text-[10px] flex items-center gap-1"><Cpu size={10} /> CPU</span>
              <span class="font-mono">
                {#if i.usage}{i.usage.cpu_percent.toFixed(1)}%{/if}
                {#if i.resources.cpus}/ {i.resources.cpus} CPU{:else}(no limit){/if}
              </span>
            </div>
            <div class="h-1.5 bg-deep-700">
              <div class="h-full bg-teal-500" style="width: {i.resources.cpus && i.usage ? Math.min(100, (i.usage.cpu_percent / (i.resources.cpus * 100)) * 100) : Math.min(100, i.usage?.cpu_percent ?? 0)}%"></div>
            </div>
          </div>
        </div>

        <div class="flex justify-between text-deep-400 text-[10px] uppercase tracking-wider">
          <span class="flex items-center gap-1"><HardDrive size={10} /> Disk (monitored, unenforced)</span>
          <span class="font-mono text-deep-300">{i.usage ? fmtMb(i.usage.disk_mb ?? 0) : '…'}</span>
        </div>
        {#if i.access.length > 0}
          <p class="text-[10px] text-deep-400 flex items-center gap-1">
            <Users2 size={10} />
            Shared with
            {#each i.access as a, ai}{#if ai > 0},{/if}
              {a.user} ({a.roles.map((r) => roleInfo(r)?.display_name ?? r).join('/')}){/each}
          </p>
        {/if}

        <div class="flex flex-wrap gap-2 pt-1 border-t border-deep-700/40">
          {#if $userPermissions.includes('SERVER_MANAGE') && canControl(i)}
            {#if i.status === 'running'}
              <button onclick={() => act(i, 'restart')} disabled={acting !== null}
                      class="btn-secondary text-xs flex items-center gap-1.5">
                <RotateCw size={12} /> {acting === `${i.id}:restart` ? 'Restarting…' : 'Restart'}
              </button>
              <button onclick={() => act(i, 'stop')} disabled={acting !== null}
                      class="btn-danger text-xs flex items-center gap-1.5">
                <Square size={12} /> Stop
              </button>
            {:else if i.status !== 'starting' && i.status !== 'stopping'}
              <button onclick={() => act(i, 'start')} disabled={acting !== null}
                      class="btn-success text-xs flex items-center gap-1.5">
                <Play size={12} /> {acting === `${i.id}:start` ? 'Starting…' : 'Start'}
              </button>
            {/if}
          {/if}
          <button onclick={() => open(i)} class="btn-secondary text-xs flex items-center gap-1.5">
            <Terminal size={12} /> Console
          </button>
          {#if canEdit(i)}
            <button onclick={() => openEdit(i)} class="btn-secondary text-xs flex items-center gap-1.5">
              <Settings2 size={12} /> Edit
            </button>
          {/if}
          {#if (isPanelOwner() || i.owner === $currentUser?.username) && i.id !== 'default'}
            <button onclick={() => { deleting = i; deleteKeepData = false; }} class="btn-danger text-xs flex items-center gap-1.5 ml-auto">
              <Trash2 size={12} /> Delete
            </button>
          {/if}
        </div>
      </div>
    {:else}
      <div class="card col-span-full text-center py-10 text-deep-400 text-xs">
        {#if loading}Loading…{:else}
          <Layers size={24} class="mx-auto mb-3 opacity-50" />
          No instances yet{#if $userPermissions.includes('INSTANCES_CREATE')} — create one to get started{/if}
        {/if}
      </div>
    {/each}
  </div>
</div>

<!-- Create modal -->
{#if showCreate}
  <div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) showCreate = false; }}
       onkeydown={(e) => { if (e.key === 'Escape') showCreate = false; }}>
    <div class="card max-w-md w-full mx-4">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-4">New Instance</h2>
      <div class="space-y-3">
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Name</label>
          <input bind:value={form.name} placeholder="Dad's Survival Server" class="input w-full text-xs mt-1" />
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">Runtime</label>
            <select bind:value={form.backend} class="input w-full text-xs mt-1">
              <option value="vanilla">Vanilla Bedrock</option>
              <option value="endstone">Endstone</option>
            </select>
          </div>
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">UDP port (blank = auto)</label>
            <input bind:value={form.port} type="number" placeholder="19132" class="input w-full text-xs mt-1" />
          </div>
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Deployment / limit enforcement</label>
          <select bind:value={form.deployment} class="input w-full text-xs mt-1">
            <option value="native">Native process — advisory limits only</option>
            {#if !$hostInfo?.capabilities.systemd}
              <option value="systemd" disabled>systemd service — unavailable on this host</option>
            {:else}
              <option value="systemd">systemd service — cgroup enforced (Linux)</option>
            {/if}
            {#if !$hostInfo?.capabilities.docker}
              <option value="docker" disabled>Docker container — unavailable on this host</option>
            {:else}
              <option value="docker">Docker container — runtime enforced</option>
            {/if}
          </select>
          <p class="text-[10px] text-deep-400 mt-1">
            {#if form.deployment === 'native'}Limits below are monitored but NOT enforced for native processes.{/if}
            {#if form.deployment === 'systemd'}Runs as a systemd user service (omnibedrock-&lt;id&gt;.service): MemoryMax/CPUQuota enforced, survives panel restarts.{/if}
            {#if form.deployment === 'docker'}--memory/--cpus are enforced by the container runtime; the container runs detached under Docker's unless-stopped restart policy, so it survives panel restarts and reboots (turn the instance's auto-restart off to keep it down).{/if}
          </p>
          {#if form.deployment === 'docker' && $hostInfo?.capabilities?.platform === 'macos'}
            <p class="text-[10px] text-amber-400 mt-1 flex items-start gap-1">
              <ShieldAlert size={10} class="shrink-0 mt-0.5" />
              Docker is the default here because there is no official macOS bedrock_server binary. Apple Silicon Macs run linux/amd64 images under emulation, which is slower.
            </p>
          {/if}
          {#if form.deployment === 'docker' && $hostInfo?.capabilities?.platform === 'windows'}
            <p class="text-[10px] text-amber-400 mt-1 flex items-start gap-1">
              <ShieldAlert size={10} class="shrink-0 mt-0.5" />
              Docker Desktop adds licensing and WSL2 overhead, and every UDP port must be mapped manually — native is recommended on Windows.
            </p>
          {/if}
          {#if form.deployment === 'docker'}
            <div class="mt-2 space-y-2 border border-deep-600/40 bg-deep-900/30 p-2">
              <div>
                <label class="text-[10px] uppercase tracking-wider text-deep-400">Image (blank = panel default for the runtime)</label>
                <input bind:value={form.dockerImage} placeholder="omnibedrock-vanilla:latest" class="input w-full text-xs mt-1" />
              </div>
              <div>
                <label class="text-[10px] uppercase tracking-wider text-deep-400">Extra bind mounts (one host:container[:opts] per line)</label>
                <textarea bind:value={form.dockerMounts} rows="2" placeholder="/mnt/media/worlds:/data/worlds" class="input w-full text-xs mt-1 font-mono"></textarea>
              </div>
              <p class="text-[10px] text-deep-400">
                {#if $hostInfo?.capabilities?.platform === 'linux'}
                  Runs with host networking: game ports and ICE ranges behave exactly like a native process.
                {:else}
                  The instance's UDP port pair is mapped through to the container's stock 19132/19133.
                {/if}
              </p>
            </div>
          {/if}
          {#if form.deployment === 'systemd'}
            <label class="flex items-center gap-2 text-xs text-deep-300 mt-2">
              <input type="checkbox" bind:checked={form.autostart} />
              Start at boot (enables the user unit)
            </label>
            {#if $hostInfo?.capabilities.systemd && !$hostInfo?.capabilities.linger}
              <p class="text-[10px] text-amber-400 mt-1">
                Linger is off — run <code>loginctl enable-linger $USER</code> so units start at boot without a login session.
              </p>
            {/if}
          {/if}
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">Memory limit (MB, blank = ∞)</label>
            <input bind:value={form.memory} type="number" placeholder="1024" class="input w-full text-xs mt-1" />
          </div>
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">CPU limit (cores, blank = ∞)</label>
            <input bind:value={form.cpus} type="number" step="0.1" placeholder="1.0" class="input w-full text-xs mt-1" />
          </div>
        </div>
        <label class="flex items-center gap-2 text-xs text-deep-300">
          <input type="checkbox" bind:checked={form.installAfter} />
          Install the runtime now and open its console
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => showCreate = false} class="btn-secondary text-xs">Cancel</button>
          <button onclick={create} disabled={creating} class="btn-primary text-xs">{creating ? 'Creating…' : 'Create'}</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Edit modal -->
{#if editing}
  <div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) editing = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') editing = null; }}>
    <div class="card max-w-md w-full mx-4 max-h-[85vh] overflow-y-auto">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-1">Edit {editing.name}</h2>
      {#if editing.status === 'running'}
        <p class="text-[10px] text-amber-400 mb-3 flex items-center gap-1"><ShieldAlert size={10} /> Stop the instance before changing deployment, resources, or port.</p>
      {:else}
        <p class="mb-3"></p>
      {/if}
      <div class="space-y-3">
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Name</label>
          <input bind:value={editForm.name} class="input w-full text-xs mt-1" />
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">Port</label>
            <input bind:value={editForm.port} type="number" class="input w-full text-xs mt-1" />
          </div>
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">Memory MB</label>
            <input bind:value={editForm.memory} type="number" placeholder="∞" class="input w-full text-xs mt-1" />
          </div>
          <div>
            <label class="text-[10px] uppercase tracking-wider text-deep-400">CPUs</label>
            <input bind:value={editForm.cpus} type="number" step="0.1" placeholder="∞" class="input w-full text-xs mt-1" />
          </div>
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Deployment</label>
          <select bind:value={editForm.deployment} class="input w-full text-xs mt-1">
            <option value="native">Native — advisory</option>
            {#if $hostInfo?.capabilities.systemd}<option value="systemd">systemd service — cgroup enforced</option>{/if}
            {#if $hostInfo?.capabilities.docker}<option value="docker">Docker — container enforced</option>{/if}
          </select>
          {#if editForm.deployment === 'docker'}
            <div class="mt-2 space-y-2 border border-deep-600/40 bg-deep-900/30 p-2">
              <div>
                <label class="text-[10px] uppercase tracking-wider text-deep-400">Image (blank = panel default for the runtime)</label>
                <input bind:value={editForm.dockerImage} placeholder="omnibedrock-vanilla:latest" class="input w-full text-xs mt-1" />
              </div>
              <div>
                <label class="text-[10px] uppercase tracking-wider text-deep-400">Extra bind mounts (one host:container[:opts] per line)</label>
                <textarea bind:value={editForm.dockerMounts} rows="2" placeholder="/mnt/media/worlds:/data/worlds" class="input w-full text-xs mt-1 font-mono"></textarea>
              </div>
            </div>
          {/if}
          {#if editForm.deployment === 'systemd'}
            <label class="flex items-center gap-2 text-xs text-deep-300 mt-2">
              <input type="checkbox" bind:checked={editForm.autostart} />
              Start at boot (enables the user unit)
            </label>
          {/if}
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Share with users — roles grant what they may do here</label>
          <div class="flex gap-2 mt-1">
            <input list="instance-users" bind:value={accessUser} placeholder="username" class="input flex-1 text-xs" />
            <datalist id="instance-users">
              {#each users as u}<option value={u} />{/each}
            </datalist>
            <button onclick={addAccess} disabled={accessRoles.size === 0} class="btn-secondary text-xs"><Plus size={12} /></button>
          </div>
          <div class="flex flex-wrap gap-2 mt-1.5">
            {#each roleCatalog as r (r.name)}
              <label class="flex items-center gap-1 text-[10px] uppercase tracking-wider cursor-pointer px-1.5 py-0.5 border {accessRoles.has(r.name) ? 'border-bedrock-500/60 bg-bedrock-500/10' : 'border-deep-600/40 bg-deep-900/40'}">
                <input type="checkbox" class="accent-bedrock-500 w-3 h-3"
                       checked={accessRoles.has(r.name)}
                       onchange={() => { const next = new Set(accessRoles); next.has(r.name) ? next.delete(r.name) : next.add(r.name); accessRoles = next; }} />
                <span class="w-2 h-2 inline-block" style="background: #{r.color || '99AAB5'}"></span>
                {r.display_name}
              </label>
            {/each}
          </div>
          {#if editForm.access.length > 0}
            <div class="mt-2 space-y-1.5">
              {#each editForm.access as a (a.user)}
                <div class="border border-deep-600/50 bg-deep-700/30 p-2">
                  <div class="flex items-center justify-between">
                    <span class="text-xs font-bold text-strong">{a.user}</span>
                    <button onclick={() => { editForm.access = editForm.access.filter((x) => x.user !== a.user); }}
                            class="text-red-400 hover:text-red-300 text-xs" aria-label="Remove {a.user}">remove</button>
                  </div>
                  <div class="flex flex-wrap gap-2 mt-1.5">
                    {#each roleCatalog as r (r.name)}
                      <label class="flex items-center gap-1 text-[10px] uppercase tracking-wider cursor-pointer">
                        <input type="checkbox" class="accent-bedrock-500 w-3 h-3"
                               checked={a.roles.includes(r.name)}
                               onchange={() => toggleEntryRole(a, r.name)} />
                        <span class="w-2 h-2 inline-block" style="background: #{r.color || '99AAB5'}"></span>
                        {r.display_name}
                      </label>
                    {/each}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => { editing = null; }} class="btn-secondary text-xs">Cancel</button>
          <button onclick={saveEdit} class="btn-primary text-xs">Save</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Delete modal -->
{#if deleting}
  <div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) deleting = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') deleting = null; }}>
    <div class="card max-w-sm w-full mx-4">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-2">Delete {deleting.name}?</h2>
      <p class="text-xs text-deep-300 mb-3">
        The instance is stopped and removed from the panel. This cannot be undone.
      </p>
      <label class="flex items-center gap-2 text-xs text-deep-300 mb-4">
        <input type="checkbox" bind:checked={deleteKeepData} />
        Keep files on disk (worlds, backups, logs stay under the data directory)
      </label>
      <div class="flex justify-end gap-2">
        <button onclick={() => { deleting = null; }} class="btn-secondary text-xs">Cancel</button>
        <button onclick={remove} class="btn-danger text-xs flex items-center gap-1.5"><Trash2 size={12} /> Delete</button>
      </div>
    </div>
  </div>
{/if}
