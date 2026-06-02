<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { metrics } from '$stores/index';
  import { wsManager } from '$lib/websocket';
  import { Cpu, HardDrive, Activity } from '@lucide/svelte';

  let history = $state<{ time: string; cpu: number; mem: number; tps: number }[]>([]);

  onMount(() => {
    return wsManager.connect('/api/v1/performance/ws', (data: any) => {
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
  <div>
    <h1 class="text-lg font-bold text-white uppercase tracking-widest">Performance</h1>
    <div class="pixel-divider mt-2 w-40"></div>
  </div>

  <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">CPU Avg</span>
        <Cpu size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">{cpuAvg}%</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">Memory Avg</span>
        <HardDrive size={16} class="text-bedrock-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">{memAvg} MB</p>
    </div>
    <div class="card">
      <div class="flex items-center justify-between mb-2">
        <span class="text-deep-400 text-xs uppercase tracking-wider">TPS Avg</span>
        <Activity size={16} class="text-teal-400" />
      </div>
      <p class="text-xl font-bold text-white font-mono">{tpsAvg}</p>
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
            <tr><td colspan="4" class="text-center py-8 text-deep-500">Waiting for data...</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
