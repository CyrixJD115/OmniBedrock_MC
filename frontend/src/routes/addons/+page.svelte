<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { Package, Eye, RotateCw } from '@lucide/svelte';

  let behaviorPacks = $state<any[]>([]);
  let resourcePacks = $state<any[]>([]);
  let loading = $state(true);
  let selectedManifest = $state<any>(null);
  let showManifestPath = $state('');

  onMount(async () => {
    try {
      const addons = await api.listAddons();
      behaviorPacks = addons.behavior_packs;
      resourcePacks = addons.resource_packs;
    } catch { /* ignore */ }
    loading = false;
  });

  async function viewManifest(path: string) {
    showManifestPath = path;
    selectedManifest = await api.getManifest(path);
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Addon Organizer</h1>
    <button onclick={async () => { const a = await api.listAddons(); behaviorPacks = a.behavior_packs; resourcePacks = a.resource_packs; }}
            class="btn-ghost p-2">
      <RotateCw size={16} />
    </button>
  </div>

  {#if selectedManifest}
    <div class="card">
      <div class="flex items-center justify-between mb-4">
        <h2 class="card-header mb-0">Manifest: {showManifestPath.split('/').pop()}</h2>
        <button onclick={() => selectedManifest = null} class="btn-secondary text-sm">Close</button>
      </div>
      <pre class="text-xs font-mono bg-surface-950 p-4 rounded-lg overflow-auto max-h-96">
        {JSON.stringify(selectedManifest, null, 2)}
      </pre>
    </div>
  {/if}

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    <div class="card">
      <h2 class="card-header">Behavior Packs ({behaviorPacks.length})</h2>
      <div class="space-y-2 max-h-96 overflow-y-auto">
        {#each behaviorPacks as pack}
          <div class="flex items-center justify-between p-2 rounded-lg bg-surface-800/50 hover:bg-surface-800">
            <div class="flex items-center gap-3">
              <Package size={16} class="text-bedrock-400" />
              <div>
                <p class="text-sm font-medium">{pack.name}</p>
                <p class="text-xs text-surface-500">{pack.uuid?.slice(0, 8)}...</p>
              </div>
            </div>
            <button onclick={() => viewManifest(pack.path)} class="btn-ghost p-1.5">
              <Eye size={14} />
            </button>
          </div>
        {:else}
          <p class="text-surface-500 text-sm py-4 text-center">No behavior packs found</p>
        {/each}
      </div>
    </div>

    <div class="card">
      <h2 class="card-header">Resource Packs ({resourcePacks.length})</h2>
      <div class="space-y-2 max-h-96 overflow-y-auto">
        {#each resourcePacks as pack}
          <div class="flex items-center justify-between p-2 rounded-lg bg-surface-800/50 hover:bg-surface-800">
            <div class="flex items-center gap-3">
              <Package size={16} class="text-neon-purple" />
              <div>
                <p class="text-sm font-medium">{pack.name}</p>
                <p class="text-xs text-surface-500">{pack.uuid?.slice(0, 8)}...</p>
              </div>
            </div>
            <button onclick={() => viewManifest(pack.path)} class="btn-ghost p-1.5">
              <Eye size={14} />
            </button>
          </div>
        {:else}
          <p class="text-surface-500 text-sm py-4 text-center">No resource packs found</p>
        {/each}
      </div>
    </div>
  </div>
</div>
