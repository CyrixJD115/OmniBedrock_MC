<script lang="ts">
  /** Trashed backups: put back, or empty the trash for good. */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import type { Backup } from '$types/index';
  import { Undo2, Trash2, X, Loader2 } from '@lucide/svelte';

  let { open = $bindable(false), world, canDelete, onchanged }: {
    open?: boolean;
    world: string;
    canDelete: boolean;
    onchanged: () => void;
  } = $props();

  let items = $state<Backup[]>([]);
  let loading = $state(false);
  let confirmEmpty = $state(false);

  async function load() {
    loading = true;
    try { items = await api.listTrash(world || undefined); } catch (e: any) { addToast(`Trash: ${e.message}`, 'error'); }
    loading = false;
  }

  $effect(() => {
    if (open) { confirmEmpty = false; void load(); }
  });

  async function putBack(t: Backup) {
    try {
      await api.restoreBackup(t.world, t.filename);
      addToast(`${t.filename} moved back`, 'success');
      await load();
      onchanged();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  async function empty() {
    try {
      const r = await api.emptyTrash(world || undefined);
      addToast(r.message, 'success');
      confirmEmpty = false;
      await load();
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
  }

  const total = $derived(items.reduce((a, t) => a + t.size_bytes, 0));
</script>

{#if open}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-backdrop" role="presentation"
       onclick={(e) => { if (e.target === e.currentTarget) open = false; }}
       onkeydown={(e) => e.key === 'Escape' && (open = false)}>
    <div class="card w-full max-w-xl" role="dialog" aria-modal="true" aria-label="Backup trash">
      <div class="flex items-center justify-between mb-3">
        <h2 class="text-sm font-bold text-strong uppercase tracking-widest">Trash · {world || 'all worlds'}</h2>
        <button class="btn-ghost p-1" onclick={() => (open = false)} aria-label="Close"><X size={14} /></button>
      </div>
      <div class="max-h-80 overflow-y-auto space-y-1">
        {#if loading}
          <p class="text-center py-6 text-xs text-deep-400"><Loader2 size={14} class="inline animate-spin" /> Loading…</p>
        {:else}
          {#each items as t (t.world + t.filename)}
            <div class="flex justify-between items-center gap-3 p-2 mc-slot text-xs">
              <span class="min-w-0">
                <span class="block font-mono text-deep-200 truncate" title={t.filename}>{t.filename}</span>
                <span class="text-[10px] text-deep-400">{t.world} · {formatBytes(t.size_bytes)} · trashed {new Date(t.modified).toLocaleDateString()}</span>
              </span>
              <button onclick={() => putBack(t)} class="btn-ghost text-[10px] px-2 py-1 text-teal-300 flex items-center gap-1 shrink-0"><Undo2 size={11} /> Put back</button>
            </div>
          {:else}
            <p class="text-deep-400 text-center py-8 text-xs">The trash is empty.</p>
          {/each}
        {/if}
      </div>
      <div class="flex items-center justify-between mt-4 gap-3">
        <span class="text-[10px] text-deep-400">{items.length} item(s) · {formatBytes(total)}</span>
        {#if canDelete && items.length}
          {#if confirmEmpty}
            <span class="flex items-center gap-2 text-xs">
              <span class="text-red-300">Delete permanently?</span>
              <button class="btn-danger text-xs px-3 py-1" onclick={empty}>Delete {items.length}</button>
              <button class="btn-ghost text-xs px-2 py-1" onclick={() => (confirmEmpty = false)}>Cancel</button>
            </span>
          {:else}
            <button class="btn-ghost text-xs px-3 py-1 text-red-300 flex items-center gap-1" onclick={() => (confirmEmpty = true)}><Trash2 size={11} /> Empty trash</button>
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}
