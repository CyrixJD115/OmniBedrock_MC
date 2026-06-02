<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Users, Ban, Shield, ShieldOff } from '@lucide/svelte';

  let players = $state<{ name: string }[]>([]);
  let count = $state(0);
  let loading = $state(true);

  onMount(async () => {
    try {
      const r = await api.listPlayers();
      players = r.players || [];
      count = r.count || 0;
    } catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  });

  async function act(action: string, target: string) {
    try {
      await api.playerAction(action, target);
      addToast(`${action} ${target}`, 'success');
    } catch (e: any) { addToast(`${action} failed: ${e.message}`, 'error'); }
  }

  async function refresh() {
    loading = true;
    try { const r = await api.listPlayers(); players = r.players || []; count = r.count || 0; }
    catch (e: any) { addToast(`Failed: ${e.message}`, 'error'); }
    loading = false;
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Players</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <button onclick={refresh} class="btn-secondary text-xs">Refresh</button>
  </div>

  <div class="card">
    <div class="flex items-center gap-2 mb-4">
      <Users size={14} class="text-bedrock-400" />
      <span class="text-xs text-deep-400 uppercase tracking-wider">{count} online</span>
    </div>
    <div class="overflow-x-auto">
      <table class="w-full text-xs">
        <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
          <th class="text-left py-2 px-3 font-medium">Name</th>
          <th class="text-right py-2 px-3 font-medium">Actions</th>
        </tr></thead>
        <tbody>
          {#each players as p}
            <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
              <td class="py-1.5 px-3 font-medium">{p.name}</td>
              <td class="py-1.5 px-3 text-right">
                <button onclick={() => act('kick', p.name)} class="btn-ghost p-1 text-yellow-400" title="Kick"><Ban size={12} /></button>
                <button onclick={() => act('ban', p.name)} class="btn-ghost p-1 text-red-400" title="Ban"><ShieldOff size={12} /></button>
                <button onclick={() => act('op', p.name)} class="btn-ghost p-1 text-teal-400" title="OP"><Shield size={12} /></button>
                <button onclick={() => act('deop', p.name)} class="btn-ghost p-1 text-deep-400" title="DeOP"><ShieldOff size={12} /></button>
              </td>
            </tr>
          {:else}
            <tr><td colspan="2" class="text-center py-8 text-deep-500">{loading ? 'Loading...' : 'No players online'}</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
