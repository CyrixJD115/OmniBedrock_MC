<script lang="ts">
  /** Panel login requirement. */
  import { api, setToken } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, authToken as authTokenStore } from '$stores/auth';
  import { LockKeyhole, LockOpen } from '@lucide/svelte';
  import { onMount } from 'svelte';

  let { canEditSettings }: { canEditSettings: boolean } = $props();
  let panel = $state<{ login_enabled: boolean } | null>(null);
  let accessBusy = $state(false);

  onMount(async () => {
    try { panel = await api.getPanelSettings(); } catch { /* silent */ }
  });

  async function toggleLogin(event: Event) {
    const enabled = (event.target as HTMLInputElement).checked;
    // Disabling login serves EVERY visitor as the owner account, so the
    // backend demands an explicit confirmation before accepting it.
    if (!enabled && !window.confirm('Disable login? Everyone opening the panel will act as the owner account (Admin).')) {
      (event.target as HTMLInputElement).checked = true;
      return;
    }
    accessBusy = true;
    try {
      const r = await api.updatePanelSettings(
        enabled ? { login_enabled: true } : { login_enabled: false, confirm: true },
      );
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
</script>

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
      {#if $currentUser?.default_credentials}
        <p class="text-[10px] text-yellow-400/90 uppercase tracking-wider">
          <LockOpen size={10} class="inline mr-1" />
          Default credentials (Admin / 123456) are still active — change the password in Users.
        </p>
      {/if}
    </div>
  </div>
