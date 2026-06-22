<script lang="ts">
  import { onMount } from 'svelte';
  import { serverStatus, metrics, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { Play, Square, RotateCw, Terminal, Users, HardDrive, Globe, Cpu, Clock, Activity } from '@lucide/svelte';

  let loading = $state(true);
  let acting = $state<string | null>(null);

  function formatUptime(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    return `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  }

  onMount(async () => {
    try {
      const s = await api.getServerStatus();
      serverStatus.set(s);
    } catch {
      addToast('Could not reach backend', 'error');
    }
    loading = false;
  });

  async function act(action: string) {
    acting = action;
    try {
      const res = await api.serverAction(action);
      addToast(res.message, res.success ? 'success' : 'error');
      if (res.success) {
        setTimeout(async () => {
          try {
            const s = await api.getServerStatus();
            serverStatus.set(s);
          } catch { /* ignore */ }
        }, 1500);
      }
    } catch (e: any) {
      addToast(`Action failed: ${e.message}`, 'error');
    }
    acting = null;
  }
</script>

<div class="space-y-6">
  <!-- Header -->
  <div class="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Dashboard</h1>
      <div class="pixel-divider mt-2 w-48"></div>
    </div>
    {#if $userPermissions.includes('SERVER_MANAGE')}
      <div class="flex gap-2">
        <button onclick={() => act('start')} disabled={$isServerRunning || acting !== null}
                class="btn-success flex items-center gap-2 text-xs">
          <Play size={14} /> {acting === 'start' ? 'Starting...' : 'Start'}
        </button>
        <button onclick={() => act('restart')} disabled={!$isServerRunning || acting !== null}
                class="btn-secondary flex items-center gap-2 text-xs">
          <RotateCw size={14} /> {acting === 'restart' ? 'Restarting...' : 'Restart'}
        </button>
        <button onclick={() => act('stop')} disabled={!$isServerRunning || acting !== null}
                class="btn-danger flex items-center gap-2 text-xs">
          <Square size={14} /> {acting === 'stop' ? 'Stopping...' : 'Stop'}
        </button>
      </div>
    {/if}
  </div>

  <!-- Status Cards -->
  <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Status</span>
        <span class="status-dot {$serverStatus.status}"></span>
      </div>
      <p class="text-xl font-bold {$isServerRunning ? 'text-teal-400' : 'text-red-400'} uppercase tracking-wider">
        {$serverStatus.status}
      </p>
      {#if $serverStatus.pid}
        <p class="text-deep-500 text-xs mt-1 font-mono">PID {$serverStatus.pid}</p>
      {/if}
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Uptime</span>
        <Clock size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">
        {$serverStatus.uptime > 0 ? formatUptime($serverStatus.uptime) : '--:--:--'}
      </p>
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">CPU</span>
        <Cpu size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">{$metrics?.cpu_percent?.toFixed(1) ?? '--'}</p>
      <div class="mt-2 h-1.5 bg-deep-900 rounded overflow-hidden">
        <div class="h-full bg-teal-500 transition-all duration-500" style="width: {Math.min($metrics?.cpu_percent ?? 0, 100)}%"></div>
      </div>
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Memory</span>
        <Activity size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">{$metrics?.memory_mb ? $metrics.memory_mb.toFixed(0) + ' MB' : '--'}</p>
      <div class="mt-2 h-1.5 bg-deep-900 rounded overflow-hidden">
        <div class="h-full bg-bedrock-500 transition-all duration-500" style="width: {Math.min($metrics?.memory_percent ?? 0, 100)}%"></div>
      </div>
    </div>
  </div>

  <!-- Quick Actions -->
  <div class="card">
    <h2 class="card-header">Quick Actions</h2>
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      <a href="/console" class="flex items-center gap-3 p-3 border border-deep-600/30 hover:border-bedrock-500/30 bg-deep-900/50 hover:bg-deep-800 transition-all">
        <Terminal size={18} class="text-bedrock-400" />
        <span class="text-xs uppercase tracking-wider font-semibold">Console</span>
      </a>
      <a href="/backups" class="flex items-center gap-3 p-3 border border-deep-600/30 hover:border-bedrock-500/30 bg-deep-900/50 hover:bg-deep-800 transition-all">
        <HardDrive size={18} class="text-teal-400" />
        <span class="text-xs uppercase tracking-wider font-semibold">Backups</span>
      </a>
      <a href="/players" class="flex items-center gap-3 p-3 border border-deep-600/30 hover:border-bedrock-500/30 bg-deep-900/50 hover:bg-deep-800 transition-all">
        <Users size={18} class="text-bedrock-400" />
        <span class="text-xs uppercase tracking-wider font-semibold">Players</span>
      </a>
      <a href="/worlds" class="flex items-center gap-3 p-3 border border-deep-600/30 hover:border-bedrock-500/30 bg-deep-900/50 hover:bg-deep-800 transition-all">
        <Globe size={18} class="text-teal-400" />
        <span class="text-xs uppercase tracking-wider font-semibold">Worlds</span>
      </a>
    </div>
  </div>
</div>


