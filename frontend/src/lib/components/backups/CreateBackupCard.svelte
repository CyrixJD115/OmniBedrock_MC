<script lang="ts">
  /** Manual backup form with live progress from the backup event stream. */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { backupRunning, latestBackupEvent } from '$stores/backup';
  import { serverStatus } from '$stores/index';
  import { formatBytes } from '$lib/utils';
  import IncludePicker from './IncludePicker.svelte';
  import FolderBrowser from './FolderBrowser.svelte';
  import { Play, HardDrive, FolderOpen, Zap, Info } from '@lucide/svelte';

  let { worlds, canCreate, oncreated }: { worlds: string[]; canCreate: boolean; oncreated: () => void } = $props();

  let world = $state('');
  let zipPrefix = $state('');
  let compression = $state<'deflate' | 'store'>('deflate');
  let dryRun = $state(false);
  let verify = $state(true);
  let exportFolder = $state('');
  let includeItems = $state<string[]>([]);
  let includeOpen = $state(false);
  let folderOpen = $state(false);
  let creating = $state(false);

  $effect(() => {
    if (!world || !worlds.includes(world)) world = worlds[0] ?? '';
  });

  const running = $derived($serverStatus?.status === 'running');
  const ev = $derived($latestBackupEvent as (Record<string, any> | null));

  async function run() {
    if (!world) return;
    creating = true;
    try {
      const r = await api.createBackup({
        world,
        tag: 'manual',
        zip_prefix: zipPrefix.trim() || undefined,
        compression,
        export_folder: exportFolder || undefined,
        full_backup: includeItems.length === 0,
        include_items: includeItems.length ? includeItems : undefined,
        dry_run: dryRun || undefined,
        verify,
      });
      addToast(r.message ?? (r.success ? 'Backup complete' : 'Backup failed'), r.success ? 'success' : 'error');
      oncreated();
    } catch (e: any) {
      addToast(`Backup failed: ${e.message.replace(/^API Error \d+: /, '')}`, 'error');
    }
    creating = false;
  }
</script>

<div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
  <div class="card lg:col-span-2 space-y-3">
    <h2 class="card-header">New backup</h2>
    <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
      <label class="block text-xs">
        <span class="block text-deep-400 uppercase tracking-wider mb-1">World</span>
        <select bind:value={world} class="input w-full text-xs py-1.5">
          {#each worlds as w (w)}<option value={w}>{w}</option>{/each}
        </select>
      </label>
      <label class="block text-xs">
        <span class="block text-deep-400 uppercase tracking-wider mb-1">File name prefix</span>
        <input bind:value={zipPrefix} class="input w-full text-xs py-1.5" placeholder="manual_backup" maxlength="48" />
      </label>
      <label class="block text-xs">
        <span class="block text-deep-400 uppercase tracking-wider mb-1">Compression</span>
        <select bind:value={compression} class="input w-full text-xs py-1.5">
          <option value="deflate">Compressed (smaller, slower)</option>
          <option value="store">Stored (larger, fastest)</option>
        </select>
      </label>
      <div class="text-xs">
        <span class="block text-deep-400 uppercase tracking-wider mb-1">Contents &amp; destination</span>
        <div class="flex gap-2">
          <button onclick={() => (includeOpen = true)} class="btn-ghost text-xs py-1.5 px-3 border border-deep-600/30 flex items-center gap-1">
            <HardDrive size={12} /> {includeItems.length ? `${includeItems.length} item(s)` : 'Whole world'}
          </button>
          <button onclick={() => (folderOpen = true)} class="btn-ghost text-xs py-1.5 px-3 border border-deep-600/30 flex items-center gap-1 min-w-0">
            <FolderOpen size={12} /> <span class="truncate">{exportFolder || 'Default folder'}</span>
          </button>
        </div>
      </div>
    </div>
    <div class="flex flex-wrap gap-x-5 gap-y-2 text-xs text-deep-300">
      <label class="flex items-center gap-2 cursor-pointer"><input type="checkbox" bind:checked={verify} class="accent-bedrock-500" /> Verify after writing</label>
      <label class="flex items-center gap-2 cursor-pointer"><input type="checkbox" bind:checked={dryRun} class="accent-bedrock-500" /> Dry run (hooks only, no archive)</label>
    </div>
    <p class="flex items-start gap-2 text-[11px] text-deep-400 leading-relaxed">
      {#if running}
        <Zap size={12} class="shrink-0 mt-0.5 text-yellow-300" />
        <span>The server is running: the panel sends <code>save hold</code>, waits for <code>save query</code>, copies each file only up to the length the server reports (consistent LevelDB snapshot), then always sends <code>save resume</code>. Players stay connected.</span>
      {:else}
        <Info size={12} class="shrink-0 mt-0.5" />
        <span>The server is stopped, so the world folder is copied as-is.</span>
      {/if}
    </p>
  </div>

  <div class="card space-y-3">
    <h2 class="card-header">Run</h2>
    <button onclick={run} disabled={!canCreate || creating || !world || $backupRunning}
            class="btn-primary w-full flex items-center justify-center gap-2 text-sm py-2 disabled:opacity-40">
      <Play size={15} /> {creating || $backupRunning ? 'Backing up…' : dryRun ? 'Dry run' : 'Back up now'}
    </button>
    {#if ev && ($backupRunning || creating)}
      <div class="text-xs space-y-1.5">
        <div class="mc-progress w-full"><span style="width: {ev.percent ?? 0}%"></span></div>
        <div class="flex justify-between text-deep-400">
          <span>{ev.message ?? ev.phase ?? ''}</span>
          <span class="font-mono">{ev.percent ?? 0}%</span>
        </div>
        {#if ev.total_bytes}
          <p class="text-[10px] text-deep-400 font-mono">{formatBytes(ev.done_bytes ?? 0)} / {formatBytes(ev.total_bytes)}</p>
        {/if}
      </div>
    {/if}
  </div>
</div>

<IncludePicker open={includeOpen} {world} selected={includeItems}
  onconfirm={(items) => { includeItems = items; includeOpen = false; }}
  oncancel={() => (includeOpen = false)} />
<FolderBrowser open={folderOpen} current={exportFolder}
  onconfirm={(path) => { exportFolder = path; folderOpen = false; }}
  oncancel={() => (folderOpen = false)} />
