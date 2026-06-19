<script lang="ts">
  import { api } from '$lib/api/client';
  import type { FolderEntry } from '$types/index';

  let { open = false, current = '', onconfirm, oncancel }: {
    open: boolean; current?: string; onconfirm: (path: string) => void; oncancel: () => void;
  } = $props();

  let base = $state(current);
  let dirs = $state<FolderEntry[]>([]);
  let loading = $state(false);

  $effect(() => {
    if (open) {
      base = current;
      loadDirs(current);
    }
  });

  async function loadDirs(path: string) {
    loading = true;
    try {
      const res = await api.listFolders(path || undefined);
      dirs = res.dirs;
      base = res.base;
    } catch {
      dirs = [];
    }
    loading = false;
  }

  function enterDir(entry: FolderEntry) {
    loadDirs(entry.path);
  }

  function goUp() {
    const parts = base.split('/').filter(Boolean);
    parts.pop();
    loadDirs(parts.join('/'));
  }
</script>

{#if open}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) oncancel(); }}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg shadow-black/50"
         style="box-shadow: inset 2px 2px 0 rgba(255,255,255,0.03), inset -1px -1px 0 rgba(0,0,0,0.3), 6px 6px 0 rgba(0,0,0,0.5);">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-1">Export Folder</h2>
      <p class="text-deep-400 text-xs mb-3">Browse to select the backup export folder.</p>

      <div class="flex items-center gap-2 mb-2">
        <span class="text-deep-500 text-xs">Path:</span>
        <span class="text-deep-200 text-xs font-mono truncate flex-1">/{base || '.'}</span>
        {#if base}
          <button onclick={goUp} class="btn-ghost text-xs px-1 py-0.5 rounded border border-deep-600/30 text-deep-300 hover:text-deep-100">&uarr; Up</button>
        {/if}
      </div>

      <div class="max-h-56 overflow-y-auto border border-deep-600/30 rounded p-2 bg-deep-800/40">
        {#if loading}
          <p class="text-deep-400 text-xs text-center py-4">Loading...</p>
        {:else if dirs.length === 0}
          <p class="text-deep-500 text-xs text-center py-4">No subdirectories.</p>
        {:else}
          {#each dirs as entry}
            <button
              onclick={() => enterDir(entry)}
              class="w-full text-left flex items-center gap-2 px-2 py-1.5 rounded hover:bg-deep-700/40 text-xs text-deep-200 transition"
            >
              <span class="text-bedrock-400 font-semibold">{entry.name}</span>
            </button>
          {/each}
        {/if}
      </div>

      <div class="flex gap-2 justify-end mt-4">
        <button onclick={oncancel} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => onconfirm(base)} class="btn-ghost text-xs px-3 py-1 rounded uppercase tracking-wider border border-bedrock-500/40 text-bedrock-400 hover:bg-bedrock-500/10">
          Select &quot;/{base || '.'}&quot;
        </button>
      </div>
    </div>
  </div>
{/if}
