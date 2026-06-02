<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { Settings as SettingsIcon } from '@lucide/svelte';

  let s = $state<Record<string, unknown>>({});

  onMount(async () => {
    try { s = await api.getSettings(); }
    catch { /* silent */ }
  });
</script>

<div class="space-y-4">
  <div>
    <h1 class="text-lg font-bold text-white uppercase tracking-widest">Settings</h1>
    <div class="pixel-divider mt-2 w-24"></div>
  </div>

  <div class="card">
    <h2 class="card-header">Application</h2>
    <div class="space-y-3">
      {#each Object.entries(s) as [k, v]}
        <div class="flex items-center justify-between py-2 border-b border-deep-600/20 last:border-0">
          <span class="text-xs font-mono text-deep-300">{k}</span>
          <span class="text-xs text-deep-400">{String(v)}</span>
        </div>
      {/each}
    </div>
  </div>
</div>
