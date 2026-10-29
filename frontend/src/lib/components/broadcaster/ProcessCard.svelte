<script lang="ts">
  /**
   * Broadcaster process card: live status fields (state, PID, uptime, java,
   * relay target, ping, dir, jar) and the start/stop/restart controls, plus
   * the external-broadcaster hint. Actions are delegated to the page.
   */
  import { Play, RotateCw, Square } from '@lucide/svelte';
  import type { BroadcasterStatus } from '$types/index';

  let { status, canManage, busy, onaction }: {
    status: BroadcasterStatus | null;
    canManage: boolean;
    busy: string;
    onaction: (kind: 'start' | 'stop' | 'restart' | 'takeover') => void;
  } = $props();

  function fmtUptime(s: number): string {
    if (s < 60) return `${Math.floor(s)}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
    return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  }
</script>

<div class="card">
  <h2 class="card-header">Process</h2>
  <div class="space-y-3">
    {#if status}
      <div class="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
        <span class="text-deep-400">State</span>
        <span class="text-strong capitalize">
          {status.status.status}{status.external && status.status.status !== 'running' ? ' (external)' : ''}
        </span>
        <span class="text-deep-400">PID</span><span class="font-mono text-strong">{status.status.pid ?? '—'}</span>
        <span class="text-deep-400">Uptime</span><span class="font-mono text-strong">{status.status.status === 'running' ? fmtUptime(status.status.uptime) : '—'}</span>
        <span class="text-deep-400">Java</span>
        <span class="font-mono text-strong truncate" title={status.java ?? ''}>{status.java ?? 'not detected'}</span>
        <span class="text-deep-400">Relay target</span>
        <span class="font-mono text-strong truncate" title={status.relay?.relay?.text ?? ''}>
          {status.relay?.relay?.text ?? 'auto (127.0.0.1)'}
          {#if status.relay?.relay?.public}<span class="text-red-400"> public!</span>{/if}
        </span>
        <span class="text-deep-400">Ping</span>
        <span class="font-mono {status.ping?.ok === false ? 'text-red-400' : status.ping?.ok === true ? 'text-teal-400' : 'text-strong'}">
          {status.ping?.ok === true ? 'ok' : status.ping?.ok === false ? 'failing' : '—'}
        </span>
        <span class="text-deep-400">Dir</span>
        <span class="font-mono text-deep-300 break-all" title={status.dir}>{status.dir}</span>
        <span class="text-deep-400">Jar</span>
        <span class="font-mono text-deep-300 break-all" title={status.jar}>{status.jar}</span>
      </div>
    {:else}
      <p class="text-xs text-deep-400">Loading…</p>
    {/if}
    {#if canManage}
      <div class="flex gap-2 pt-1">
        <button onclick={() => onaction('start')} disabled={!!busy || status?.status.status === 'running' || !!status?.external}
                class="btn-primary flex-1 flex items-center justify-center gap-1.5 text-xs">
          <Play size={13} /> Start
        </button>
        <button onclick={() => onaction('stop')} disabled={!!busy || (status?.status.status !== 'running' && !status?.external)}
                class="btn-ghost flex-1 flex items-center justify-center gap-1.5 text-xs">
          <Square size={13} /> Stop
        </button>
        <button onclick={() => onaction('restart')} disabled={!!busy || status?.status.status !== 'running'}
                class="btn-ghost flex items-center justify-center gap-1.5 text-xs px-3" title="Restart">
          <RotateCw size={13} />
        </button>
      </div>
      {#if status?.external && status.status.status !== 'running'}
        <p class="text-[10px] text-yellow-400 leading-snug">
          An external broadcaster (PID {status.external.pid}) is running — use the banner above
          to stop it or take over.
        </p>
      {/if}
    {/if}
  </div>
</div>
