<script lang="ts">
  import { api } from '$lib/api/client';
  import type { IncludeItem } from '$types/index';

  let { open = false, world = '', selected = [], onconfirm, oncancel }: {
    open: boolean; world?: string; selected?: string[];
    onconfirm: (items: string[]) => void; oncancel: () => void;
  } = $props();

  let items = $state<IncludeItem[]>([]);
  let checked = $state<Set<string>>(new Set(selected));
  let loading = $state(false);

  $effect(() => {
    if (open && world) {
      loading = true;
      api.getIncludeItems(world).then((res) => {
        items = res.items;
        loading = false;
      }).catch(() => { loading = false; });
      checked = new Set(selected);
    }
  });

  function toggle(name: string) {
    const next = new Set(checked);
    if (next.has(name)) next.delete(name); else next.add(name);
    checked = next;
  }

  function selectAll() {
    checked = new Set(items.map((i) => i.name));
  }

  function selectNone() {
    checked = new Set<string>();
  }
</script>

{#if open}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) oncancel(); }}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-lg shadow-block-lg shadow-black/50"
         style="box-shadow: inset 2px 2px 0 rgba(255,255,255,0.03), inset -1px -1px 0 rgba(0,0,0,0.3), 6px 6px 0 rgba(0,0,0,0.5);">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-1">Include Items — {world}</h2>
      <p class="text-deep-400 text-xs mb-3">Select files and folders to include in the backup.</p>

      <div class="flex gap-2 mb-3">
        <button onclick={selectAll} class="btn-ghost text-xs px-2 py-0.5 rounded border border-deep-600/30 text-deep-300 hover:text-deep-100">All</button>
        <button onclick={selectNone} class="btn-ghost text-xs px-2 py-0.5 rounded border border-deep-600/30 text-deep-300 hover:text-deep-100">None</button>
        <span class="text-deep-500 text-xs self-center ml-auto">{checked.size} of {items.length} selected</span>
      </div>

      <div class="max-h-64 overflow-y-auto space-y-1 border border-deep-600/30 rounded p-2 bg-deep-800/40">
        {#if loading}
          <p class="text-deep-400 text-xs text-center py-4">Loading...</p>
        {:else if items.length === 0}
          <p class="text-deep-500 text-xs text-center py-4">No items found.</p>
        {:else}
          {#each items as item}
            <label class="flex items-center gap-2 cursor-pointer hover:bg-deep-700/40 rounded px-2 py-1 text-xs">
              <input type="checkbox" checked={checked.has(item.name)} onchange={() => toggle(item.name)} class="accent-bedrock-500" />
              <span class={item.is_dir ? 'font-semibold text-bedrock-300' : 'text-deep-200'}>{item.name}</span>
              {#if item.is_dir}
                <span class="text-deep-500 text-[10px]">(directory)</span>
              {/if}
            </label>
          {/each}
        {/if}
      </div>

      <div class="flex gap-2 justify-end mt-4">
        <button onclick={oncancel} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => onconfirm([...checked])} class="btn-ghost text-xs px-3 py-1 rounded uppercase tracking-wider border border-bedrock-500/40 text-bedrock-400 hover:bg-bedrock-500/10">
          Confirm ({checked.size})
        </button>
      </div>
    </div>
  </div>
{/if}
