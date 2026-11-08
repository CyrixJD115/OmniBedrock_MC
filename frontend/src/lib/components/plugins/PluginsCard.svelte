<!-- Endstone Plugins: upload/list/delete .py/.whl plugins for the active
     instance's server/plugins directory. Rendered only for endstone-backend
     instances by the dashboard; PLUGIN_MANAGE is enforced server-side too. -->
<script lang="ts">
  import { Blocks, Trash2, Upload, RotateCw, TriangleAlert, Check } from '@lucide/svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { fmtDate, fmtSize } from '$lib/addons/display';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import type { PluginFile } from '$types/index';

  let { instanceId }: { instanceId: string } = $props();

  let plugins = $state<PluginFile[]>([]);
  let loading = $state(true);
  let dragOver = $state(false);
  let restartNeeded = $state(false);
  let uploading = $state(false);
  let confirmOpen = $state(false);
  let pendingDelete = $state<PluginFile | null>(null);

  let fileInput = $state<HTMLInputElement | null>(null);

  // Reload on instance switches; stale responses are dropped by epoch.
  let epoch = 0;
  $effect(() => {
    instanceId;
    const mine = ++epoch;
    restartNeeded = false;
    void load(mine);
  });

  async function load(mine: number = epoch) {
    loading = true;
    try {
      const result = await api.listPlugins(instanceId);
      if (mine !== epoch) return;
      plugins = result.plugins;
    } catch (e: any) {
      if (mine === epoch) addToast(`Failed to list plugins: ${e.message}`, 'error');
    } finally {
      if (mine === epoch) loading = false;
    }
  }

  async function upload(files: FileList | File[], overwrite = false) {
    uploading = true;
    try {
      for (const file of Array.from(files)) {
        try {
          const res = await api.uploadPlugin(instanceId, file, overwrite);
          addToast(res.message, 'success');
          if (res.restart_required) restartNeeded = true;
        } catch (e: any) {
          addToast(`${file.name}: ${e.message}`, 'error');
        }
      }
    } finally {
      uploading = false;
      void load();
    }
  }

  async function doDelete() {
    const target = pendingDelete;
    confirmOpen = false;
    pendingDelete = null;
    if (!target) return;
    try {
      const res = await api.deletePlugin(instanceId, target.filename);
      addToast(res.message, 'success');
      restartNeeded = true;
    } catch (e: any) {
      addToast(`Delete failed: ${e.message}`, 'error');
    }
    void load();
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) void upload(files);
  }
</script>

<div class="card">
  <div class="flex items-center justify-between mb-3">
    <h2 class="card-header mb-0 flex items-center gap-2">
      <Blocks size={16} class="text-bedrock-400" /> Endstone Plugins
    </h2>
    <div class="flex items-center gap-2">
      <button
        onclick={() => fileInput?.click()}
        disabled={uploading}
        class="btn-primary flex items-center gap-1.5 text-xs"
      >
        <Upload size={13} /> {uploading ? 'Uploading…' : 'Upload'}
      </button>
      <button onclick={() => void load()} class="btn-ghost p-1.5" title="Refresh">
        <RotateCw size={14} />
      </button>
      <input
        bind:this={fileInput}
        type="file"
        multiple
        accept=".py,.whl"
        class="hidden"
        onchange={(e) => {
          const files = (e.currentTarget as HTMLInputElement).files;
          if (files && files.length > 0) void upload(files);
          (e.currentTarget as HTMLInputElement).value = '';
        }}
      />
    </div>
  </div>

  {#if restartNeeded}
    <div class="flex items-center justify-between gap-3 border border-yellow-500/40 bg-yellow-500/5 px-3 py-2 mb-3">
      <p class="flex items-center gap-2 text-xs text-yellow-300">
        <TriangleAlert size={14} class="shrink-0" />
        Restart the server to load plugin changes.
      </p>
      <button onclick={() => (restartNeeded = false)} class="text-yellow-400 hover:text-yellow-300" aria-label="Dismiss">
        <Check size={14} />
      </button>
    </div>
  {/if}

  <div
    role="region"
    aria-label="Endstone plugins"
    ondragover={(e) => { e.preventDefault(); dragOver = true; }}
    ondragleave={() => (dragOver = false)}
    ondrop={onDrop}
  >
    <table class="w-full text-xs">
      <thead>
        <tr class="text-left text-deep-400 uppercase tracking-wider text-[10px]">
          <th class="py-2 pr-2">Plugin</th>
          <th class="py-2 pr-2 w-24">Size</th>
          <th class="py-2 pr-2 w-28">Modified</th>
          <th class="py-2 w-12 text-right">Del</th>
        </tr>
      </thead>
      <tbody>
        {#if loading}
          <tr><td colspan="4" class="py-6 text-center text-deep-400">Loading…</td></tr>
        {:else if plugins.length > 0}
          {#each plugins as p (p.filename)}
            <tr class="group border-t border-deep-700/40 hover:bg-deep-800/40 transition-colors">
              <td class="py-1.5 pr-2">
                <button
                  class="text-deep-200 hover:text-strong truncate max-w-72 text-left"
                  title="Download {p.filename}"
                  onclick={() => void api.downloadPlugin(instanceId, p.filename).catch((e: any) => addToast(`Download failed: ${e.message}`, 'error'))}
                >{p.filename}</button>
              </td>
              <td class="py-1.5 pr-2 font-mono text-deep-400">{fmtSize(p.size)}</td>
              <td class="py-1.5 pr-2 text-deep-400">{fmtDate(p.modified)}</td>
              <td class="py-1.5 text-right">
                <button
                  onclick={() => { pendingDelete = p; confirmOpen = true; }}
                  class="btn-ghost p-1 text-red-400 hover:text-red-300 opacity-60 group-hover:opacity-100 transition-opacity"
                  title="Delete {p.filename}"
                ><Trash2 size={12} /></button>
              </td>
            </tr>
          {/each}
        {:else}
          <tr>
            <td colspan="4" class="py-8 text-center text-deep-400 text-xs">
              No plugins — drop .py / .whl files here or use Upload
            </td>
          </tr>
        {/if}
      </tbody>
    </table>
    <p class="text-[10px] text-deep-500 mt-2">
      Plugins live in the instance's <span class="font-mono">server/plugins/</span> directory and load at server start.
    </p>
  </div>
</div>

<ConfirmDialog
  open={confirmOpen}
  title="Delete plugin"
  message="Delete '{pendingDelete?.filename ?? ''}'? It will no longer load on the next restart."
  confirmText="Delete"
  onconfirm={doDelete}
  oncancel={() => { confirmOpen = false; pendingDelete = null; }}
/>
