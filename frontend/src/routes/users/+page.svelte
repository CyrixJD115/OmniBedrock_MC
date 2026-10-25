<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { hexColor } from '$lib/permissions';
  import type { UserInfo, RoleInfo } from '$types/index';
  import { ShieldOff, UserPlus, UserCog, RefreshCw } from '@lucide/svelte';

  let users = $state<UserInfo[]>([]);
  let roles = $state<RoleInfo[]>([]);
  let loading = $state(true);
  let showCreateModal = $state(false);
  let showEditModal = $state(false);
  let editingUser = $state<UserInfo | null>(null);
  let form = $state({ username: '', password: '', display_name: '', roles: new Set<string>() });
  let editForm = $state({ display_name: '', roles: new Set<string>(), password: '' });
  let deleting = $state<string | null>(null);
  let busy = $state(false);

  const canEdit = $derived($userPermissions.includes('USERS_EDIT'));
  const canDelete = $derived($userPermissions.includes('USERS_DELETE'));

  onMount(() => { void loadAll(); });

  async function loadAll() {
    loading = true;
    try { users = (await api.listUsers()) as never; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    try { roles = (await api.listRoles()) as never; } catch { /* chips degrade */ }
    loading = false;
  }

  function roleInfo(name: string): RoleInfo | undefined {
    return roles.find((r) => r.name === name);
  }
  function roleChips(user: UserInfo): string[] {
    return user.roles?.length ? user.roles : [user.role];
  }
  function defaultRoleName(): string {
    return roles.find((r) => r.is_default)?.name ?? 'viewer';
  }

  function openCreate() {
    form = { username: '', password: '', display_name: '', roles: new Set([defaultRoleName()]) };
    showCreateModal = true;
  }

  function openEdit(u: UserInfo) {
    editingUser = u;
    editForm = {
      display_name: u.display_name,
      roles: new Set(u.roles?.length ? u.roles : [u.role]),
      password: '',
    };
    showEditModal = true;
  }

  function toggleRole(set: Set<string>, name: string): Set<string> {
    const next = new Set(set);
    if (next.has(name)) next.delete(name); else next.add(name);
    return next;
  }

  async function createUser() {
    if (!form.username || !form.password) { addToast('Username and password required', 'error'); return; }
    if (form.roles.size === 0) { addToast('Pick at least one role', 'error'); return; }
    busy = true;
    try {
      await api.createUser({
        username: form.username, password: form.password,
        display_name: form.display_name || form.username,
        roles: [...form.roles],
      });
      addToast(`Created ${form.username}`, 'success');
      showCreateModal = false;
      await loadAll();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    busy = false;
  }

  async function updateUser() {
    if (!editingUser) return;
    if (editForm.roles.size === 0) { addToast('Pick at least one role', 'error'); return; }
    busy = true;
    const data: Record<string, unknown> = {
      display_name: editForm.display_name,
      roles: [...editForm.roles],
    };
    if (editForm.password) data.password = editForm.password;
    try {
      await api.updateUser(editingUser.username, data);
      addToast(`Updated ${editingUser.username}`, 'success');
      showEditModal = false;
      editingUser = null;
      await loadAll();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    busy = false;
  }

  async function deleteUser(username: string) {
    busy = true;
    try {
      await api.deleteUser(username);
      addToast(`Deleted ${username}`, 'success');
      deleting = null;
      await loadAll();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    busy = false;
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Users</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadAll} class="btn-secondary text-xs" title="Refresh"><RefreshCw size={12} /></button>
      {#if $userPermissions.includes('USERS_CREATE')}
        <button onclick={openCreate} class="btn-primary text-xs flex items-center gap-1.5">
          <UserPlus size={12} /> New User
        </button>
      {/if}
    </div>
  </div>

  <div class="card">
    <div class="overflow-x-auto">
      <table class="w-full text-xs">
        <thead>
          <tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
            <th class="text-left py-2 px-3 font-medium">Username</th>
            <th class="text-left py-2 px-3 font-medium">Display Name</th>
            <th class="text-left py-2 px-3 font-medium">Roles</th>
            <th class="text-left py-2 px-3 font-medium">Created</th>
            {#if canEdit || canDelete}
              <th class="text-right py-2 px-3 font-medium">Actions</th>
            {/if}
          </tr>
        </thead>
        <tbody>
          {#each users as u (u.username)}
            <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
              <td class="py-1.5 px-3 font-medium text-strong">{u.username}</td>
              <td class="py-1.5 px-3">{u.display_name}</td>
              <td class="py-1.5 px-3">
                <div class="flex flex-wrap gap-1">
                  {#each roleChips(u) as name (name)}
                    {@const r = roleInfo(name)}
                    <span class="px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider border"
                          style="color:{hexColor(r?.color ?? '')}; border-color:{hexColor(r?.color ?? '')}66; background:{hexColor(r?.color ?? '')}1a">
                      {r?.display_name ?? name}
                    </span>
                  {/each}
                </div>
              </td>
              <td class="py-1.5 px-3 text-deep-400">{new Date(u.created_at).toLocaleDateString()}</td>
              {#if canEdit || canDelete}
                <td class="py-1.5 px-3 text-right">
                  {#if canEdit}
                    <button onclick={() => openEdit(u)} class="btn-ghost p-1 text-bedrock-400" title="Edit"><UserCog size={12} /></button>
                  {/if}
                  {#if canDelete}
                    <button onclick={() => deleting = u.username} class="btn-ghost p-1 text-red-400" title="Delete"><ShieldOff size={12} /></button>
                  {/if}
                </td>
              {/if}
            </tr>
          {:else}
            <tr><td colspan="{canEdit || canDelete ? 5 : 4}" class="text-center py-8 text-deep-400">{loading ? 'Loading…' : 'No users'}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>

  <p class="text-[10px] text-deep-400">
    Users hold any number of roles and get the union of their permissions. Instance shares
    (Instances → Edit) grant the same roles on a single server. Only owners may edit owner
    accounts; editors cannot grant permissions they do not hold themselves.
  </p>
</div>

<!-- Create Modal -->
{#if showCreateModal}
  <div class="fixed inset-0 flex items-center justify-center z-50 modal-backdrop" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) showCreateModal = false; }}
       onkeydown={(e) => { if (e.key === 'Escape') showCreateModal = false; }}>
    <div class="card max-w-md w-full mx-4 max-h-[90vh] overflow-y-auto">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-4">Create User</h2>
      <div class="space-y-3">
        <input bind:value={form.username} placeholder="Username" class="input w-full text-xs" />
        <input bind:value={form.password} type="password" placeholder="Password" class="input w-full text-xs" />
        <input bind:value={form.display_name} placeholder="Display Name" class="input w-full text-xs" />
        <div>
          <p class="text-[10px] uppercase tracking-wider text-deep-400 mb-1.5">Roles (any number)</p>
          <div class="flex flex-wrap gap-1.5">
            {#each roles as r (r.name)}
              <label class="flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider px-2 py-1 border cursor-pointer transition-colors"
                     style={form.roles.has(r.name)
                       ? `color:${hexColor(r.color)}; border-color:${hexColor(r.color)}88; background:${hexColor(r.color)}1a`
                       : 'color:var(--deep-300); border-color:rgb(var(--deep-600) / 0.5)'}>
                <input type="checkbox" class="accent-bedrock-500 w-3 h-3" checked={form.roles.has(r.name)}
                       onchange={() => form.roles = toggleRole(form.roles, r.name)} />
                {r.display_name}
              </label>
            {/each}
          </div>
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => showCreateModal = false} class="btn-secondary text-xs">Cancel</button>
          <button onclick={createUser} disabled={busy} class="btn-primary text-xs">{busy ? 'Creating…' : 'Create'}</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Edit Modal -->
{#if showEditModal && editingUser}
  <div class="fixed inset-0 flex items-center justify-center z-50 modal-backdrop" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) { showEditModal = false; editingUser = null; } }}
       onkeydown={(e) => { if (e.key === 'Escape') { showEditModal = false; editingUser = null; } }}>
    <div class="card max-w-md w-full mx-4 max-h-[90vh] overflow-y-auto">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-4">Edit {editingUser.username}</h2>
      <div class="space-y-3">
        <input bind:value={editForm.display_name} placeholder="Display Name" class="input w-full text-xs" />
        <div>
          <p class="text-[10px] uppercase tracking-wider text-deep-400 mb-1.5">Roles (any number)</p>
          <div class="flex flex-wrap gap-1.5">
            {#each roles as r (r.name)}
              <label class="flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider px-2 py-1 border cursor-pointer transition-colors"
                     style={editForm.roles.has(r.name)
                       ? `color:${hexColor(r.color)}; border-color:${hexColor(r.color)}88; background:${hexColor(r.color)}1a`
                       : 'color:var(--deep-300); border-color:rgb(var(--deep-600) / 0.5)'}>
                <input type="checkbox" class="accent-bedrock-500 w-3 h-3" checked={editForm.roles.has(r.name)}
                       onchange={() => editForm.roles = toggleRole(editForm.roles, r.name)} />
                {r.display_name}
              </label>
            {/each}
          </div>
          <p class="text-[10px] text-deep-400 mt-1">Assigning or removing the Owner role requires being an owner.</p>
        </div>
        <input bind:value={editForm.password} type="password" placeholder="New password (leave empty to keep)" class="input w-full text-xs" />
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => { showEditModal = false; editingUser = null; }} class="btn-secondary text-xs">Cancel</button>
          <button onclick={updateUser} disabled={busy} class="btn-primary text-xs">{busy ? 'Saving…' : 'Save'}</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Delete Confirmation -->
{#if deleting}
  <div class="fixed inset-0 flex items-center justify-center z-50 modal-backdrop" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => { if (e.target === e.currentTarget) deleting = null; }}
       onkeydown={(e) => { if (e.key === 'Escape') deleting = null; }}>
    <div class="card max-w-sm w-full mx-4">
      <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-2">Delete User</h2>
      <p class="text-xs text-deep-300 mb-4">Are you sure you want to delete <strong>{deleting}</strong>? This cannot be undone.</p>
      <div class="flex justify-end gap-2">
        <button onclick={() => deleting = null} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => deleteUser(deleting!)} disabled={busy} class="btn-danger text-xs">{busy ? 'Deleting…' : 'Delete'}</button>
      </div>
    </div>
  </div>
{/if}
