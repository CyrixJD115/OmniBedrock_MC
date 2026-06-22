<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { Shield, ShieldOff, UserPlus, UserCog } from '@lucide/svelte';

  let users = $state<{ username: string; display_name: string; role: string; created_at: string }[]>([]);
  let loading = $state(true);
  let showCreateModal = $state(false);
  let showEditModal = $state(false);
  let editingUser = $state<{ username: string; display_name: string; role: string } | null>(null);
  let form = $state({ username: '', password: '', display_name: '', role: 'viewer' });
  let editForm = $state({ display_name: '', role: 'viewer', password: '' });
  let deleting = $state<string | null>(null);

  onMount(loadUsers);

  async function loadUsers() {
    loading = true;
    try { users = (await api.listUsers()) as any; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  }

  function openCreate() {
    form = { username: '', password: '', display_name: '', role: 'viewer' };
    showCreateModal = true;
  }

  function openEdit(u: typeof users[0]) {
    editingUser = u;
    editForm = { display_name: u.display_name, role: u.role, password: '' };
    showEditModal = true;
  }

  async function createUser() {
    if (!form.username || !form.password) { addToast('Username and password required', 'error'); return; }
    try {
      await api.createUser(form);
      addToast(`Created ${form.username}`, 'success');
      showCreateModal = false;
      await loadUsers();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function updateUser() {
    if (!editingUser) return;
    const data: Record<string, unknown> = { display_name: editForm.display_name, role: editForm.role };
    if (editForm.password) data.password = editForm.password;
    try {
      await api.updateUser(editingUser.username, data);
      addToast(`Updated ${editingUser.username}`, 'success');
      showEditModal = false;
      editingUser = null;
      await loadUsers();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function deleteUser(username: string) {
    try {
      await api.deleteUser(username);
      addToast(`Deleted ${username}`, 'success');
      deleting = null;
      await loadUsers();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Users</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadUsers} class="btn-secondary text-xs">Refresh</button>
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
            <th class="text-left py-2 px-3 font-medium">Role</th>
            <th class="text-left py-2 px-3 font-medium">Created</th>
            {#if $userPermissions.includes('USERS_EDIT') || $userPermissions.includes('USERS_DELETE')}
              <th class="text-right py-2 px-3 font-medium">Actions</th>
            {/if}
          </tr>
        </thead>
        <tbody>
          {#each users as u}
            <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
              <td class="py-1.5 px-3 font-medium text-white">{u.username}</td>
              <td class="py-1.5 px-3">{u.display_name}</td>
              <td class="py-1.5 px-3">
                <span class="text-{u.role === 'owner' ? 'amber' : u.role === 'admin' ? 'bedrock' : u.role === 'moderator' ? 'teal' : 'deep'}-400">
                  {u.role}
                </span>
              </td>
              <td class="py-1.5 px-3 text-deep-400">{new Date(u.created_at).toLocaleDateString()}</td>
              {#if $userPermissions.includes('USERS_EDIT') || $userPermissions.includes('USERS_DELETE')}
                <td class="py-1.5 px-3 text-right">
                  {#if $userPermissions.includes('USERS_EDIT')}
                    <button onclick={() => openEdit(u)} class="btn-ghost p-1 text-bedrock-400" title="Edit"><UserCog size={12} /></button>
                  {/if}
                  {#if $userPermissions.includes('USERS_DELETE')}
                    <button onclick={() => deleting = u.username} class="btn-ghost p-1 text-red-400" title="Delete"><ShieldOff size={12} /></button>
                  {/if}
                </td>
              {/if}
            </tr>
          {:else}
            <tr><td colspan="{$userPermissions.includes('USERS_EDIT') || $userPermissions.includes('USERS_DELETE') ? 5 : 4}" class="text-center py-8 text-deep-500">{loading ? 'Loading...' : 'No users'}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>

<!-- Create Modal -->
{#if showCreateModal}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onclick={() => showCreateModal = false}>
    <div class="card max-w-md w-full mx-4" onclick={(e) => e.stopPropagation()}>
      <h2 class="text-sm font-bold text-white uppercase tracking-wider mb-4">Create User</h2>
      <div class="space-y-3">
        <input bind:value={form.username} placeholder="Username" class="input w-full text-xs" />
        <input bind:value={form.password} type="password" placeholder="Password" class="input w-full text-xs" />
        <input bind:value={form.display_name} placeholder="Display Name" class="input w-full text-xs" />
        <select bind:value={form.role} class="input w-full text-xs">
          <option value="viewer">Viewer</option>
          <option value="moderator">Moderator</option>
          <option value="admin">Admin</option>
          <option value="owner">Owner</option>
        </select>
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => showCreateModal = false} class="btn-secondary text-xs">Cancel</button>
          <button onclick={createUser} class="btn-primary text-xs">Create</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Edit Modal -->
{#if showEditModal && editingUser}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onclick={() => { showEditModal = false; editingUser = null; }}>
    <div class="card max-w-md w-full mx-4" onclick={(e) => e.stopPropagation()}>
      <h2 class="text-sm font-bold text-white uppercase tracking-wider mb-4">Edit {editingUser.username}</h2>
      <div class="space-y-3">
        <input bind:value={editForm.display_name} placeholder="Display Name" class="input w-full text-xs" />
        <select bind:value={editForm.role} class="input w-full text-xs">
          <option value="viewer">Viewer</option>
          <option value="moderator">Moderator</option>
          <option value="admin">Admin</option>
          <option value="owner">Owner</option>
        </select>
        <input bind:value={editForm.password} type="password" placeholder="New password (leave empty to keep)" class="input w-full text-xs" />
        <div class="flex justify-end gap-2 pt-2">
          <button onclick={() => { showEditModal = false; editingUser = null; }} class="btn-secondary text-xs">Cancel</button>
          <button onclick={updateUser} class="btn-primary text-xs">Save</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Delete Confirmation -->
{#if deleting}
  <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onclick={() => deleting = null}>
    <div class="card max-w-sm w-full mx-4" onclick={(e) => e.stopPropagation()}>
      <h2 class="text-sm font-bold text-white uppercase tracking-wider mb-2">Delete User</h2>
      <p class="text-xs text-deep-300 mb-4">Are you sure you want to delete <strong>{deleting}</strong>? This cannot be undone.</p>
      <div class="flex justify-end gap-2">
        <button onclick={() => deleting = null} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => deleteUser(deleting!)} class="btn-danger text-xs">Delete</button>
      </div>
    </div>
  </div>
{/if}
