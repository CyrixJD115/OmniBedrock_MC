<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { metrics } from '$stores/index';
  import { wsManager } from '$lib/websocket';
  import { api } from '$lib/api/client';
  import { BarChart3, Activity, Cpu, HardDrive } from '@lucide/svelte';

  let history = $state<{ time: string; cpu: number; mem: number; tps: number }[]>([]);
  let loading = $state(true);

  onMount(() => {
    const unsub = wsManager.connect('/api/v1/performance/ws', (data: any) => {
      if (data.type === 'metrics') {
        metrics.set(data.data);
        const m = data.data;
        history.push({
          time: new Date(m.timestamp * 1000).toLocaleTimeString(),
          cpu: m.cpu_percent,
          mem: m.memory_mb,
          tps: m.tps,
        });
        if (history.length > 60) history.shift();
      }
    });
    return unsub;
  });

  let cpuAvg = $derived(history.length ? (history.reduce((a, b) => a + b.cpu, 0) / history.length).toFixed(1) : '0');
  let memAvg = $derived(history.length ? (history.reduce((a, b) => a + b.mem, 0) / history.length).toFixed(0) : '0');
  let tpsAvg = $derived(history.length ? (history.reduce((a, b) => a + b.tps, 0) / history.length).toFixed(1) : '0');
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Performance</h1>
  </div>

  <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">CPU (avg)</span>
        <Cpu size={18} class="text-neon-green" />
      </div>
      <p class="text-2xl font-bold text-white">{cpuAvg}%</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">Memory (avg)</span>
        <HardDrive size={18} class="text-neon-purple" />
      </div>
      <p class="text-2xl font-bold text-white">{memAvg} MB</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-surface-400 text-sm">TPS (avg)</span>
        <Activity size={18} class="text-neon-cyan" />
      </div>
      <p class="text-2xl font-bold text-white">{tpsAvg}</p>
    </div>
  </div>

  <div class="card">
    <h2 class="card-header">Real-Time Metrics (last 60 samples)</h2>
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="text-surface-400 border-b border-surface-700">
            <th class="text-left py-2 px-3 font-medium">Time</th>
            <th class="text-right py-2 px-3 font-medium">CPU %</th>
            <th class="text-right py-2 px-3 font-medium">Memory MB</th>
            <th class="text-right py-2 px-3 font-medium">TPS</th>
          </tr>
        </thead>
        <tbody>
          {#each [...history].reverse() as h}
            <tr class="border-b border-surface-800 text-surface-300">
              <td class="py-1.5 px-3 text-xs font-mono">{h.time}</td>
              <td class="py-1.5 px-3 text-right">{h.cpu.toFixed(1)}</td>
              <td class="py-1.5 px-3 text-right">{h.mem.toFixed(0)}</td>
              <td class="py-1.5 px-3 text-right">{h.tps.toFixed(1)}</td>
            </tr>
          {:else}
            <tr>
              <td colspan="4" class="text-center py-8 text-surface-500">Waiting for metrics data...</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
