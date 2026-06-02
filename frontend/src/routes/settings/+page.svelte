<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { Settings as SettingsIcon } from '@lucide/svelte';

  let appSettings = $state<Record<string, unknown>>({});
  let loading = $state(true);

  onMount(async () => {
    try {
      appSettings = await api.getSettings();
    } catch { /* ignore */ }
    loading = false;
  });
</script>

<div class="space-y-4">
  <h1 class="text-2xl font-bold text-white">Settings</h1>

  <div class="card">
    <h2 class="card-header">Application Settings</h2>
    <div class="space-y-4">
      {#each Object.entries(appSettings) as [key, value]}
        <div class="flex items-center justify-between py-2 border-b border-surface-800 last:border-0">
          <span class="text-sm text-surface-300 font-mono">{key}</span>
          <span class="text-sm text-surface-400">{String(value)}</span>
        </div>
      {/each}
    </div>
  </div>
</div>
