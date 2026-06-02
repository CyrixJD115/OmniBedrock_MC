<script lang="ts">
  import { onMount } from 'svelte';
  import { serverStatus, metrics, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { formatUptime, formatBytes } from '$lib/utils';
  import { Power, Play, RotateCw, Terminal, Users, HardDrive, Globe, Activity } from '@lucide/svelte';

  let loading = $state(true);

  onMount(async () => {
    try {
      const status = await api.getServerStatus();
      serverStatus.set(status);
    } catch { /* ignore */ }
    loading = false;
  });

  async function handleAction(action: string) {
    const res = await api.serverAction(action);
    if (res.success) {
      setTimeout(async () => {
        const status = await api.getServerStatus();
        serverStatus.set(status);
      }, 2000);
    }
  }
</script>

<div class="space-y-6">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Dashboard</h1>
    <div class="flex gap-2">
      <button onclick={() => handleAction('start')} disabled={$isServerRunning}
              class="btn-primary flex items-center gap-2">
        <Play size={16} /> Start
      </button>
      <button onclick={() => handleAction('restart')} disabled={!$isServerRunning}
              class="btn-secondary flex items-center gap-2">
        <RotateCw size={16} /> Restart
      </button>
      <button onclick={() => handleAction('stop')} disabled={!$isServerRunning}
              class="btn-danger flex items-center gap-2">
        <Power size={16} /> Stop
      </button>
    </div>
  </div>

  <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">Server Status</span>
        <Activity size={18} class={$isServerRunning ? 'text-neon-green' : 'text-red-400'} />
      </div>
      <p class="text-2xl font-bold {$isServerRunning ? 'text-neon-green' : 'text-red-400'}">
        {$serverStatus.status.toUpperCase()}
      </p>
      {#if $serverStatus.pid}
        <p class="text-xs text-surface-500 mt-1">PID: {$serverStatus.pid}</p>
      {/if}
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">Uptime</span>
        <Terminal size={18} class="text-neon-cyan" />
      </div>
      <p class="text-2xl font-bold text-white">
        {formatUptime($serverStatus.uptime)}
      </p>
      {#if $serverStatus.version}
        <p class="text-xs text-surface-500 mt-1">v{$serverStatus.version}</p>
      {/if}
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">Players</span>
        <Users size={18} class="text-neon-purple" />
      </div>
      <p class="text-2xl font-bold text-white">--</p>
      <p class="text-xs text-surface-500 mt-1">RCON-based</p>
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">TPS</span>
        <Activity size={18} class="text-neon-green" />
      </div>
      <p class="text-2xl font-bold {$metrics?.tps && $metrics.tps >= 15 ? 'text-neon-green' : 'text-yellow-400'}">
        {$metrics?.tps?.toFixed(1) ?? '--'}
      </p>
    </div>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    <div class="card">
      <h2 class="card-header">Performance</h2>
      <div class="space-y-4">
        <div>
          <div class="flex justify-between text-sm mb-1">
            <span class="text-surface-400">CPU</span>
            <span class="text-white font-medium">{$metrics?.cpu_percent?.toFixed(1) ?? '0'}%</span>
          </div>
          <div class="h-2 bg-surface-800 rounded-full overflow-hidden">
            <div class="h-full bg-neon-green rounded-full transition-all duration-500"
                 style="width: {Math.min($metrics?.cpu_percent ?? 0, 100)}%"></div>
          </div>
        </div>
        <div>
          <div class="flex justify-between text-sm mb-1">
            <span class="text-surface-400">Memory</span>
            <span class="text-white font-medium">{$metrics?.memory_mb?.toFixed(0) ?? '0'} MB</span>
          </div>
          <div class="h-2 bg-surface-800 rounded-full overflow-hidden">
            <div class="h-full bg-neon-purple rounded-full transition-all duration-500"
                 style="width: {Math.min($metrics?.memory_percent ?? 0, 100)}%"></div>
          </div>
        </div>
      </div>
    </div>

    <div class="card">
      <h2 class="card-header">Quick Actions</h2>
      <div class="grid grid-cols-2 gap-3">
        <a href="/console" class="flex items-center gap-3 p-3 rounded-lg bg-surface-800 hover:bg-surface-700 transition-colors">
          <Terminal size={20} class="text-neon-cyan" />
          <span class="text-sm font-medium">Console</span>
        </a>
        <a href="/backups" class="flex items-center gap-3 p-3 rounded-lg bg-surface-800 hover:bg-surface-700 transition-colors">
          <HardDrive size={20} class="text-bedrock-400" />
          <span class="text-sm font-medium">Backups</span>
        </a>
        <a href="/players" class="flex items-center gap-3 p-3 rounded-lg bg-surface-800 hover:bg-surface-700 transition-colors">
          <Users size={20} class="text-neon-purple" />
          <span class="text-sm font-medium">Players</span>
        </a>
        <a href="/worlds" class="flex items-center gap-3 p-3 rounded-lg bg-surface-800 hover:bg-surface-700 transition-colors">
          <Globe size={20} class="text-blue-400" />
          <span class="text-sm font-medium">Worlds</span>
        </a>
      </div>
    </div>
  </div>
</div>
