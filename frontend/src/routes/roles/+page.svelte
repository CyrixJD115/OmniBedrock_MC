<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, userPermissions } from '$stores/auth';
  import { Shield, ShieldOff, Plus, Save, X, Check } from '@lucide/svelte';

  let roles = $state<{ name: string; display_name: string; permissions: string[]; is_default: boolean; created_at: string }[]>([]);
  let loading = $state(true);
  let editing = $state<{ name: string; display_name: string; permissions: Set<string>; is_default: boolean } | null>(null);
  let creating = $state(false);
  let newRole = $state({ name: '', display_name: '', permissions: new Set<string>(), is_default: false });
  let deleting = $state<string | null>(null);

  const PERM_GROUPS: { label: string; perms: string[] }[] = [
    { label: 'Console', perms: ['CONSOLE_VIEW', 'CONSOLE_SEND'] },
    { label: 'Server', perms: ['SERVER_VIEW', 'SERVER_MANAGE'] },
    { label: 'Players', perms: ['PLAYERS_VIEW', 'PLAYERS_KICK', 'PLAYERS_BAN', 'PLAYERS_OP'] },
    { label: 'Properties', perms: ['PROPERTIES_VIEW', 'PROPERTIES_EDIT'] },
    { label: 'Addons', perms: ['ADDONS_VIEW', 'ADDONS_MANAGE'] },
    { label: 'Backups', perms: ['BACKUPS_VIEW', 'BACKUPS_CREATE', 'BACKUPS_RESTORE', 'BACKUPS_DELETE'] },
    { label: 'Users', perms: ['USERS_VIEW', 'USERS_CREATE', 'USERS_EDIT', 'USERS_DELETE'] },
    { label: 'Files', perms: ['FILES_VIEW', 'FILES_EDIT'] },
    { label: 'Audit', perms: ['AUDIT_VIEW'] },
    { label: 'Settings', perms: ['SETTINGS_VIEW', 'SETTINGS_EDIT'] },
  ];

  const permLabel: Record<string, string> = {
    CONSOLE_VIEW: 'View console', CONSOLE_SEND: 'Send commands',
    SERVER_VIEW: 'View server', SERVER_MANAGE: 'Manage server',
    PLAYERS_VIEW: 'View players', PLAYERS_KICK: 'Kick players', PLAYERS_BAN: 'Ban players', PLAYERS_OP: 'OP players',
    PROPERTIES_VIEW: 'View properties', PROPERTIES_EDIT: 'Edit properties',
    ADDONS_VIEW: 'View addons', ADDONS_MANAGE: 'Manage addons',
    BACKUPS_VIEW: 'View backups', BACKUPS_CREATE: 'Create backups', BACKUPS_RESTORE: 'Restore backups', BACKUPS_DELETE: 'Delete backups',
    USERS_VIEW: 'View users', USERS_CREATE: 'Create users', USERS_EDIT: 'Edit users', USERS_DELETE: 'Delete users',
    FILES_VIEW: 'View files', FILES_EDIT: 'Edit files',
    AUDIT_VIEW: 'View audit log',
    SETTINGS_VIEW: 'View settings', SETTINGS_EDIT: 'Edit settings',
  };

  onMount(loadRoles);

  async function loadRoles() {
    loading = true;
    try { roles = (await api.listRoles()) as any; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  }

  function canManage() { return $userPermissions.includes('USERS_CREATE'); }

  function openCreate() {
    newRole = { name: '', display_name: '', permissions: new Set<string>(), is_default: false };
    creating = true;
  }

  function openEdit(r: typeof roles[0]) {
    editing = { name: r.name, display_name: r.display_name, permissions: new Set(r.permissions), is_default: r.is_default };
  }

  function togglePerm(set: Set<string>, perm: string) {
    if (set.has(perm)) set.delete(perm); else set.add(perm);
  }

  async function createRole() {
    if (!newRole.name) { addToast('Name is required', 'error'); return; }
    try {
      await api.createRole({ name: newRole.name, display_name: newRole.display_name, permissions: [...newRole.permissions], is_default: newRole.is_default });
      addToast(`Created role ${newRole.name}`, 'success');
      creating = false;
      await loadRoles();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function saveRole() {
    if (!editing) return;
    try {
      await api.updateRole(editing.name, { display_name: editing.display_name, permissions: [...editing.permissions], is_default: editing.is_default });
      addToast(`Updated ${editing.name}`, 'success');
      editing = null;
      await loadRoles();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function deleteRole(name: string) {
    try {
      await api.deleteRole(name);
      addToast(`Deleted ${name}`, 'success');
      deleting = null;
      await loadRoles();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Roles</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadRoles} class="btn-secondary text-xs">Refresh</button>
      {#if canManage()}
        <button onclick={openCreate} class="btn-primary text-xs flex items-center gap-1.5">
          <Plus size={12} /> New Role
        </button>
      {/if}
    </div>
  </div>

  <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
    {#each roles as r}
      <div class="card relative">
        <div class="flex items-start justify-between mb-4">
          <div>
            <h3 class="text-sm font-bold text-white uppercase tracking-wider">{r.display_name || r.name}</h3>
            <p class="text-[11px] text-deep-400 uppercase tracking-wider mt-1">{r.name}{r.is_default ? ' — Default' : ''}</p>
          </div>
          {#if canManage() && r.name !== 'owner'}
            <div class="flex gap-1 shrink-0">
              <button onclick={() => openEdit(r)} class="btn-ghost p-1.5 text-bedrock-400 hover:text-bedrock-300" title="Edit"><Save size={14} /></button>
              <button onclick={() => deleting = r.name} class="btn-ghost p-1.5 text-red-400 hover:text-red-300" title="Delete"><ShieldOff size={14} /></button>
            </div>
          {/if}
        </div>
        <div class="space-y-2.5">
          {#each PERM_GROUPS as group}
            {#if group.perms.some(p => r.permissions.includes(p))}
              <div>
                <p class="text-[10px] text-deep-500 uppercase tracking-wider mb-1.5">{group.label}</p>
                <div class="flex flex-wrap gap-1.5">
                  {#each group.perms as perm}
                    {#if r.permissions.includes(perm)}
                      <span class="text-[11px] px-2.5 py-1 bg-bedrock-500/15 text-bedrock-300 rounded font-medium">{permLabel[perm] || perm}</span>
                    {/if}
                  {/each}
                </div>
              </div>
            {/if}
          {/each}
        </div>
      </div>
    {:else}
      <div class="col-span-full text-center py-8 text-deep-500">{loading ? 'Loading...' : 'No roles defined'}</div>
    {/each}
  </div>
</div>

<!-- Create Modal -->
{#if creating}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) creating = false; }}
       onkeydown={(e) => { if (e.key === 'Escape') creating = false; }}>
    <div class="card max-w-2xl w-full mx-4 max-h-[80vh] overflow-y-auto">
      <div class="flex items-center justify-between mb-4">
        <h2 class="text-sm font-bold text-white uppercase tracking-wider">Create Role</h2>
        <button onclick={() => creating = false} class="btn-ghost p-1 text-deep-400"><X size={14} /></button>
      </div>
      <div class="space-y-3 mb-4">
        <input bind:value={newRole.name} placeholder="Role name" class="input w-full text-xs" />
        <input bind:value={newRole.display_name} placeholder="Display name (optional)" class="input w-full text-xs" />
        <label class="flex items-center gap-2 text-xs text-deep-300">
          <input type="checkbox" bind:checked={newRole.is_default} class="accent-bedrock-500" />
          Default role for new users
        </label>
      </div>
      <h3 class="text-xs font-bold text-white uppercase tracking-wider mb-2">Permissions</h3>
      <div class="space-y-4">
        {#each PERM_GROUPS as group}
          <div>
            <p class="text-[11px] text-deep-400 font-semibold uppercase tracking-wider mb-2">{group.label}</p>
            <div class="flex flex-wrap gap-2">
              {#each group.perms as perm}
                {@const active = newRole.permissions.has(perm)}
                <button onclick={() => togglePerm(newRole.permissions, perm)}
                        class="text-xs px-3 py-1.5 rounded border transition-all duration-150 {active ? 'bg-bedrock-500/40 border-bedrock-400 text-white shadow-sm shadow-bedrock-500/20 ring-1 ring-bedrock-500/40' : 'bg-deep-800/80 border-deep-600 text-deep-400 hover:border-deep-400 hover:text-deep-200'}">
                  {#if active}<Check size={10} class="inline mr-1 -mt-0.5" />{/if}{permLabel[perm] || perm}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
      <div class="flex justify-end gap-2 pt-4">
        <button onclick={() => creating = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={createRole} class="btn-primary text-xs">Create</button>
      </div>
    </div>
  </div>
{/if}

<!-- Edit Modal -->
{#if editing}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) editing = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') editing = null; }}>
    <div class="card max-w-2xl w-full mx-4 max-h-[80vh] overflow-y-auto">
      <div class="flex items-center justify-between mb-4">
        <h2 class="text-sm font-bold text-white uppercase tracking-wider">Edit {editing.name}</h2>
        <button onclick={() => editing = null} class="btn-ghost p-1 text-deep-400"><X size={14} /></button>
      </div>
      <div class="space-y-3 mb-4">
        <input bind:value={editing.display_name} placeholder="Display name" class="input w-full text-xs" />
        <label class="flex items-center gap-2 text-xs text-deep-300">
          <input type="checkbox" bind:checked={editing.is_default} class="accent-bedrock-500" />
          Default role for new users
        </label>
      </div>
      <h3 class="text-xs font-bold text-white uppercase tracking-wider mb-2">Permissions</h3>
      <div class="space-y-4">
        {#each PERM_GROUPS as group}
          <div>
            <p class="text-[11px] text-deep-400 font-semibold uppercase tracking-wider mb-2">{group.label}</p>
            <div class="flex flex-wrap gap-2">
              {#each group.perms as perm}
                {@const active = editing!.permissions.has(perm)}
                <button onclick={() => togglePerm(editing!.permissions, perm)}
                        class="text-xs px-3 py-1.5 rounded border transition-all duration-150 {active ? 'bg-bedrock-500/40 border-bedrock-400 text-white shadow-sm shadow-bedrock-500/20 ring-1 ring-bedrock-500/40' : 'bg-deep-800/80 border-deep-600 text-deep-400 hover:border-deep-400 hover:text-deep-200'}">
                  {#if active}<Check size={10} class="inline mr-1 -mt-0.5" />{/if}{permLabel[perm] || perm}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
      <div class="flex justify-end gap-2 pt-4">
        <button onclick={() => editing = null} class="btn-secondary text-xs">Cancel</button>
        <button onclick={saveRole} class="btn-primary text-xs">Save</button>
      </div>
    </div>
  </div>
{/if}

<!-- Delete Confirmation -->
{#if deleting}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) deleting = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') deleting = null; }}>
    <div class="card max-w-sm w-full mx-4">
      <h2 class="text-sm font-bold text-white uppercase tracking-wider mb-2">Delete Role</h2>
      <p class="text-xs text-deep-300 mb-4">Are you sure you want to delete <strong>{deleting}</strong>?</p>
      <div class="flex justify-end gap-2">
        <button onclick={() => deleting = null} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => deleteRole(deleting!)} class="btn-danger text-xs">Delete</button>
      </div>
    </div>
  </div>
{/if}
