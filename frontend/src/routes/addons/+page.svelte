<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Package, Eye, RotateCw } from '@lucide/svelte';

  let bp = $state<any[]>([]);
  let rp = $state<any[]>([]);
  let loading = $state(true);
  let manifest: any = $state(null);

  onMount(async () => {
    try {
      const a = await api.listAddons();
      bp = a.behavior_packs;
      rp = a.resource_packs;
    } catch (e: any) { addToast(`Failed to load addons: ${e.message}`, 'error'); }
    loading = false;
  });

  async function viewManifest(path: string) {
    try { manifest = await api.getManifest(path); }
    catch (e: any) { addToast(`Failed to load manifest: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Addon Organizer</h1>
      <div class="pixel-divider mt-2 w-40"></div>
    </div>
    <button onclick={async () => { try { const a = await api.listAddons(); bp = a.behavior_packs; rp = a.resource_packs; addToast('Reloaded', 'success'); } catch {} }}
            class="btn-ghost p-2"><RotateCw size={14} /></button>
  </div>

  {#if manifest}
    <div class="card">
      <div class="flex items-center justify-between mb-4">
        <h2 class="card-header mb-0">Manifest</h2>
        <button onclick={() => manifest = null} class="btn-secondary text-xs">Close</button>
      </div>
      <pre class="text-xs font-mono bg-deep-950 p-4 overflow-auto max-h-80 border border-deep-600/30">{JSON.stringify(manifest, null, 2)}</pre>
    </div>
  {/if}

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    <div class="card"><h2 class="card-header">Behavior Packs ({bp.length})</h2>
      <div class="space-y-1 max-h-80 overflow-y-auto">
        {#each bp as p}
          <div class="flex items-center justify-between p-2 border border-deep-600/20 hover:bg-deep-800/30">
            <div class="flex items-center gap-2">
              <Package size={14} class="text-bedrock-400 shrink-0" />
              <div><p class="text-xs font-medium">{p.name}</p><p class="text-[10px] text-deep-500">{p.uuid?.slice(0,8)}</p></div>
            </div>
            <button onclick={() => viewManifest(p.path)} class="btn-ghost p-1"><Eye size={12} /></button>
          </div>
        {:else}<p class="text-deep-500 text-xs py-4 text-center">None found</p>{/each}
      </div>
    </div>
    <div class="card"><h2 class="card-header">Resource Packs ({rp.length})</h2>
      <div class="space-y-1 max-h-80 overflow-y-auto">
        {#each rp as p}
          <div class="flex items-center justify-between p-2 border border-deep-600/20 hover:bg-deep-800/30">
            <div class="flex items-center gap-2">
              <Package size={14} class="text-teal-400 shrink-0" />
              <div><p class="text-xs font-medium">{p.name}</p><p class="text-[10px] text-deep-500">{p.uuid?.slice(0,8)}</p></div>
            </div>
            <button onclick={() => viewManifest(p.path)} class="btn-ghost p-1"><Eye size={12} /></button>
          </div>
        {:else}<p class="text-deep-500 text-xs py-4 text-center">None found</p>{/each}
      </div>
    </div>
  </div>
</div>
