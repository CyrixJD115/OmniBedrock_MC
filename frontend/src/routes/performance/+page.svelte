<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { metrics } from '$stores/index';
  import { getInstance } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { activeInstance, hostInfo } from '$stores/instances';
  import { Cpu, HardDrive, Activity, ShieldCheck, ShieldAlert } from '@lucide/svelte';

  let history = $state<{ time: string; cpu: number; mem: number; tps: number }[]>([]);

  onMount(() => {
    const instance = getInstance() || 'default';
    const wsToken = `?instance=${encodeURIComponent(instance)}`;
    return wsManager.connect(`/api/v1/performance/ws${wsToken}`, (data: any) => {
      if (data.type !== 'metrics') return;
      metrics.set(data.data);
      const m = data.data;
      history.push({
        time: new Date(m.timestamp * 1000).toLocaleTimeString(),
        cpu: m.cpu_percent,
        mem: m.memory_mb,
        tps: m.tps,
      });
      if (history.length > 60) history.shift();
    });
  });

  let cpuAvg = $derived(history.length ? (history.reduce((a, b) => a + b.cpu, 0) / history.length).toFixed(1) : '--');
  let memAvg = $derived(history.length ? (history.reduce((a, b) => a + b.mem, 0) / history.length).toFixed(0) : '--');
  let tpsAvg = $derived(history.length ? (history.reduce((a, b) => a + b.tps, 0) / history.length).toFixed(1) : '--');
</script>

<div class="space-y-4">
  <div class="flex items-end justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Performance</h1>
      <div class="pixel-divider mt-2 w-40"></div>
    </div>
    {#if $activeInstance}
      <p class="text-xs text-deep-400 uppercase tracking-wider">{$activeInstance.name} · {$activeInstance.deployment}</p>
    {/if}
  </div>

  {#if $activeInstance}
    <div class="card flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
      <span class="flex items-center gap-1.5 {$activeInstance.enforcement === 'advisory' ? 'text-amber-400' : 'text-teal-400'}">
        {#if $activeInstance.enforcement === 'advisory'}<ShieldAlert size={14} />{:else}<ShieldCheck size={14} />{/if}
        {#if $activeInstance.enforcement === 'container'}Limits enforced by the container runtime{:else if $activeInstance.enforcement === 'cgroup'}Limits enforced by systemd cgroups{:else}Advisory limits — not enforced (native process){/if}
      </span>
      <span class="text-deep-400">Memory limit: <span class="font-mono text-strong">{$activeInstance.resources.memory_mb ? `${$activeInstance.resources.memory_mb} MB` : 'unlimited'}</span></span>
      <span class="text-deep-400">CPU limit: <span class="font-mono text-strong">{$activeInstance.resources.cpus ? `${$activeInstance.resources.cpus} cores` : 'unlimited'}</span></span>
      <span class="text-deep-400">Usage source: <span class="font-mono">{$metrics?.usage_source ?? '…'}</span></span>
    </div>
  {/if}

  {#if $hostInfo}
    <div class="card text-xs flex flex-wrap gap-x-6 gap-y-1">
      <span class="text-deep-400">Host memory: <span class="font-mono text-strong">{$hostInfo.memory_used_mb.toFixed(0)} / {$hostInfo.memory_total_mb.toFixed(0)} MB</span></span>
      <span class="text-deep-400">Cores: <span class="font-mono text-strong">{$hostInfo.cpu_count}</span></span>
      <span class="text-deep-400">Load (1m): <span class="font-mono text-strong">{$hostInfo.load_1.toFixed(2)}</span></span>
      <span class="text-deep-400">Allocated: <span class="font-mono text-strong">{$hostInfo.allocated.memory_mb} MB · {$hostInfo.allocated.cpus.toFixed(1)} CPU across {$hostInfo.allocated.instances} instances</span></span>
    </div>
  {/if}

  <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">CPU Avg</span>
        <Cpu size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">{cpuAvg}%</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Memory Avg</span>
        <HardDrive size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">{memAvg} MB</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">TPS Avg</span>
        <Activity size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-strong font-mono">{tpsAvg}</p>
    </div>
  </div>

  <div class="card">
    <h2 class="card-header">Real-Time (last 60)</h2>
    <div class="overflow-x-auto">
      <table class="w-full text-xs">
        <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
          <th class="text-left py-2 px-3 font-medium">Time</th>
          <th class="text-right py-2 px-3 font-medium">CPU</th>
          <th class="text-right py-2 px-3 font-medium">Memory</th>
          <th class="text-right py-2 px-3 font-medium">TPS</th>
        </tr></thead>
        <tbody>
          {#each [...history].reverse() as h}
            <tr class="border-b border-deep-700/20 text-deep-300">
              <td class="py-1 px-3 font-mono">{h.time}</td>
              <td class="py-1 px-3 text-right">{h.cpu.toFixed(1)}</td>
              <td class="py-1 px-3 text-right">{h.mem.toFixed(0)}</td>
              <td class="py-1 px-3 text-right">{h.tps.toFixed(1)}</td>
            </tr>
          {:else}
            <tr><td colspan="4" class="text-center py-8 text-deep-400">Waiting for data...</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
