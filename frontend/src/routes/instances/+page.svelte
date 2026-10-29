<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, userPermissions } from '$stores/auth';
  import { instances, hostInfo, activeInstanceId, loadHost, refreshInstances, switchInstance } from '$stores/instances';
  import type { Instance, AccessEntry, RoleInfo } from '$types/index';
  import { Plus, RefreshCw, Layers } from '@lucide/svelte';
  import { goto } from '$app/navigation';
  import { parseMounts } from '$lib/instances/forms';
  import type { CreateInstanceForm, EditInstanceForm } from '$lib/instances/forms';
  import { roleInfo } from '$lib/instances/access';
  import HostCapacityCard from '$lib/components/instances/HostCapacityCard.svelte';
  import InstanceCard from '$lib/components/instances/InstanceCard.svelte';
  import CreateInstanceModal from '$lib/components/instances/CreateInstanceModal.svelte';
  import EditInstanceModal from '$lib/components/instances/EditInstanceModal.svelte';
  import DeleteInstanceModal from '$lib/components/instances/DeleteInstanceModal.svelte';

  let loading = $state(true);
  let acting = $state<string | null>(null);
  let showCreate = $state(false);
  let editing = $state<Instance | null>(null);
  let deleting = $state<Instance | null>(null);
  let deleteKeepData = $state(false);
  let poll: ReturnType<typeof setInterval> | undefined;

  let form = $state<CreateInstanceForm>({
    name: '',
    backend: 'vanilla',
    deployment: 'native',
    dockerImage: '',
    dockerMounts: '',
    memory: '',
    cpus: '',
    port: '',
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
  function emptyForm(): CreateInstanceForm {
    return {
      name: '',
      backend: 'vanilla',
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
  let editForm = $state<EditInstanceForm>({
    name: '',
    port: '',
    memory: '',
    cpus: '',
    deployment: 'native',
    dockerImage: '',
    dockerMounts: '',
    autostart: false,
    access: [],
  });
  let accessUser = $state('');
  let accessRoles = $state<Set<string>>(new Set(['viewer']));
  let users = $state<string[]>([]);
  let roleCatalog = $state<RoleInfo[]>([]);
  let creating = $state(false);

  /** Frontend mirror of the backend rule; the API stays the boundary. */
  function entryPerms(i: Instance): Set<string> {
    const perms = new Set<string>();
    const entry = i.access.find((a) => a.user === $currentUser?.username);
    if (entry) {
      for (const r of entry.roles) {
        for (const p of roleInfo(roleCatalog, r)?.permissions ?? []) perms.add(p);
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
    <HostCapacityCard hostInfo={$hostInfo} memoryPercent={hostPercent} diskPercent={diskPercent} />
  {/if}

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    {#each $instances as i (i.id)}
      <InstanceCard instance={i}
                    active={$activeInstanceId === i.id}
                    acting={acting}
                    canAct={$userPermissions.includes('SERVER_MANAGE') && canControl(i)}
                    canEdit={canEdit(i)}
                    canDelete={(isPanelOwner() || i.owner === $currentUser?.username) && i.id !== 'default'}
                    roleCatalog={roleCatalog}
                    onact={(action) => act(i, action)}
                    onopen={() => open(i)}
                    onedit={() => openEdit(i)}
                    ondelete={() => { deleting = i; deleteKeepData = false; }} />
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
  <CreateInstanceModal bind:form hostInfo={$hostInfo} creating={creating}
                       oncreate={create} onclose={() => (showCreate = false)} />
{/if}

<!-- Edit modal -->
{#if editing}
  <EditInstanceModal instance={editing} bind:editForm hostInfo={$hostInfo}
                     users={users} roleCatalog={roleCatalog}
                     bind:accessUser bind:accessRoles
                     onaddaccess={addAccess} ontoggleentryrole={toggleEntryRole}
                     onsave={saveEdit} onclose={() => (editing = null)} />
{/if}

<!-- Delete modal -->
{#if deleting}
  <DeleteInstanceModal instance={deleting} bind:keepData={deleteKeepData}
                       onremove={remove} onclose={() => (deleting = null)} />
{/if}
