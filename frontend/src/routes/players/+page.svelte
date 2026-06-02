<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { Users, Ban, UserCheck, Shield, ShieldOff } from '@lucide/svelte';

  let players = $state<{ name: string }[]>([]);
  let count = $state(0);
  let loading = $state(true);
  let actionFeedback = $state('');

  onMount(async () => {
    try {
      const res = await api.listPlayers();
      players = res.players;
      count = res.count;
    } catch { /* ignore */ }
    loading = false;
  });

  async function playerAction(action: string, target: string) {
    await api.playerAction(action, target);
    actionFeedback = `${action} ${target}`;
    setTimeout(() => actionFeedback = '', 3000);
  }

  async function refresh() {
    loading = true;
    const res = await api.listPlayers();
    players = res.players;
    count = res.count;
    loading = false;
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Players</h1>
    <button onclick={refresh} class="btn-secondary">Refresh</button>
  </div>

  {#if actionFeedback}
    <div class="bg-bedrock-600/20 border border-bedrock-500/30 text-bedrock-300 px-4 py-2 rounded-lg text-sm">
      Sent: {actionFeedback}
    </div>
  {/if}

  <div class="card">
    <div class="flex items-center gap-2 mb-4">
      <Users size={16} class="text-neon-purple" />
      <span class="text-sm text-surface-400">{count} player{count !== 1 ? 's' : ''} online</span>
    </div>

    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="text-surface-400 border-b border-surface-700">
            <th class="text-left py-2 px-3 font-medium">Name</th>
            <th class="text-right py-2 px-3 font-medium">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each players as player}
            <tr class="border-b border-surface-800 hover:bg-surface-800/50">
              <td class="py-2 px-3 font-medium">{player.name}</td>
              <td class="py-2 px-3 text-right">
                <div class="flex justify-end gap-1">
                  <button onclick={() => playerAction('kick', player.name)}
                          class="btn-ghost p-1.5 text-yellow-400" title="Kick">
                    <Ban size={14} />
                  </button>
                  <button onclick={() => playerAction('ban', player.name)}
                          class="btn-ghost p-1.5 text-red-400" title="Ban">
                    <ShieldOff size={14} />
                  </button>
                  <button onclick={() => playerAction('op', player.name)}
                          class="btn-ghost p-1.5 text-neon-green" title="OP">
                    <Shield size={14} />
                  </button>
                  <button onclick={() => playerAction('deop', player.name)}
                          class="btn-ghost p-1.5 text-surface-400" title="DeOP">
                    <UserCheck size={14} />
                  </button>
                </div>
              </td>
            </tr>
          {:else}
            <tr>
              <td colspan="2" class="text-center py-8 text-surface-500">
                {#if loading}
                  Loading...
                {:else}
                  No players online. Start the server to see connected players.
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
