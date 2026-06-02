<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { FileText, Download, Trash2, Save, RefreshCw } from '@lucide/svelte';

  let files = $state<{ name: string; size: number; modified: number }[]>([]);
  let selectedFile = $state('');
  let fileContent = $state('');
  let loading = $state(true);

  onMount(async () => {
    try {
      files = await api.listIniFiles();
    } catch { /* ignore */ }
    loading = false;
  });

  async function openFile(name: string) {
    selectedFile = name;
    const res = await api.readIniFile(name);
    fileContent = res.content;
  }

  async function saveFile() {
    if (!selectedFile) return;
    await api.writeIniFile(selectedFile, fileContent);
  }

  async function deleteFile(name: string) {
    await api.deleteIniFile(name);
    files = await api.listIniFiles();
    if (selectedFile === name) {
      selectedFile = '';
      fileContent = '';
    }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Configuration Files</h1>
    <button onclick={async () => files = await api.listIniFiles()} class="btn-ghost p-2">
      <RefreshCw size={16} />
    </button>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-4 gap-4">
    <div class="card lg:col-span-1">
      <h2 class="card-header">INI Files</h2>
      <div class="space-y-1">
        {#each files as f}
          <div onclick={() => openFile(f.name)}
               onkeydown={(e) => e.key === 'Enter' && openFile(f.name)}
               role="button" tabindex="0"
               class={"w-full text-left flex items-center gap-2 p-2 rounded-lg text-sm transition-colors cursor-pointer " + (selectedFile === f.name ? "bg-bedrock-600/20 text-bedrock-300" : "hover:bg-surface-800")}>
            <FileText size={14} class="shrink-0" />
            <span class="truncate flex-1">{f.name}</span>
            <button type="button" onclick={(e) => { e.stopPropagation(); deleteFile(f.name); }}
                    class="p-0.5 hover:text-red-400 shrink-0 cursor-pointer bg-transparent border-0">
              <Trash2 size={12} />
            </button>
          </div>
        {:else}
          <p class="text-surface-500 text-sm py-4">No INI files found</p>
        {/each}
      </div>
    </div>

    <div class="card lg:col-span-3">
      {#if selectedFile}
        <div class="flex items-center justify-between mb-4">
          <h2 class="card-header mb-0">{selectedFile}</h2>
          <button onclick={saveFile} class="btn-primary flex items-center gap-2">
            <Save size={16} /> Save
          </button>
        </div>
        <textarea bind:value={fileContent}
                  class="input w-full h-[60vh] font-mono text-sm" spellcheck="false"></textarea>
      {:else}
        <div class="flex items-center justify-center h-48 text-surface-500">
          Select a file to edit
        </div>
      {/if}
    </div>
  </div>
</div>
