<script lang="ts">
  import { onMount } from 'svelte';
  import { api, getToken } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Search, RotateCw, Filter } from '@lucide/svelte';

  let entries = $state<any[]>([]);
  let loading = $state(true);
  let filterUser = $state('');
  let filterAction = $state('');
  let filterCategory = $state('');

  onMount(async () => {
    await loadEntries();
  });

  async function loadEntries() {
    loading = true;
    try {
      const params = new URLSearchParams();
      if (filterUser) params.set('username', filterUser);
      if (filterAction) params.set('action', filterAction);
      if (filterCategory) params.set('category', filterCategory);
      const res = await fetch(`/api/v1/audit/?${params}`, {
        headers: { Authorization: `Bearer ${getToken()}` },
      });
      entries = await res.json();
    } catch (e: any) {
      addToast(`Failed to load audit: ${e.message}`, 'error');
    }
    loading = false;
  }

  function categoryColor(cat: string): string {
    const colors: Record<string, string> = {
      server: 'text-teal-400',
      player: 'text-bedrock-400',
      file: 'text-yellow-400',
      config: 'text-purple-400',
      auth: 'text-red-400',
      backup: 'text-green-400',
    };
    return colors[cat] || 'text-deep-300';
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Audit Log</h1>
      <div class="pixel-divider mt-2 w-28"></div>
    </div>
    <button onclick={loadEntries} class="btn-ghost p-2"><RotateCw size={14} /></button>
  </div>

  <div class="card">
    <div class="flex flex-wrap gap-2 mb-4">
      <div class="relative flex-1 min-w-[120px]">
        <Search size={13} class="absolute left-3 top-1/2 -translate-y-1/2 text-deep-400" />
        <input type="text" bind:value={filterUser} placeholder="Filter user"
               class="input pl-8 py-1.5 text-xs w-full" />
      </div>
      <input type="text" bind:value={filterAction} placeholder="Filter action"
             class="input py-1.5 text-xs w-40" />
      <select bind:value={filterCategory} class="input py-1.5 text-xs w-32">
        <option value="">All categories</option>
        <option value="server">Server</option>
        <option value="player">Player</option>
        <option value="file">File</option>
        <option value="config">Config</option>
        <option value="auth">Auth</option>
        <option value="backup">Backup</option>
      </select>
      <button onclick={loadEntries} class="btn-primary text-xs py-1.5">Search</button>
    </div>

    <div class="overflow-x-auto">
      <table class="w-full text-xs">
        <thead>
          <tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
            <th class="text-left py-2 px-3 font-medium">Time</th>
            <th class="text-left py-2 px-3 font-medium">User</th>
            <th class="text-left py-2 px-3 font-medium">Category</th>
            <th class="text-left py-2 px-3 font-medium">Action</th>
            <th class="text-left py-2 px-3 font-medium">Detail</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as entry}
            <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
              <td class="py-1.5 px-3 font-mono text-deep-400 whitespace-nowrap">
                {new Date(entry.timestamp).toLocaleString()}
              </td>
              <td class="py-1.5 px-3 font-medium">{entry.username}</td>
              <td class="py-1.5 px-3">
                <span class={"badge " + categoryColor(entry.category)}>{entry.category}</span>
              </td>
              <td class="py-1.5 px-3 font-mono">{entry.action}</td>
              <td class="py-1.5 px-3 text-deep-300 max-w-xs truncate">{entry.detail}</td>
            </tr>
          {:else}
            <tr><td colspan="5" class="text-center py-8 text-deep-500">No audit entries</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
