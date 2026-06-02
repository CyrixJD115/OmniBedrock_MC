<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { formatBytes } from '$lib/utils';
  import { Globe, FolderOpen, RefreshCw } from '@lucide/svelte';

  let worlds = $state<string[]>([]);
  let selectedWorld = $state('');
  let worldInfo = $state<any>(null);
  let loading = $state(true);

  onMount(async () => {
    try {
      worlds = await api.listWorldsInfo();
      if (worlds.length > 0) {
        selectedWorld = worlds[0];
        worldInfo = await api.getWorldInfo(selectedWorld);
      }
    } catch { /* ignore */ }
    loading = false;
  });

  async function selectWorld(name: string) {
    selectedWorld = name;
    worldInfo = await api.getWorldInfo(name);
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Worlds</h1>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-4 gap-4">
    <div class="card lg:col-span-1">
      <h2 class="card-header">Available Worlds</h2>
      <div class="space-y-2">
        {#each worlds as w}
          <button onclick={() => selectWorld(w)}
                  class={"w-full text-left flex items-center gap-3 p-2.5 rounded-lg transition-colors " + (selectedWorld === w ? "bg-bedrock-600/20 text-bedrock-300" : "hover:bg-surface-800")}>
            <Globe size={16} class="text-bedrock-400" />
            <span class="text-sm font-medium">{w}</span>
          </button>
        {:else}
          <p class="text-surface-500 text-sm">No worlds found</p>
        {/each}
      </div>
    </div>

    <div class="card lg:col-span-3">
      {#if worldInfo}
        <h2 class="card-header">{worldInfo.name}</h2>
        <div class="grid grid-cols-3 gap-4 mb-4">
          <div class="bg-surface-800 rounded-lg p-3">
            <p class="text-xs text-surface-400 mb-1">Size</p>
            <p class="text-lg font-bold text-white">{formatBytes(worldInfo.size_bytes)}</p>
          </div>
          <div class="bg-surface-800 rounded-lg p-3">
            <p class="text-xs text-surface-400 mb-1">Files</p>
            <p class="text-lg font-bold text-white">{worldInfo.files.toLocaleString()}</p>
          </div>
          <div class="bg-surface-800 rounded-lg p-3">
            <p class="text-xs text-surface-400 mb-1">Path</p>
            <p class="text-sm font-mono text-surface-300 truncate">{worldInfo.path}</p>
          </div>
        </div>
      {:else}
        <div class="flex items-center justify-center h-48 text-surface-500">
          Select a world to view details
        </div>
      {/if}
    </div>
  </div>
</div>
