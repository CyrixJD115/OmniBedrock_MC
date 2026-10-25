<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import {
    PERM_GROUPS, hexColor, COLOR_PRESETS, isInstancePermission, allKnownPermissions,
  } from '$lib/permissions';
  import type { RoleInfo } from '$types/index';
  import {
    Plus, Pencil, Copy, Trash2, ChevronUp, ChevronDown, Save, Star, Users2, Check, RefreshCw, Shield
  } from '@lucide/svelte';

  let roles = $state<RoleInfo[]>([]);
  let loading = $state(true);
  let editing = $state<null | {
    name: string;               // '' while creating
    display_name: string;
    description: string;
    color: string;
    permissions: Set<string>;
    position: number;
    is_default: boolean;
    duplicate_of: string;
  }>(null);
  let deleting = $state<RoleInfo | null>(null);
  let busy = $state(false);

  const canManage = $derived($userPermissions.includes('ROLES_MANAGE'));
  const totalPerms = allKnownPermissions().length;
  const instancePermCount = $derived(
    editing ? [...editing.permissions].filter(isInstancePermission).length : 0
  );
  const sorted = $derived(
    [...roles].sort((a, b) => b.position - a.position || a.name.localeCompare(b.name))
  );

  onMount(loadRoles);

  async function loadRoles() {
    loading = true;
    try { roles = (await api.listRoles()) as never; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  }

  function openCreate() {
    const lowest = sorted.length ? sorted[sorted.length - 1].position : 0;
    editing = {
      name: '', display_name: '', description: '', color: '5865F2',
      permissions: new Set(), position: lowest - 1, is_default: false,
      duplicate_of: '',
    };
  }

  function openEdit(r: RoleInfo) {
    editing = {
      name: r.name,
      display_name: r.display_name,
      description: r.description ?? '',
      color: r.color || '99AAB5',
      permissions: new Set(r.permissions),
      position: r.position,
      is_default: r.is_default,
      duplicate_of: '',
    };
  }

  function openDuplicate(r: RoleInfo) {
    editing = {
      name: '', display_name: `${r.display_name} copy`,
      description: r.description ?? '', color: r.color || '99AAB5',
      permissions: new Set(r.permissions), position: r.position,
      is_default: false, duplicate_of: r.name,
    };
  }

  function toggle(perm: string) {
    if (!editing) return;
    const next = new Set(editing.permissions);
    if (next.has(perm)) next.delete(perm); else next.add(perm);
    editing.permissions = next;
  }

  function setGroup(perms: string[], on: boolean) {
    if (!editing) return;
    const next = new Set(editing.permissions);
    for (const p of perms) (on ? next.add(p) : next.delete(p));
    editing.permissions = next;
  }

  async function uniqueCopyName(source: string): Promise<string> {
    const taken = new Set(roles.map((r) => r.name));
    let candidate = `${source}-copy`;
    for (let i = 2; taken.has(candidate); i++) candidate = `${source}-copy-${i}`;
    return candidate;
  }

  async function save() {
    if (!editing || busy) return;
    if (!editing.name && !editing.duplicate_of) {
      addToast('Give the role a key name (a-z, 0-9, dashes)', 'error');
      return;
    }
    busy = true;
    const body: Record<string, unknown> = {
      display_name: editing.display_name || editing.name,
      description: editing.description,
      color: editing.color,
      permissions: [...editing.permissions],
      is_default: editing.is_default,
    };
    try {
      if (editing.name) {
        await api.updateRole(editing.name, body);
      } else {
        const name = editing.duplicate_of
          ? await uniqueCopyName(editing.duplicate_of)
          : editing.name.trim().toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '');
        if (!name) throw new Error('Key name required');
        await api.createRole({
          name,
          display_name: editing.display_name || name,
          description: editing.description,
          color: editing.color,
          permissions: [...editing.permissions],
          is_default: editing.is_default,
          duplicate_of: editing.duplicate_of || undefined,
        });
      }
      addToast('Role saved', 'success');
      editing = null;
      await loadRoles();
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
    busy = false;
  }

  async function move(r: RoleInfo, dir: -1 | 1) {
    const idx = sorted.findIndex((x) => x.name === r.name);
    const swap = idx + dir;
    if (swap < 0 || swap >= sorted.length) return;
    const a = sorted[idx], b = sorted[swap];
    try {
      await api.updateRole(a.name, { position: b.position });
      await api.updateRole(b.name, { position: a.position });
      await loadRoles();
    } catch (e: any) { addToast(`Reorder failed: ${e.message}`, 'error'); }
  }

  async function remove() {
    if (!deleting || busy) return;
    busy = true;
    try {
      await api.deleteRole(deleting.name);
      addToast(`Deleted ${deleting.display_name}`, 'success');
      deleting = null;
      await loadRoles();
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
    busy = false;
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Roles</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadRoles} class="btn-secondary text-xs" title="Refresh"><RefreshCw size={12} /></button>
      {#if canManage}
        <button onclick={openCreate} class="btn-primary text-xs flex items-center gap-1.5">
          <Plus size={12} /> New Role
        </button>
      {/if}
    </div>
  </div>

  <p class="text-[10px] text-deep-400 uppercase tracking-wider">
    One role catalog for everything: panel permissions are global; shares on the Instances page
    grant these same roles on one instance. Permissions combine across roles — a user holds the
    union of every role they have.
  </p>

  {#if loading}
    <div class="card py-6 text-center text-xs text-deep-400 uppercase tracking-wider">Loading roles…</div>
  {:else}
    <div class="space-y-1.5">
      {#each sorted as r (r.name)}
        <div class="card !p-3 flex items-center gap-3">
          <span class="w-3.5 h-3.5 shrink-0 border border-black/30 shadow-block" style="background:{hexColor(r.color)}"></span>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="text-sm font-bold uppercase tracking-wider" style="color:{hexColor(r.color)}">{r.display_name}</span>
              <span class="text-[10px] text-deep-400 font-mono">@{r.name}</span>
              {#if r.grants_owner}<span class="badge-warn !py-0 !px-1.5 !text-[9px]">owner</span>{/if}
              {#if r.builtin}<span class="badge-info !py-0 !px-1.5 !text-[9px]">built-in</span>{/if}
              {#if r.is_default}<span class="badge-running !py-0 !px-1.5 !text-[9px]"><Star size={9} class="inline -mt-0.5" /> default</span>{/if}
            </div>
            {#if r.description}
              <p class="text-[11px] text-deep-400 mt-0.5 truncate">{r.description}</p>
            {/if}
          </div>
          <div class="hidden sm:flex items-center gap-1.5 text-[10px] text-deep-400 uppercase tracking-wider shrink-0">
            <Users2 size={11} /> {r.user_count ?? 0}
            <span class="text-deep-500">·</span>
            {r.permissions.length}/{totalPerms} perms
          </div>
          {#if canManage}
            <div class="flex items-center gap-0.5 shrink-0">
              <button onclick={() => move(r, 1)} disabled={sorted.indexOf(r) === 0} class="btn-ghost !p-1.5" title="Move up"><ChevronUp size={13} /></button>
              <button onclick={() => move(r, -1)} disabled={sorted.indexOf(r) === sorted.length - 1} class="btn-ghost !p-1.5" title="Move down"><ChevronDown size={13} /></button>
              <button onclick={() => openEdit(r)} class="btn-ghost !p-1.5 text-bedrock-400" title="Edit"><Pencil size={12} /></button>
              <button onclick={() => openDuplicate(r)} class="btn-ghost !p-1.5" title="Duplicate"><Copy size={12} /></button>
              {#if !r.builtin}
                <button onclick={() => deleting = r} class="btn-ghost !p-1.5 text-red-400" title="Delete"><Trash2 size={12} /></button>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if editing}
  <div class="fixed inset-0 flex items-center justify-center z-50 modal-backdrop" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) editing = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') editing = null; }}>
    <div class="card max-w-2xl w-full mx-4 max-h-[90vh] overflow-y-auto">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-1">
        {editing.name ? `Edit @${editing.name}` : editing.duplicate_of ? `Duplicate @${editing.duplicate_of}` : 'New Role'}
      </h2>
      <p class="text-[10px] text-deep-400 mb-4">
        {instancePermCount} of {editing.permissions.size} selected permissions apply per-instance
        when this role is used in an instance share.
      </p>
      <div class="space-y-3">
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
          {#if !editing.name}
            <label class="block">
              <span class="text-[10px] uppercase tracking-wider text-deep-400">Key name</span>
              <input bind:value={editing.name} placeholder="my-role" class="input w-full text-xs mt-1 font-mono"
                     oninput={(e) => { const t = e.currentTarget; t.value = t.value.toLowerCase().replace(/[^a-z0-9-]+/g, '-'); }} />
            </label>
          {:else}
            <label class="block">
              <span class="text-[10px] uppercase tracking-wider text-deep-400">Key</span>
              <input value={editing.name} disabled class="input w-full text-xs mt-1 font-mono opacity-60" />
            </label>
          {/if}
          <label class="block">
            <span class="text-[10px] uppercase tracking-wider text-deep-400">Display name</span>
            <input bind:value={editing.display_name} placeholder="My Role" class="input w-full text-xs mt-1" />
          </label>
        </div>
        <label class="block">
          <span class="text-[10px] uppercase tracking-wider text-deep-400">Description</span>
          <input bind:value={editing.description} placeholder="What is this role for?" class="input w-full text-xs mt-1" />
        </label>
        <div>
          <span class="text-[10px] uppercase tracking-wider text-deep-400">Color</span>
          <div class="flex items-center gap-1.5 mt-1 flex-wrap">
            {#each COLOR_PRESETS as c (c)}
              <button onclick={() => editing!.color = c} aria-label="Color {c}"
                      class="w-6 h-6 border-2 {editing.color === c ? 'border-strong' : 'border-black/30'}"
                      style="background:#{c}"></button>
            {/each}
            <input value={editing.color} maxlength="6" class="input w-24 text-xs font-mono py-1"
                   oninput={(e) => { editing!.color = e.currentTarget.value.toUpperCase().replace(/[^0-9A-F]/g, ''); }} />
            <span class="w-5 h-5 border border-black/30" style="background:{hexColor(editing.color)}"></span>
          </div>
        </div>
        <div class="flex items-center justify-between border border-deep-600/40 bg-deep-900/40 px-3 py-2">
          <span class="text-xs text-deep-300">Default role for new users</span>
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="checkbox" class="accent-bedrock-500 w-4 h-4" checked={editing.is_default}
                   onchange={(e) => editing!.is_default = e.currentTarget.checked} />
            <span class="text-xs">{editing.is_default ? 'Default' : 'Not default'}</span>
          </label>
        </div>
        <div>
          <div class="flex items-center justify-between mb-1.5">
            <span class="text-[10px] uppercase tracking-wider text-deep-400">Permissions</span>
            <span class="text-[10px] font-mono text-deep-300">{editing.permissions.size}/{totalPerms} · {instancePermCount} per-instance</span>
          </div>
          <div class="space-y-2">
            {#each PERM_GROUPS as group (group.label)}
              <div class="border border-deep-600/40 bg-deep-900/40">
                <div class="flex items-center justify-between px-3 py-1.5 border-b border-deep-600/30">
                  <div>
                    <span class="text-xs font-bold uppercase tracking-wider text-strong">{group.label}</span>
                    <span class="text-[10px] text-deep-400 ml-2">{group.hint}</span>
                  </div>
                  <button onclick={() => {
                      const perms = group.perms.map((p) => p.perm);
                      const allOn = perms.every((p) => editing!.permissions.has(p));
                      setGroup(perms, !allOn);
                    }}
                    class="text-[10px] uppercase tracking-wider text-bedrock-300 hover:text-bedrock-200">
                    toggle all
                  </button>
                </div>
                <div class="p-2 grid grid-cols-1 sm:grid-cols-2 gap-1">
                  {#each group.perms as pm (pm.perm)}
                    <button onclick={() => toggle(pm.perm)} role="checkbox" aria-checked={editing.permissions.has(pm.perm)}
                            class={"flex items-start gap-2 text-left px-2 py-1.5 transition-colors " + (editing.permissions.has(pm.perm)
                              ? 'bg-bedrock-500/10 border border-bedrock-500/40'
                              : 'border border-transparent hover:bg-deep-700/40')}>
                      <span class="mt-0.5 w-3.5 h-3.5 shrink-0 border flex items-center justify-center {editing.permissions.has(pm.perm) ? 'bg-bedrock-500 border-bedrock-400' : 'border-deep-500'}">
                        {#if editing.permissions.has(pm.perm)}<Check size={11} class="text-white" />{/if}
                      </span>
                      <span class="min-w-0">
                        <span class="text-xs text-strong block leading-tight">{pm.label}</span>
                        <span class="text-[10px] text-deep-400 block leading-tight">{pm.hint}</span>
                      </span>
                    </button>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
          <p class="text-[10px] text-deep-400 mt-2">
            Permissions are additive: members hold the union of all their roles. Security rules
            (quotas, owner-only actions) sit outside this list and cannot be bypassed by combinations.
          </p>
        </div>
        <div class="flex justify-end gap-2 pt-1">
          <button onclick={() => editing = null} class="btn-secondary text-xs">Cancel</button>
          <button onclick={save} disabled={busy} class="btn-primary text-xs flex items-center gap-1.5">
            <Save size={12} /> {busy ? 'Saving…' : 'Save Role'}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if deleting}
  <div class="fixed inset-0 flex items-center justify-center z-50 modal-backdrop" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) deleting = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') deleting = null; }}>
    <div class="card max-w-sm w-full mx-4">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-2 flex items-center gap-2"><Shield size={14} /> Delete Role</h2>
      <p class="text-xs text-deep-300 mb-4">
        Delete <strong>@{deleting.name}</strong>? Users holding it keep every other role; instance
        shares lose this role's permissions.
      </p>
      <div class="flex justify-end gap-2">
        <button onclick={() => deleting = null} class="btn-secondary text-xs">Cancel</button>
        <button onclick={remove} disabled={busy} class="btn-danger text-xs">{busy ? 'Deleting…' : 'Delete'}</button>
      </div>
    </div>
  </div>
{/if}
