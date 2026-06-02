<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { FileText, Save, Trash2, RotateCw } from '@lucide/svelte';

  let files = $state<{ name: string; size: number; modified: number }[]>([]);
  let selected = $state('');
  let content = $state('');
  let loading = $state(true);

  onMount(async () => {
    try { files = await api.listIniFiles(); }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  });

  async function open(name: string) {
    selected = name;
    try { const r = await api.readIniFile(name); content = r.content; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function save() {
    if (!selected) return;
    try { await api.writeIniFile(selected, content); addToast('Saved', 'success'); }
    catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  async function del(name: string) {
    try {
      await api.deleteIniFile(name);
      files = await api.listIniFiles();
      if (selected === name) { selected = ''; content = ''; }
      addToast('Deleted', 'success');
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Config Files</h1>
      <div class="pixel-divider mt-2 w-28"></div>
    </div>
    <button onclick={async () => { try { files = await api.listIniFiles(); } catch {} }} class="btn-ghost p-2"><RotateCw size={14} /></button>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-4 gap-4">
    <div class="card lg:col-span-1">
      <h2 class="card-header">INI Files</h2>
      <div class="space-y-1">
        {#each files as f}
          <div onclick={() => open(f.name)} onkeydown={(e) => e.key === 'Enter' && open(f.name)}
               role="button" tabindex="0"
               class={"flex items-center gap-2 p-2 text-xs cursor-pointer border-l-2 transition-all " + (selected === f.name ? 'bg-bedrock-500/10 border-bedrock-400 text-bedrock-300' : 'border-transparent hover:border-deep-500 text-deep-300 hover:bg-deep-800/50')}>
            <FileText size={12} class="shrink-0" />
            <span class="truncate flex-1">{f.name}</span>
            <button onclick={(e) => { e.stopPropagation(); del(f.name); }} class="p-0.5 hover:text-red-400 shrink-0"><Trash2 size={10} /></button>
          </div>
        {:else}<p class="text-deep-500 text-xs py-4">No files</p>{/each}
      </div>
    </div>

    <div class="card lg:col-span-3">
      {#if selected}
        <div class="flex items-center justify-between mb-4">
          <h2 class="card-header mb-0">{selected}</h2>
          <button onclick={save} class="btn-primary flex items-center gap-2 text-xs"><Save size={14} /> Save</button>
        </div>
        <textarea bind:value={content} class="input w-full h-[55vh] font-mono text-sm" spellcheck="false"></textarea>
      {:else}
        <div class="flex items-center justify-center h-48 text-deep-500 text-xs uppercase tracking-wider">Select a file</div>
      {/if}
    </div>
  </div>
</div>
