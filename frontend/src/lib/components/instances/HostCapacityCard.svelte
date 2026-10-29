<script lang="ts">
  /**
   * Host capacity card: memory/CPU/disk/allocated totals, deployment
   * backend availability (systemd/docker), and the linger hint. Memory and
   * disk percentages are computed by the page from the same host snapshot.
   */
  import { ServerCog } from '@lucide/svelte';
  import type { HostInfo } from '$types/index';
  import { deploymentBadge, fmtMb } from '$lib/instances/format';

  let { hostInfo, memoryPercent, diskPercent }: {
    hostInfo: HostInfo;
    memoryPercent: number;
    diskPercent: number;
  } = $props();
</script>

<div class="card">
  <h2 class="card-header flex items-center gap-2"><ServerCog size={14} /> Host Capacity</h2>
  <div class="grid grid-cols-2 md:grid-cols-4 gap-4 text-xs">
    <div>
      <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Memory used</p>
      <p class="font-mono text-strong">{fmtMb(hostInfo.memory_used_mb)} / {fmtMb(hostInfo.memory_total_mb)}</p>
      <div class="h-1.5 bg-deep-700 mt-1.5">
        <div class="h-full {memoryPercent > 90 ? 'bg-red-500' : memoryPercent > 75 ? 'bg-amber-500' : 'bg-teal-500'}" style="width: {memoryPercent}%"></div>
      </div>
    </div>
    <div>
      <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">CPU cores</p>
      <p class="font-mono text-strong">{hostInfo.cpu_count} <span class="text-deep-400">(load {hostInfo.load_1.toFixed(2)})</span></p>
    </div>
    {#if hostInfo.disk_total_mb}
      <div>
        <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Disk used</p>
        <p class="font-mono text-strong">{fmtMb(hostInfo.disk_used_mb ?? 0)} / {fmtMb(hostInfo.disk_total_mb)}</p>
        <div class="h-1.5 bg-deep-700 mt-1.5">
          <div class="h-full {diskPercent > 90 ? 'bg-red-500' : diskPercent > 75 ? 'bg-amber-500' : 'bg-teal-500'}" style="width: {diskPercent}%"></div>
        </div>
      </div>
    {/if}
    <div>
      <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Allocated to instances</p>
      <p class="font-mono text-strong">{fmtMb(hostInfo.allocated.memory_mb)} · {hostInfo.allocated.cpus.toFixed(1)} CPU · {hostInfo.allocated.instances} instances</p>
      <p class="text-[10px] text-deep-400 mt-0.5">host reserve {fmtMb(hostInfo.reserve_mb)} (monitoring-only budget)</p>
    </div>
    <div>
      <p class="text-deep-400 uppercase tracking-wider text-[10px] mb-1">Enforcement backends</p>
      <div class="flex flex-wrap gap-1.5">
        <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('native')}">native</span>
        <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('systemd')} {hostInfo.capabilities.systemd ? '' : 'opacity-40'}">
          systemd {#if !hostInfo.capabilities.systemd}(unavailable){/if}
        </span>
        <span class="px-1.5 py-0.5 border text-[10px] uppercase {deploymentBadge('docker')} {hostInfo.capabilities.docker ? '' : 'opacity-40'}">
          docker {#if !hostInfo.capabilities.docker}(unavailable){/if}
        </span>
      </div>
      {#if hostInfo.capabilities.systemd && !hostInfo.capabilities.linger}
        <p class="text-[10px] text-amber-400 mt-1">
          linger off — systemd units stop at logout; run <code>loginctl enable-linger $USER</code> for boot-persistent servers
        </p>
      {/if}
    </div>
  </div>
</div>
