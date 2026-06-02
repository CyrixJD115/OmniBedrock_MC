<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { formatBytes } from '$lib/utils';
  import { HardDrive, Download, Trash2, Plus, RefreshCw } from '@lucide/svelte';

  let backups = $state<{ filename: string; world: string; size_bytes: number; modified: string }[]>([]);
  let worlds = $state<string[]>([]);
  let selectedWorld = $state('');
  let loading = $state(true);
  let creating = $state(false);
  let schedulerEnabled = $state(false);

  onMount(async () => {
    try {
      worlds = await api.listWorlds();
      if (worlds.length > 0) selectedWorld = worlds[0];
      const res = await api.listBackups();
      backups = res.backups;
      const sched = await api.getSchedulerConfig();
      schedulerEnabled = sched.enabled;
    } catch { /* ignore */ }
    loading = false;
  });

  async function loadBackups() {
    const res = await api.listBackups(selectedWorld || undefined);
    backups = res.backups;
  }

  async function createBackup() {
    creating = true;
    await api.createBackup(selectedWorld, 'manual');
    await loadBackups();
    creating = false;
  }

  async function deleteBackup(world: string, filename: string) {
    await api.deleteBackup(world, filename);
    await loadBackups();
  }

  async function downloadBackup(world: string, filename: string) {
    const token = (await import('$lib/api/client')).getToken();
    window.open(`/api/v1/backups/${world}/${filename}/download?token=${token}`, '_blank');
  }

  async function toggleScheduler() {
    schedulerEnabled = !schedulerEnabled;
    await api.updateScheduler({ enabled: schedulerEnabled, interval_minutes: 30, keep_count: 10 });
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Backups</h1>
    <div class="flex gap-2">
      <select bind:value={selectedWorld} onchange={loadBackups} class="input w-48">
        {#each worlds as w}
          <option value={w}>{w}</option>
        {/each}
      </select>
      <button onclick={createBackup} disabled={creating || !selectedWorld}
              class="btn-primary flex items-center gap-2">
        <Plus size={16} /> {creating ? 'Creating...' : 'Backup Now'}
      </button>
      <button onclick={loadBackups} class="btn-ghost p-2">
        <RefreshCw size={16} />
      </button>
    </div>
  </div>

  <div class="card">
    <div class="flex items-center justify-between mb-4">
      <h2 class="card-header mb-0">Backup Archive</h2>
      <label class="flex items-center gap-2 text-sm text-surface-400 cursor-pointer">
        <input type="checkbox" checked={schedulerEnabled} onchange={toggleScheduler}
               class="accent-bedrock-500" />
        Auto-backup (30 min)
      </label>
    </div>
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="text-surface-400 border-b border-surface-700">
            <th class="text-left py-2 px-3 font-medium">Filename</th>
            <th class="text-left py-2 px-3 font-medium">World</th>
            <th class="text-right py-2 px-3 font-medium">Size</th>
            <th class="text-right py-2 px-3 font-medium">Modified</th>
            <th class="text-right py-2 px-3 font-medium">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each backups as backup}
            <tr class="border-b border-surface-800 hover:bg-surface-800/50">
              <td class="py-2 px-3 font-mono text-sm">{backup.filename}</td>
              <td class="py-2 px-3">{backup.world}</td>
              <td class="py-2 px-3 text-right">{formatBytes(backup.size_bytes)}</td>
              <td class="py-2 px-3 text-right text-surface-400">{new Date(backup.modified).toLocaleString()}</td>
              <td class="py-2 px-3 text-right">
                <div class="flex justify-end gap-1">
                  <button onclick={() => downloadBackup(backup.world, backup.filename)}
                          class="btn-ghost p-1.5" title="Download">
                    <Download size={14} />
                  </button>
                  <button onclick={() => deleteBackup(backup.world, backup.filename)}
                          class="btn-ghost p-1.5 text-red-400 hover:text-red-300" title="Delete">
                    <Trash2 size={14} />
                  </button>
                </div>
              </td>
            </tr>
          {:else}
            <tr>
              <td colspan="5" class="text-center py-8 text-surface-500">No backups found</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
