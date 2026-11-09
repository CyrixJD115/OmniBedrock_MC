<!-- Users & Roles: one page, two sub-tabs. Both panels keep their own
     toolbars and load lazily when their tab is shown; /roles redirects here
     with #roles so old links keep working. -->
<script lang="ts">
  import { Users as UsersIcon, Shield } from '@lucide/svelte';
  import UsersPanel from '$lib/components/users/UsersPanel.svelte';
  import RolesPanel from '$lib/components/users/RolesPanel.svelte';

  let tab = $state<'users' | 'roles'>(initial());

  function initial(): 'users' | 'roles' {
    if (typeof location !== 'undefined' && location.hash === '#roles') return 'roles';
    return 'users';
  }

  function pick(next: 'users' | 'roles') {
    tab = next;
    if (typeof history !== 'undefined') {
      history.replaceState(null, '', next === 'roles' ? '#roles' : '#users');
    }
  }

  function tabClass(target: 'users' | 'roles'): string {
    return 'flex items-center gap-1.5 px-3 py-1.5 text-xs uppercase tracking-wider font-semibold transition-all border-b-2 ' +
      (tab === target
        ? 'border-bedrock-400 text-bedrock-300 bg-bedrock-500/10'
        : 'border-transparent text-deep-400 hover:text-deep-200');
  }
</script>

<div class="space-y-4">
  <div>
    <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Users &amp; Roles</h1>
    <div class="pixel-divider mt-2 w-28"></div>
    <div class="flex gap-1 mt-3" role="tablist" aria-label="Users and roles">
      <button onclick={() => pick('users')} class={tabClass('users')} role="tab" aria-selected={tab === 'users'}>
        <UsersIcon size={13} /> Users
      </button>
      <button onclick={() => pick('roles')} class={tabClass('roles')} role="tab" aria-selected={tab === 'roles'}>
        <Shield size={13} /> Roles
      </button>
    </div>
  </div>

  {#if tab === 'users'}
    <UsersPanel />
  {:else}
    <RolesPanel />
  {/if}
</div>
