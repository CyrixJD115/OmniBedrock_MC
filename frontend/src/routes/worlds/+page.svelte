<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import { Globe } from '@lucide/svelte';

  let worlds = $state<string[]>([]);
  let selected = $state('');
  let info: any = $state(null);
  let loading = $state(true);

  onMount(async () => {
    try {
      worlds = await api.listWorldsInfo();
      if (worlds.length) { selected = worlds[0]; info = await api.getWorldInfo(selected); }
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  });

  async function select(name: string) {
    selected = name;
    try { info = await api.getWorldInfo(name); }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <div>
    <h1 class="text-lg font-bold text-white uppercase tracking-widest">Worlds</h1>
    <div class="pixel-divider mt-2 w-24"></div>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-4 gap-4">
    <div class="card lg:col-span-1">
      <h2 class="card-header">Available</h2>
      <div class="space-y-1">
        {#each worlds as w}
          <div onclick={() => select(w)} onkeydown={(e) => e.key === 'Enter' && select(w)}
               role="button" tabindex="0"
               class={"flex items-center gap-2 p-2 text-xs uppercase tracking-wider cursor-pointer border-l-2 transition-all " + (selected === w ? 'bg-bedrock-500/10 border-bedrock-400 text-bedrock-300' : 'border-transparent hover:border-deep-500 text-deep-300 hover:bg-deep-800/50')}>
            <Globe size={14} class="shrink-0 text-bedrock-400" />
            <span>{w}</span>
          </div>
        {:else}<p class="text-deep-500 text-xs py-4">No worlds</p>{/each}
      </div>
    </div>

    <div class="card lg:col-span-3">
      {#if info}
        <h2 class="card-header">{info.name}</h2>
        <div class="grid grid-cols-3 gap-3 mb-4">
          <div class="bg-deep-900/80 border border-deep-600/30 p-3">
            <p class="text-[10px] text-deep-400 uppercase tracking-wider mb-1">Size</p>
            <p class="text-sm font-bold text-white font-mono">{formatBytes(info.size_bytes)}</p>
          </div>
          <div class="bg-deep-900/80 border border-deep-600/30 p-3">
            <p class="text-[10px] text-deep-400 uppercase tracking-wider mb-1">Files</p>
            <p class="text-sm font-bold text-white font-mono">{info.files.toLocaleString()}</p>
          </div>
          <div class="bg-deep-900/80 border border-deep-600/30 p-3">
            <p class="text-[10px] text-deep-400 uppercase tracking-wider mb-1">Path</p>
            <p class="text-xs font-mono text-deep-300 truncate">{info.path}</p>
          </div>
        </div>
      {:else}
        <div class="flex items-center justify-center h-48 text-deep-500 text-xs uppercase tracking-wider">Select a world</div>
      {/if}
    </div>
  </div>
</div>
