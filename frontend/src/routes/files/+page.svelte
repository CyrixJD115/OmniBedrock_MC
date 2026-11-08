<!-- FILES: two sub-tabs — the classic config-file editor and the
     instance-scoped file explorer. The page itself is gated by FILES_VIEW
     (sidebar); write actions gate again on FILES_WRITE server-side. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { isServerRunning } from '$stores/index';
  import { activeInstanceId } from '$stores/instances';
  import { FileText, Save, Trash2, RotateCw, FolderTree } from '@lucide/svelte';
  import FileExplorer from '$lib/components/files/FileExplorer.svelte';

  let subTab = $state<'config' | 'explorer'>('config');
  let files = $state<{ name: string; size: number; modified: number }[]>([]);
  let selected = $state('');
  let content = $state('');
  let loading = $state(true);

  let canWrite = $derived($userPermissions.includes('FILES_WRITE'));

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

  const tabClass = (tab: 'config' | 'explorer') =>
    'flex items-center gap-2 px-3 py-1.5 text-xs uppercase tracking-wider font-semibold transition-all border-b-2 ' +
    (subTab === tab
      ? 'border-bedrock-400 text-bedrock-300 bg-bedrock-500/10'
      : 'border-transparent text-deep-400 hover:text-deep-200');
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Files</h1>
      <div class="pixel-divider mt-2 w-28"></div>
    </div>
    <div class="flex gap-1 border-b border-deep-700/60">
      <button onclick={() => (subTab = 'config')} class={tabClass('config')}>
        <FileText size={13} /> Config Files
      </button>
      <button onclick={() => (subTab = 'explorer')} class={tabClass('explorer')}>
        <FolderTree size={13} /> Explorer
      </button>
    </div>
  </div>

  {#if subTab === 'config'}
    <div class="flex items-center justify-between">
      <div></div>
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
          {:else}<p class="text-deep-400 text-xs py-4">No files</p>{/each}
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
          <div class="flex items-center justify-center h-48 text-deep-400 text-xs uppercase tracking-wider">Select a file</div>
        {/if}
      </div>
    </div>
  {:else}
    <FileExplorer
      instanceId={$activeInstanceId ?? 'default'}
      canWrite={canWrite}
      serverRunning={$isServerRunning}
    />
  {/if}
</div>
