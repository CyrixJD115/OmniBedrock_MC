<script lang="ts">
  /**
   * Live resource usage for the active instance. The layout owns the metrics
   * WebSocket (one per instance) and feeds `$metrics`; this page seeds an
   * hour of server-side history on open / instance switch and appends live
   * samples, so charts survive navigation and never mix instances.
   */
  import { untrack } from 'svelte';
  import { api } from '$lib/api/client';
  import { metrics } from '$stores/index';
  import { activeInstance, hostInfo } from '$stores/instances';
  import MetricChart from '$lib/components/performance/MetricChart.svelte';
  import { Cpu, HardDrive, Activity, ShieldCheck, ShieldAlert, Server, Gauge } from '@lucide/svelte';

  type Point = { t: number; cpu: number; mem: number; tps: number | null };
  const RANGES = [5, 15, 60] as const;
  let range = $state<(typeof RANGES)[number]>(15);
  let samples = $state<Point[]>([]);
  let loadingHistory = $state(true);

  let epoch = 0;
  $effect(() => {
    $activeInstance?.id;
    const mine = ++epoch;
    samples = [];
    loadingHistory = true;
    api.getMetricsHistory(60)
      .then((r) => {
        if (mine !== epoch) return;
        const seeded = r.history.map(([t, cpu, mem, tps]) => ({ t, cpu, mem, tps }));
        // Keep live samples that arrived while the request was in flight.
        const last = seeded.length ? seeded[seeded.length - 1].t : 0;
        samples = [...seeded, ...samples.filter((s) => s.t > last)];
      })
      .catch(() => undefined)
      .finally(() => { if (mine === epoch) loadingHistory = false; });
  });

  // Append live samples from the layout's metrics stream.
  $effect(() => {
    const m = $metrics;
    if (!m || m.status !== 'running' || typeof m.timestamp !== 'number') return;
    untrack(() => {
      const last = samples[samples.length - 1];
      if (last && m.timestamp <= last.t) return;
      const next = [...samples, { t: m.timestamp, cpu: m.cpu_percent, mem: m.memory_mb, tps: m.tps ?? null }];
      const cutoff = m.timestamp - 3600;
      samples = next[0].t < cutoff ? next.filter((s) => s.t >= cutoff) : next;
    });
  });

  const now = $derived(samples.length ? samples[samples.length - 1].t : Date.now() / 1000);
  const view = $derived(samples.filter((s) => s.t >= now - range * 60));
  const cpuPts = $derived(view.map((s) => ({ t: s.t, v: s.cpu })));
  const memPts = $derived(view.map((s) => ({ t: s.t, v: s.mem })));
  const tpsPts = $derived(view.map((s) => ({ t: s.t, v: s.tps })));
  const hasTps = $derived(view.some((s) => s.tps !== null));
  const running = $derived($metrics?.status === 'running');
  const memLimit = $derived($activeInstance?.resources.memory_mb ?? null);
  const cpuLimit = $derived($activeInstance?.resources.cpus ?? null);
  const cpuCap = $derived(cpuLimit ? cpuLimit * 100 : ($hostInfo?.cpu_count ?? 1) * 100);
  const hostMemPct = $derived($hostInfo ? (100 * $hostInfo.memory_used_mb) / Math.max(1, $hostInfo.memory_total_mb) : 0);
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-end justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Performance</h1>
      <div class="pixel-divider mt-2 w-40"></div>
    </div>
    <div class="flex items-center gap-3">
      {#if $activeInstance}
        <p class="text-xs text-deep-400 uppercase tracking-wider">{$activeInstance.name} · {$activeInstance.deployment}</p>
      {/if}
      <div class="flex mc-slot" role="group" aria-label="Time range">
        {#each RANGES as r (r)}
          <button class="text-[11px] px-2.5 py-1 font-mono {range === r ? 'bg-bedrock-500/20 text-strong' : 'text-deep-400 hover:text-deep-200'}"
                  onclick={() => (range = r)} aria-pressed={range === r}>{r === 60 ? '1h' : `${r}m`}</button>
        {/each}
      </div>
    </div>
  </div>

  <!-- current values -->
  <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
    <div class="card !p-3">
      <div class="flex items-center justify-between"><span class="text-[10px] text-deep-400 uppercase tracking-wider">Status</span><Server size={14} class="text-bedrock-400" /></div>
      <p class="mt-2 text-lg font-bold uppercase {running ? 'text-teal-300' : 'text-deep-300'}">{$metrics?.status ?? '…'}</p>
      <p class="text-[10px] text-deep-400 font-mono">{$metrics?.usage_source ?? ''}</p>
    </div>
    <div class="card !p-3">
      <div class="flex items-center justify-between"><span class="text-[10px] text-deep-400 uppercase tracking-wider">CPU</span><Cpu size={14} class="text-teal-400" /></div>
      <p class="mt-2 text-lg font-bold font-mono text-strong">{running ? `${($metrics?.cpu_percent ?? 0).toFixed(1)}%` : '—'}</p>
      <p class="text-[10px] text-deep-400">of {cpuCap}% ({cpuLimit ? `${cpuLimit} core limit` : `${$hostInfo?.cpu_count ?? '?'} cores`})</p>
    </div>
    <div class="card !p-3">
      <div class="flex items-center justify-between"><span class="text-[10px] text-deep-400 uppercase tracking-wider">Memory</span><HardDrive size={14} class="text-bedrock-400" /></div>
      <p class="mt-2 text-lg font-bold font-mono text-strong">{running ? `${($metrics?.memory_mb ?? 0).toFixed(0)} MB` : '—'}</p>
      <div class="mt-1 h-1.5 bg-deep-800/60 overflow-hidden">
        <div class="h-full bg-bedrock-500 transition-all duration-500" style="width: {Math.min(100, $metrics?.memory_percent ?? 0)}%"></div>
      </div>
      <p class="text-[10px] text-deep-400 mt-1">{memLimit ? `limit ${memLimit} MB` : 'of host memory'} · {($metrics?.memory_percent ?? 0).toFixed(1)}%</p>
    </div>
    <div class="card !p-3">
      <div class="flex items-center justify-between"><span class="text-[10px] text-deep-400 uppercase tracking-wider">TPS</span><Gauge size={14} class="text-teal-400" /></div>
      <p class="mt-2 text-lg font-bold font-mono text-strong">{$metrics?.tps != null ? $metrics.tps.toFixed(1) : '—'}</p>
      <p class="text-[10px] text-deep-400">{$metrics?.tps != null ? 'reported by the server' : running ? 'not reported by this server' : 'server stopped'}</p>
    </div>
  </div>

  <!-- charts -->
  {#if loadingHistory && samples.length === 0}
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">{#each [0, 1] as j (j)}<div class="card h-52 animate-pulse"></div>{/each}</div>
  {:else if samples.length === 0 && !running}
    <div class="card text-center py-10 text-xs text-deep-400">
      <Activity size={20} class="mx-auto mb-2 text-deep-500" />
      The server is stopped. Usage history is recorded while it runs (last hour kept by the panel).
    </div>
  {:else}
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <MetricChart points={cpuPts} label="CPU usage" unit="%" color="teal" />
      <MetricChart points={memPts} label="Memory" unit=" MB" digits={0} color="bedrock" max={memLimit} />
      {#if hasTps}
        <MetricChart points={tpsPts} label="Ticks per second" color="yellow" max={20} />
      {/if}
    </div>
  {/if}

  <!-- limits & host -->
  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    {#if $activeInstance}
      <div class="card text-xs space-y-2">
        <h2 class="card-header">Limits</h2>
        <p class="flex items-center gap-1.5 {$activeInstance.enforcement === 'advisory' ? 'text-amber-400' : 'text-teal-400'}">
          {#if $activeInstance.enforcement === 'advisory'}<ShieldAlert size={14} />{:else}<ShieldCheck size={14} />{/if}
          {#if $activeInstance.enforcement === 'container'}Enforced by the container runtime{:else if $activeInstance.enforcement === 'cgroup'}Enforced by systemd cgroups{:else}Advisory only: a native process is not limited. Use systemd or docker to enforce.{/if}
        </p>
        <p class="text-deep-400">Memory limit <span class="font-mono text-strong">{memLimit ? `${memLimit} MB` : 'unlimited'}</span> · CPU limit <span class="font-mono text-strong">{cpuLimit ? `${cpuLimit} cores` : 'unlimited'}</span></p>
      </div>
    {/if}
    {#if $hostInfo}
      <div class="card text-xs space-y-2">
        <h2 class="card-header">Host</h2>
        <div class="flex items-center gap-2">
          <span class="text-deep-400 w-16">Memory</span>
          <div class="flex-1 h-1.5 bg-deep-800/60 overflow-hidden"><div class="h-full {hostMemPct > 90 ? 'bg-red-400' : 'bg-teal-500'}" style="width: {hostMemPct}%"></div></div>
          <span class="font-mono text-strong">{($hostInfo.memory_used_mb / 1024).toFixed(1)} / {($hostInfo.memory_total_mb / 1024).toFixed(1)} GB</span>
        </div>
        <p class="text-deep-400">{$hostInfo.cpu_count} cores · load (1m) <span class="font-mono text-strong">{$hostInfo.load_1.toFixed(2)}</span>
          · allocated <span class="font-mono text-strong">{$hostInfo.allocated.memory_mb} MB / {$hostInfo.allocated.cpus.toFixed(1)} CPU</span> across {$hostInfo.allocated.instances} instances</p>
      </div>
    {/if}
  </div>
</div>
