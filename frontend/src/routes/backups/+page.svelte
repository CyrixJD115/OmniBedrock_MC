<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import { HardDrive, Download, Trash2, Plus, RefreshCw } from '@lucide/svelte';

  let backups = $state<{ filename: string; world: string; size_bytes: number; modified: string }[]>([]);
  let worlds = $state<string[]>([]);
  let selectedWorld = $state('');
  let loading = $state(true);
  let creating = $state(false);
  let schedEnabled = $state(false);

  onMount(async () => {
    try {
      worlds = await api.listWorlds();
      if (worlds.length) selectedWorld = worlds[0];
      const r = await api.listBackups();
      backups = r.backups;
      const s = await api.getSchedulerConfig();
      schedEnabled = s.enabled;
    } catch (e: any) { addToast(`Failed to load: ${e.message}`, 'error'); }
    loading = false;
  });

  async function reload() {
    try { const r = await api.listBackups(selectedWorld || undefined); backups = r.backups; } catch {}
  }

  async function create() {
    if (!selectedWorld) return;
    creating = true;
    try {
      await api.createBackup(selectedWorld, 'manual');
      addToast('Backup created', 'success');
      await reload();
    } catch (e: any) { addToast(`Backup failed: ${e.message}`, 'error'); }
    creating = false;
  }

  async function del(world: string, file: string) {
    try {
      await api.deleteBackup(world, file);
      await reload();
      addToast('Backup moved to trash', 'info', 6000, {
        label: 'Undo',
        callback: async () => {
          try {
            await api.restoreBackup(world, file);
            addToast('Backup restored', 'success');
            await reload();
          } catch (e: any) { addToast(`Restore failed: ${e.message}`, 'error'); }
        },
      });
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
  }

  function download(world: string, file: string) {
    const token = (() => { const m = /omb_token=([^;]+)/.exec(document.cookie); return ''; })();
    window.open(`/api/v1/backups/${world}/${file}/download`, '_blank');
  }

  async function toggleSched() {
    schedEnabled = !schedEnabled;
    try {
      await api.updateScheduler({ enabled: schedEnabled, interval_minutes: 30, keep_count: 10 });
      addToast(schedEnabled ? 'Auto-backup on' : 'Auto-backup off', 'success');
    } catch (e: any) { addToast(`Scheduler update failed: ${e.message}`, 'error'); schedEnabled = !schedEnabled; }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Backups</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex gap-2">
      <select bind:value={selectedWorld} onchange={reload} class="input w-36 text-xs py-1.5">
        {#each worlds as w}<option value={w}>{w}</option>{/each}
      </select>
      <button onclick={create} disabled={creating || !selectedWorld}
              class="btn-primary flex items-center gap-2 text-xs">
        <Plus size={14} /> {creating ? '...' : 'Backup'}
      </button>
      <button onclick={reload} class="btn-ghost p-2"><RefreshCw size={14} /></button>
    </div>
  </div>

  <div class="card">
    <div class="flex items-center justify-between mb-4">
      <h2 class="card-header mb-0">Archive</h2>
      <label class="flex items-center gap-2 text-xs text-deep-400 uppercase tracking-wider cursor-pointer select-none">
        <input type="checkbox" checked={schedEnabled} onchange={toggleSched} class="accent-bedrock-500" />
        Auto (30m)
      </label>
    </div>
    <div class="overflow-x-auto">
      <table class="w-full text-xs">
        <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
          <th class="text-left py-2 px-3 font-medium">File</th>
          <th class="text-right py-2 px-3 font-medium">Size</th>
          <th class="text-right py-2 px-3 font-medium">Modified</th>
          <th class="text-right py-2 px-3 font-medium"></th>
        </tr></thead>
        <tbody>
          {#each backups as b}
            <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
              <td class="py-1.5 px-3 font-mono">{b.filename}</td>
              <td class="py-1.5 px-3 text-right">{formatBytes(b.size_bytes)}</td>
              <td class="py-1.5 px-3 text-right text-deep-400">{new Date(b.modified).toLocaleString()}</td>
              <td class="py-1.5 px-3 text-right">
                <button onclick={() => download(b.world, b.filename)} class="btn-ghost p-1"><Download size={12} /></button>
                <button onclick={() => del(b.world, b.filename)} class="btn-ghost p-1 text-red-400"><Trash2 size={12} /></button>
              </td>
            </tr>
          {:else}
            <tr><td colspan="4" class="text-center py-8 text-deep-500">No backups</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
