<script lang="ts">
  /** Confirm restoring a backup over its world. */
  import type { Backup } from '$types/index';
  import { formatBytes } from '$lib/utils';
  import { TriangleAlert, X } from '@lucide/svelte';

  let { target, busy = false, onconfirm, oncancel }: {
    target: Backup | null;
    busy?: boolean;
    onconfirm: (startAfter: boolean) => void;
    oncancel: () => void;
  } = $props();

  let startAfter = $state(true);
  let typed = $state('');

  $effect(() => {
    target;
    typed = '';
  });
</script>

{#if target}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-backdrop" role="presentation"
       onclick={(e) => { if (e.target === e.currentTarget && !busy) oncancel(); }}
       onkeydown={(e) => e.key === 'Escape' && !busy && oncancel()}>
    <div class="card w-full max-w-lg border-yellow-600/50 space-y-3 text-xs" role="dialog" aria-modal="true" aria-label="Restore backup">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-bold text-strong uppercase tracking-widest">Restore backup</h2>
        <button class="btn-ghost p-1" onclick={oncancel} disabled={busy} aria-label="Close"><X size={14} /></button>
      </div>
      <p class="text-deep-200">
        Replace world <b class="text-strong">{target.world}</b> with
        <code class="font-mono bg-deep-800/60 px-1">{target.filename}</code>
        ({formatBytes(target.size_bytes)}, {new Date(target.modified).toLocaleString()}).
      </p>
      {#if target.meta?.verified === false}
        <p class="text-red-300">This archive failed verification; restoring it may not work.</p>
      {/if}
      <div class="flex items-start gap-2 p-2 border border-yellow-700/50 bg-yellow-900/20 text-yellow-200">
        <TriangleAlert size={14} class="shrink-0 mt-0.5" />
        <span>The server is stopped first. The current world is kept as a hidden pre-restore copy (the newest two are kept), so this can be undone from the Files tab.</span>
      </div>
      <label class="flex items-center gap-2 cursor-pointer text-deep-200">
        <input type="checkbox" bind:checked={startAfter} class="accent-bedrock-500" /> Start the server after restoring
      </label>
      <label class="block text-deep-300">Type the world name to confirm
        <input class="input w-full mt-1 text-xs" bind:value={typed} placeholder={target.world} disabled={busy} />
      </label>
      <div class="flex justify-end gap-2">
        <button onclick={oncancel} class="btn-secondary text-xs" disabled={busy}>Cancel</button>
        <button onclick={() => onconfirm(startAfter)} disabled={busy || typed.trim() !== target.world}
                class="btn-danger text-xs px-4 py-1.5 uppercase tracking-wider disabled:opacity-40">{busy ? 'Restoring…' : 'Restore'}</button>
      </div>
    </div>
  </div>
{/if}
