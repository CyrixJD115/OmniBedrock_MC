<script lang="ts">
  /** Read-only panel configuration (internal paths redacted by the API). */
  import { api } from '$lib/api/client';
  import { Settings as SettingsIcon } from '@lucide/svelte';
  import { onMount } from 'svelte';

  let s = $state<Record<string, unknown>>({});
  onMount(async () => {
    try { s = await api.getSettings(); } catch { /* silent */ }
  });
</script>

  <div class="card">
    <h2 class="card-header flex items-center gap-2"><SettingsIcon size={13} /> Application</h2>
    <div class="space-y-3">
      <!-- `ok` is the API's uniform envelope marker, not a setting. -->
      {#each Object.entries(s).filter(([k]) => k !== 'ok') as [k, v]}
        <div class="flex items-center justify-between py-2 border-b border-deep-600/20 last:border-0">
          <span class="text-xs font-mono text-deep-300">{k}</span>
          <span class="text-xs text-deep-400">{String(v)}</span>
        </div>
      {/each}
    </div>
  </div>
