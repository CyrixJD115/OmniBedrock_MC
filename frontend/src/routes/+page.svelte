<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { serverStatus, metrics, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { Play, Square, RotateCw, Terminal, Users, HardDrive, Globe, Cpu, Clock, Activity, TriangleAlert, Radio } from '@lucide/svelte';
  import type { BroadcasterStatus } from '$types/index';

  let loading = $state(true);
  let acting = $state<string | null>(null);
  let broadcaster = $state<BroadcasterStatus | null>(null);
  let bcPoll: ReturnType<typeof setInterval> | undefined;

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
    // Surface a broadcaster that cannot reach its ping target (Bug 4: the
    // jar only logs an ERROR every 30s, nothing else tells the user).
    const pollBc = async () => {
      try {
        broadcaster = await api.getBroadcasterStatus();
      } catch { /* transient */ }
    };
    void pollBc();
    bcPoll = setInterval(pollBc, 10000);
  });

  onDestroy(() => {
    if (bcPoll) clearInterval(bcPoll);
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
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Dashboard</h1>
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

  {#if broadcaster?.ping?.ok === false}
    <div class="card border-yellow-500/40">
      <div class="flex items-start gap-3">
        <Radio size={18} class="text-yellow-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-yellow-300 uppercase tracking-wider">
            Broadcaster cannot reach the server ping target
          </p>
          <p>
            The broadcaster for <span class="text-strong">{$activeInstance?.name ?? $activeInstance?.id ?? 'default'}</span> cannot ping
            <span class="font-mono text-strong">{broadcaster.ping.target ?? 'its configured target'}</span>,
            so the Xbox session advertises stale player/world values. Check that the server is
            running and that <span class="font-mono">session.remote-address</span> points at this
            host's server-port (127.0.0.1 for a co-located broadcaster).
          </p>
        </div>
      </div>
    </div>
  {/if}
  {#if broadcaster?.relay && (broadcaster.relay.relay?.public || broadcaster.relay.legacy?.public)}
    <div class="card border-red-500/40">
      <div class="flex items-start gap-3">
        <TriangleAlert size={18} class="text-red-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-red-300 uppercase tracking-wider">
            Broadcaster relay target uses a public IP
          </p>
          <p>
            The broadcaster runs on the same host as the server, so its relay target must be
            <span class="font-mono text-strong">127.0.0.1</span> — a public IP breaks every
            friends-list join (hairpin NAT). Fix it on the
            <a href="/broadcaster" class="text-bedrock-400 hover:underline">Broadcaster page</a>.
          </p>
        </div>
      </div>
    </div>
  {/if}

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
        <p class="text-deep-400 text-xs mt-1 font-mono">PID {$serverStatus.pid}</p>
      {/if}
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Uptime</span>
        <Clock size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">
        {$serverStatus.uptime > 0 ? formatUptime($serverStatus.uptime) : '--:--:--'}
      </p>
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">CPU</span>
        <Cpu size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">{$metrics?.cpu_percent?.toFixed(1) ?? '--'}</p>
      <div class="mt-2 h-1.5 bg-deep-900 rounded overflow-hidden">
        <div class="h-full bg-teal-500 transition-all duration-500" style="width: {Math.min($metrics?.cpu_percent ?? 0, 100)}%"></div>
      </div>
    </div>

    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Memory</span>
        <Activity size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">{$metrics?.memory_mb ? $metrics.memory_mb.toFixed(0) + ' MB' : '--'}</p>
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


