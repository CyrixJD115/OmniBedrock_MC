<script lang="ts">
  /** CurseForge API key for the Store. */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Save, Store as StoreIcon } from '@lucide/svelte';
  import { onMount } from 'svelte';

  let { canEditSettings }: { canEditSettings: boolean } = $props();
  let cfSettings = $state<{ configured: boolean; key_hint: string | null; source: 'settings' | 'env' } | null>(null);
  let cfKeyInput = $state('');
  let cfBusy = $state(false);

  onMount(async () => {
    try { cfSettings = await api.getCurseForgeSettings(); } catch { /* silent */ }
  });

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
