<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import {
    backupEvents, backupRunning, latestBackupEvent, backupSettings,
    connectBackupWs, disconnectBackupWs
  } from '$stores/backup';
  import CommandEditor from '$lib/components/backups/CommandEditor.svelte';
  import IncludePicker from '$lib/components/backups/IncludePicker.svelte';
  import FolderBrowser from '$lib/components/backups/FolderBrowser.svelte';
  import type { CommandEntry } from '$types/index';
  import { HardDrive, Download, Trash2, Plus, RefreshCw, Play } from '@lucide/svelte';

  let activeTab = $state<'manual' | 'auto' | 'backups' | 'logs'>('manual');
  let tabs: Array<'manual' | 'auto' | 'backups' | 'logs'> = ['manual', 'auto', 'backups', 'logs'];
  let worlds = $state<string[]>([]);
  let loading = $state(true);

  // --- manual tab ---
  let selWorld = $state('');
  let zipPrefix = $state('');
  let compression = $state<'zip' | '7z'>('zip');
  let dryRun = $state(false);
  let exportFolder = $state('');
  let includePickerOpen = $state(false);
  let folderBrowserOpen = $state(false);
  let selIncludeItems = $state<string[]>([]);
  let creating = $state(false);

  // --- auto tab ---
  let schedEnabled = $state(false);
  let schedInterval = $state(30);
  let schedKeep = $state(10);
  let schedFull = $state(false);
  let schedCompression = $state<'zip' | '7z'>('zip');
  let schedWorlds = $state<string[]>([]);

  // --- backups tab ---
  let backups = $state<{ filename: string; world: string; size_bytes: number; modified: string }[]>([]);
  let selBkWorld = $state('');
  let trashItems = $state<any[]>([]);
  let showTrash = $state(false);

  // --- logs tab ---
  let logFilter = $state('');
  let logAutoScroll = $state(true);
  let logEl: HTMLDivElement;

  // --- pre/post command state ---
  let cmdBefore = $state<CommandEntry[]>([]);
  let cmdAfter = $state<CommandEntry[]>([]);

  onMount(async () => {
    try {
      worlds = await api.listWorlds();
      if (worlds.length) { selWorld = worlds[0]; selBkWorld = worlds[0]; }
      const s = await api.getBackupSettings();
      backupSettings.set(s);
      cmdBefore = s.pre_post?.before ?? [];
      cmdAfter = s.pre_post?.after ?? [];
      const sched = await api.getSchedulerConfig();
      schedEnabled = sched.enabled;
      schedInterval = sched.interval_minutes ?? 30;
      schedKeep = sched.keep_count ?? 10;
      schedFull = sched.full_backup ?? false;
      schedCompression = sched.compression === '7z' ? '7z' : 'zip';
      schedWorlds = sched.worlds ?? [];
      await loadBackups();
    } catch (e: any) { addToast(`Failed to load backups: ${e.message}`, 'error'); }
    loading = false;
    connectBackupWs();
  });

  onDestroy(() => {
    disconnectBackupWs();
  });

  // --- backups tab ---
  async function loadBackups() {
    try {
      const r = await api.listBackups(selBkWorld || undefined);
      backups = r.backups;
    } catch {}
  }

  async function delBackup(w: string, f: string) {
    try {
      await api.deleteBackup(w, f);
      await loadBackups();
      addToast('Backup moved to trash', 'info', 6000, {
        label: 'Undo',
        callback: async () => {
          try {
            await api.restoreBackup(w, f);
            addToast('Backup restored', 'success');
            await loadBackups();
          } catch (e: any) { addToast(`Restore failed: ${e.message}`, 'error'); }
        },
      });
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
  }

  function downloadBackup(w: string, f: string) {
    window.open(`/api/v1/backups/${w}/${f}/download`, '_blank');
  }

  async function loadTrash() {
    try { trashItems = await api.listTrash(selBkWorld || undefined); showTrash = true; } catch {}
  }

  // --- manual tab ---
  async function runBackup() {
    if (!selWorld) return;
    creating = true;
    try {
      await api.createBackup({
        world: selWorld,
        tag: 'manual',
        zip_prefix: zipPrefix || undefined,
        compression,
        export_folder: exportFolder || undefined,
        include_items: selIncludeItems.length > 0 ? selIncludeItems : undefined,
        dry_run: dryRun || undefined,
      });
      addToast('Backup completed', 'success');
      await loadBackups();
    } catch (e: any) { addToast(`Backup failed: ${e.message}`, 'error'); }
    creating = false;
  }

  // --- auto tab ---
  async function saveScheduler() {
    try {
      await api.updateScheduler({
        enabled: schedEnabled, interval_minutes: schedInterval, keep_count: schedKeep,
        full_backup: schedFull, compression: schedCompression, worlds: schedWorlds,
      });
      addToast('Scheduler updated', 'success');
    } catch (e: any) { addToast(`Scheduler update failed: ${e.message}`, 'error'); }
  }

  function toggleSchedWorld(w: string) {
    if (schedWorlds.includes(w)) {
      schedWorlds = schedWorlds.filter((x) => x !== w);
    } else {
      schedWorlds = [...schedWorlds, w];
    }
  }

  // --- settings save ---
  async function savePrePost() {
    try {
      await api.updateBackupSettings({ pre_post: { before: cmdBefore, after: cmdAfter } });
      addToast('Command settings saved', 'success');
    } catch (e: any) { addToast(`Failed to save: ${e.message}`, 'error'); }
  }

  // --- logs ---
  let filteredEvents = $derived(
    $backupEvents.filter((e) => !logFilter || e.type === logFilter || e.phase?.includes(logFilter))
  );

  $effect(() => {
    if (logAutoScroll && logEl && filteredEvents.length > 0) {
      requestAnimationFrame(() => { logEl.scrollTop = logEl.scrollHeight; });
    }
  });
</script>

<div class="space-y-4">
  <!-- header -->
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Backups</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex items-center gap-2">
      {#if $backupRunning}
        <span class="flex items-center gap-1 text-bedrock-400 text-xs uppercase tracking-wider">
          <span class="w-2 h-2 rounded-full bg-bedrock-500 animate-pulse"></span> Backup running...
        </span>
      {/if}
      <button onclick={() => connectBackupWs()} class="btn-ghost p-1.5" title="Reconnect WS"><RefreshCw size={13} /></button>
    </div>
  </div>

  <!-- tabs -->
  <div class="flex gap-1 border-b border-deep-600/30 pb-px">
    {#each tabs as tab}
      <button
        onclick={() => activeTab = tab}
        class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t
               {activeTab === tab
                 ? 'bg-deep-800/60 text-white border-b-2 border-bedrock-500'
                 : 'text-deep-400 hover:text-deep-200'}"
      >{tab === 'manual' ? 'Manual' : tab === 'auto' ? 'Automatic' : tab === 'backups' ? 'Backups' : 'Logs'}</button>
    {/each}
  </div>

  <!-- ======================== MANUAL TAB ======================== -->
  {#if activeTab === 'manual'}
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
      <div class="lg:col-span-2 space-y-4">
        <div class="card">
          <h2 class="card-header">Configuration</h2>
          <div class="grid grid-cols-2 gap-3">
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">World</label>
              <select bind:value={selWorld} class="input w-full text-xs py-1.5">
                {#each worlds as w}<option value={w}>{w}</option>{/each}
              </select>
            </div>
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Zip Prefix</label>
              <input bind:value={zipPrefix} class="input w-full text-xs py-1.5" placeholder="(auto)" />
            </div>
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Compression</label>
              <select bind:value={compression} class="input w-full text-xs py-1.5">
                <option value="zip">Zip</option>
                <option value="7z">7z</option>
              </select>
            </div>
            <div class="flex items-end gap-2">
              <button onclick={() => includePickerOpen = true} class="btn-ghost text-xs py-1.5 px-3 rounded border border-deep-600/30 flex items-center gap-1">
                <HardDrive size={13} /> Include ({selIncludeItems.length})
              </button>
              <button onclick={() => folderBrowserOpen = true} class="btn-ghost text-xs py-1.5 px-3 rounded border border-deep-600/30">
                Export Folder
              </button>
            </div>
          </div>
        </div>

        <div class="card">
          <h2 class="card-header">Pre / Post Commands</h2>
          <CommandEditor
            label=""
            before={cmdBefore}
            after={cmdAfter}
            onchange={(b, a) => { cmdBefore = b; cmdAfter = a; }}
          />
          <div class="mt-2">
            <button onclick={savePrePost} class="btn-ghost text-xs px-3 py-1 rounded uppercase tracking-wider border border-deep-600/30 text-deep-300 hover:text-deep-100">Save Commands</button>
          </div>
        </div>
      </div>

      <div class="space-y-4">
        <div class="card">
          <h2 class="card-header">Run Backup</h2>
          <label class="flex items-center gap-2 text-xs text-deep-300 mb-3 cursor-pointer">
            <input type="checkbox" bind:checked={dryRun} class="accent-bedrock-500" />
            Dry run (validate only)
          </label>
          <button
            onclick={runBackup}
            disabled={creating || !selWorld || $backupRunning}
            class="btn-primary w-full flex items-center justify-center gap-2 text-sm py-2 disabled:opacity-40"
          >
            <Play size={15} /> {creating ? 'Running...' : $backupRunning ? 'Busy' : 'Run Backup'}
          </button>
        </div>

        {#if $latestBackupEvent}
          <div class="card">
            <h2 class="card-header">Progress</h2>
            <div class="text-xs space-y-1">
              <div class="flex justify-between text-deep-400">
                <span>Phase</span>
                <span class="text-deep-200">{$latestBackupEvent.phase ?? 'idle'}</span>
              </div>
              {#if $latestBackupEvent.message}
                <p class="text-deep-300">{$latestBackupEvent.message}</p>
              {/if}
              {#if $latestBackupEvent.percent !== undefined}
                <div class="w-full h-2 bg-deep-700 rounded overflow-hidden mt-2">
                  <div class="h-full bg-bedrock-500 transition-all duration-300 rounded"
                       style="width: {$latestBackupEvent.percent}%"></div>
                </div>
                <span class="text-deep-400">{$latestBackupEvent.percent}%</span>
              {/if}
              {#if $latestBackupEvent.stream === 'stderr' && $latestBackupEvent.line}
                <p class="text-yellow-400 font-mono text-xs bg-deep-800/60 rounded p-1 mt-2">{$latestBackupEvent.line}</p>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    </div>

  <!-- ======================== AUTOMATIC TAB ======================== -->
  {:else if activeTab === 'auto'}
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <div class="card">
        <h2 class="card-header">Schedule</h2>
        <div class="space-y-3">
          <label class="flex items-center gap-2 text-xs cursor-pointer select-none">
            <input type="checkbox" bind:checked={schedEnabled} class="accent-bedrock-500" />
            <span class="text-deep-200 uppercase tracking-wider font-semibold">Enabled</span>
          </label>
          <div class="grid grid-cols-3 gap-3">
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Interval (min)</label>
              <input type="number" bind:value={schedInterval} min="5" max="1440" class="input w-full text-xs py-1.5" />
            </div>
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Keep Count</label>
              <input type="number" bind:value={schedKeep} min="1" max="100" class="input w-full text-xs py-1.5" />
            </div>
            <div>
              <label class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Compression</label>
              <select bind:value={schedCompression} class="input w-full text-xs py-1.5">
                <option value="zip">Zip</option>
                <option value="7z">7z</option>
              </select>
            </div>
          </div>
          <label class="flex items-center gap-2 text-xs cursor-pointer select-none">
            <input type="checkbox" bind:checked={schedFull} class="accent-bedrock-500" />
            <span class="text-deep-200">Full backup (world folder)</span>
          </label>
          <div>
            <label class="block text-deep-400 text-xs uppercase tracking-wider mb-2">Worlds</label>
            <div class="flex flex-wrap gap-2">
              {#each worlds as w}
                <label class="flex items-center gap-1 text-xs cursor-pointer px-2 py-1 rounded bg-deep-800/40 border border-deep-600/20 hover:bg-deep-700/40">
                  <input type="checkbox" checked={schedWorlds.includes(w)} onchange={() => toggleSchedWorld(w)} class="accent-bedrock-500" />
                  {w}
                </label>
              {/each}
            </div>
          </div>
          <button onclick={saveScheduler} class="btn-ghost text-xs px-3 py-1 rounded uppercase tracking-wider border border-deep-600/30 text-deep-300 hover:text-deep-100">
            Save Schedule
          </button>
        </div>
      </div>

      <div class="card">
        <h2 class="card-header">Auto Backup Status</h2>
        <div class="text-xs space-y-2">
          <div class="flex justify-between">
            <span class="text-deep-400">State</span>
            <span class={schedEnabled ? 'text-green-400' : 'text-deep-400'}>
              {schedEnabled ? 'Active' : 'Disabled'}
            </span>
          </div>
          <div class="flex justify-between">
            <span class="text-deep-400">Interval</span>
            <span class="text-deep-200">{schedInterval} minutes</span>
          </div>
          <div class="flex justify-between">
            <span class="text-deep-400">Retention</span>
            <span class="text-deep-200">{schedKeep} backups</span>
          </div>
          {#if $backupRunning}
            <p class="flex items-center gap-1 text-bedrock-400 mt-2">
              <span class="w-2 h-2 rounded-full bg-bedrock-500 animate-pulse"></span> Backup in progress...
            </p>
          {/if}
        </div>
      </div>
    </div>

  <!-- ======================== BACKUPS TAB ======================== -->
  {:else if activeTab === 'backups'}
    <div class="flex items-center gap-2 mb-3">
      <select bind:value={selBkWorld} onchange={loadBackups} class="input w-36 text-xs py-1.5">
        <option value="">All worlds</option>
        {#each worlds as w}<option value={w}>{w}</option>{/each}
      </select>
      <button onclick={loadBackups} class="btn-ghost p-2"><RefreshCw size={14} /></button>
      <button onclick={loadTrash} class="btn-ghost text-xs px-3 py-1.5 rounded border border-deep-600/30 flex items-center gap-1 ml-auto">
        <Trash2 size={13} /> Trash
      </button>
    </div>

    <div class="card">
      <div class="overflow-x-auto">
        <table class="w-full text-xs">
          <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
            <th class="text-left py-2 px-3 font-medium">File</th>
            <th class="text-left py-2 px-3 font-medium">World</th>
            <th class="text-right py-2 px-3 font-medium">Size</th>
            <th class="text-right py-2 px-3 font-medium">Modified</th>
            <th class="text-right py-2 px-3 font-medium"></th>
          </tr></thead>
          <tbody>
            {#each backups as b}
              <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
                <td class="py-1.5 px-3 font-mono">{b.filename}</td>
                <td class="py-1.5 px-3 text-deep-400">{b.world}</td>
                <td class="py-1.5 px-3 text-right">{formatBytes(b.size_bytes)}</td>
                <td class="py-1.5 px-3 text-right text-deep-400">{new Date(b.modified).toLocaleString()}</td>
                <td class="py-1.5 px-3 text-right">
                  <button onclick={() => downloadBackup(b.world, b.filename)} class="btn-ghost p-1"><Download size={12} /></button>
                  <button onclick={() => delBackup(b.world, b.filename)} class="btn-ghost p-1 text-red-400"><Trash2 size={12} /></button>
                </td>
              </tr>
            {:else}
              <tr><td colspan="5" class="text-center py-8 text-deep-500">No backups</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>

  <!-- ======================== LOGS TAB ======================== -->
  {:else if activeTab === 'logs'}
    <div class="card">
      <div class="flex items-center gap-2 mb-3">
        <input
          bind:value={logFilter}
          class="input text-xs py-1.5 w-32"
          placeholder="Filter type..."
        />
        <select bind:value={logFilter} class="input text-xs py-1.5 w-28">
          <option value="">All</option>
          <option value="hello">Hello</option>
          <option value="status">Status</option>
          <option value="progress">Progress</option>
          <option value="output">Output</option>
          <option value="done">Done</option>
          <option value="error">Error</option>
        </select>
        <label class="flex items-center gap-1 text-xs text-deep-400 cursor-pointer select-none ml-auto">
          <input type="checkbox" bind:checked={logAutoScroll} class="accent-bedrock-500" />
          Auto-scroll
        </label>
        <button onclick={() => backupEvents.set([])} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Clear</button>
      </div>

      <div bind:this={logEl} class="h-80 overflow-y-auto bg-deep-900/60 border border-deep-600/30 rounded p-3 font-mono text-xs space-y-1">
        {#each filteredEvents as ev}
          <div class="flex gap-2">
            <span class="text-deep-500 shrink-0 w-10 text-right">
              {#if ev.type === 'hello'}<span class="text-green-400">HELO</span>
              {:else if ev.type === 'status'}<span class="text-blue-400">STAT</span>
              {:else if ev.type === 'progress'}<span class="text-bedrock-400">PROG</span>
              {:else if ev.type === 'output'}<span class="text-deep-300">OUT</span>
              {:else if ev.type === 'done'}<span class="text-green-400">DONE</span>
              {:else if ev.type === 'error'}<span class="text-red-400">ERR</span>
              {:else}<span class="text-deep-500">{ev.type.toUpperCase().slice(0,4)}</span>
              {/if}
            </span>
            {#if ev.percent !== undefined}
              <span class="text-deep-500 shrink-0 w-8">[{ev.percent}%]</span>
            {/if}
            <span class="text-deep-200">
              {ev.phase ?? ev.message ?? ev.line ?? ''}
            </span>
          </div>
        {:else}
          <p class="text-deep-500 text-center py-8">No events yet. Run a backup to see progress here.</p>
        {/each}
      </div>
    </div>
  {/if}
</div>

<!-- modals -->
<IncludePicker
  open={includePickerOpen}
  world={selWorld}
  selected={selIncludeItems}
  onconfirm={(items) => { selIncludeItems = items; includePickerOpen = false; }}
  oncancel={() => { includePickerOpen = false; }}
/>
<FolderBrowser
  open={folderBrowserOpen}
  current={exportFolder}
  onconfirm={(path) => { exportFolder = path; folderBrowserOpen = false; }}
  oncancel={() => { folderBrowserOpen = false; }}
/>

{#if showTrash}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) showTrash = false; }}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-lg shadow-block-lg shadow-black/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-3">Trash — {selBkWorld || 'All'}</h2>
      <div class="max-h-64 overflow-y-auto space-y-1">
        {#each trashItems as t}
          <div class="flex justify-between items-center p-2 rounded bg-deep-800/40 border border-deep-600/20 text-xs">
            <span class="font-mono text-deep-200">{t.filename ?? t.name}</span>
            <div class="flex gap-1">
              <button
                onclick={async () => { await api.restoreBackup(t.world, t.filename ?? t.name); addToast('Restored', 'success'); await loadTrash(); }}
                class="btn-ghost text-xs px-2 py-0.5 rounded border border-deep-600/30 text-green-400">Restore</button>
              <button onclick={() => { showTrash = false; }} class="btn-ghost text-xs px-2 py-0.5 rounded border border-deep-600/30 text-deep-400">Close</button>
            </div>
          </div>
        {:else}
          <p class="text-deep-500 text-center py-8 text-xs">Trash is empty.</p>
        {/each}
      </div>
      <div class="flex justify-end mt-4">
        <button onclick={() => { showTrash = false; }} class="btn-secondary text-xs">Close</button>
      </div>
    </div>
  </div>
{/if}
