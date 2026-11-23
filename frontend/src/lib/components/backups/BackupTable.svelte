<script lang="ts">
  /** Backup archives with their metadata (live/offline, files, verification). */
  import type { Backup } from '$types/index';
  import { formatBytes } from '$lib/utils';
  import { Download, RotateCcw, Trash2, ShieldCheck, ShieldAlert, ShieldQuestion, Loader2, Zap, Clock } from '@lucide/svelte';

  let { backups, canRestore, canDelete, verifying, ondownload, onrestore, ondelete, onverify }: {
    backups: Backup[];
    canRestore: boolean;
    canDelete: boolean;
    verifying: string | null;
    ondownload: (b: Backup) => void;
    onrestore: (b: Backup) => void;
    ondelete: (b: Backup) => void;
    onverify: (b: Backup) => void;
  } = $props();

  function duration(ms: number): string {
    return ms < 1000 ? `${ms} ms` : ms < 60_000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
  }
  function when(iso: string): string {
    const d = new Date(iso);
    const mins = (Date.now() - d.getTime()) / 60_000;
    if (mins < 60) return `${Math.max(1, Math.round(mins))} min ago`;
    if (mins < 60 * 24) return `${Math.round(mins / 60)} h ago`;
    return d.toLocaleDateString();
  }
</script>

<div class="overflow-x-auto">
  <table class="w-full text-xs">
    <thead>
      <tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider text-[10px]">
        <th class="text-left py-2 px-3 font-medium">Backup</th>
        <th class="text-left py-2 px-3 font-medium hidden md:table-cell">Details</th>
        <th class="text-right py-2 px-3 font-medium">Size</th>
        <th class="text-right py-2 px-3 font-medium">Created</th>
        <th class="text-right py-2 px-3 font-medium">Actions</th>
      </tr>
    </thead>
    <tbody>
      {#each backups as b (b.world + b.filename)}
        {@const m = b.meta}
        <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
          <td class="py-2 px-3 min-w-0">
            <p class="flex items-center gap-1.5">
              {#if b.kind === 'auto'}<Clock size={11} class="text-bedrock-300 shrink-0" />{/if}
              <span class="text-strong font-medium">{b.world}</span>
              <span class="{b.kind === 'auto' ? 'badge-info' : 'badge-running'} !text-[8px] !py-0 !px-1">{b.kind}</span>
            </p>
            <p class="font-mono text-[10px] text-deep-400 truncate max-w-[22rem]" title={b.filename}>{b.filename}</p>
          </td>
          <td class="py-2 px-3 hidden md:table-cell">
            {#if m}
              <span class="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-deep-300">
                {#if m.verified === true}
                  <span class="flex items-center gap-1 text-teal-300" title="Every entry re-read and CRC-checked"><ShieldCheck size={11} />verified</span>
                {:else if m.verified === false}
                  <span class="flex items-center gap-1 text-red-300"><ShieldAlert size={11} />failed check</span>
                {:else}
                  <span class="flex items-center gap-1 text-deep-400"><ShieldQuestion size={11} />not verified</span>
                {/if}
                {#if m.live}<span class="flex items-center gap-1 text-yellow-300" title="Taken while running with save hold; {m.truncated} file(s) cut to the held length"><Zap size={10} />live</span>{/if}
                <span>{m.files.toLocaleString()} files</span>
                <span>{duration(m.duration_ms)}</span>
                {#if m.server_version}<span class="font-mono text-deep-400">{m.server_version}</span>{/if}
              </span>
            {:else}
              <span class="text-[10px] text-deep-400">no metadata (older backup)</span>
            {/if}
          </td>
          <td class="py-2 px-3 text-right font-mono whitespace-nowrap">{formatBytes(b.size_bytes)}</td>
          <td class="py-2 px-3 text-right text-deep-300 whitespace-nowrap" title={new Date(b.modified).toLocaleString()}>{when(b.modified)}</td>
          <td class="py-2 px-3">
            <div class="flex justify-end gap-0.5">
              <button onclick={() => onverify(b)} class="btn-ghost p-1.5" title="Verify (re-read and CRC-check every file)" disabled={verifying !== null}>
                {#if verifying === b.filename}<Loader2 size={12} class="animate-spin" />{:else}<ShieldCheck size={12} />{/if}
              </button>
              <button onclick={() => ondownload(b)} class="btn-ghost p-1.5" title="Download"><Download size={12} /></button>
              {#if canRestore}<button onclick={() => onrestore(b)} class="btn-ghost p-1.5 text-teal-300" title="Restore this backup"><RotateCcw size={12} /></button>{/if}
              {#if canDelete}<button onclick={() => ondelete(b)} class="btn-ghost p-1.5 text-red-400" title="Move to trash"><Trash2 size={12} /></button>{/if}
            </div>
          </td>
        </tr>
      {:else}
        <tr><td colspan="5" class="text-center py-10 text-deep-400">No backups yet</td></tr>
      {/each}
    </tbody>
  </table>
</div>
