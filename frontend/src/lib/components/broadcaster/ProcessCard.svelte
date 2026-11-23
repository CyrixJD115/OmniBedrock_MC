<script lang="ts">
  /**
   * Broadcaster process card: live status fields (state, PID, uptime, java,
   * relay target, ping, dir, jar) and the start/stop/restart controls, plus
   * the external-broadcaster hint. Actions are delegated to the page.
   */
  import { Play, RotateCw, Square, ArrowUpCircle } from '@lucide/svelte';
  import type { BroadcasterStatus, BroadcasterJarInfo } from '$types/index';

  let { status, canManage, busy, onaction, jarInfo = null, onupdatejar }: {
    status: BroadcasterStatus | null;
    canManage: boolean;
    busy: string;
    onaction: (kind: 'start' | 'stop' | 'restart' | 'takeover') => void;
    jarInfo?: BroadcasterJarInfo | null;
    onupdatejar?: () => void;
  } = $props();

  const jarText = $derived.by(() => {
    if (!jarInfo) return '—';
    const current = jarInfo.installed_build ?? jarInfo.running_build;
    const base = current ? `build ${current}` : 'unknown build';
    const bedrock = jarInfo.running_bedrock ? ` · Bedrock ${jarInfo.running_bedrock}` : '';
    const pending = jarInfo.installed_build && jarInfo.running_build && jarInfo.installed_build !== jarInfo.running_build
      ? ' (restart to run it)' : '';
    return base + bedrock + pending;
  });

  // The query verdict, in words: which transport answered and what the
  // server reported, or why there is no verdict (never a bare "failing"
  // while the game server is simply stopped or booting).
  const ping = $derived(status?.ping);
  const pingText = $derived.by(() => {
    if (!status || status.status.status !== 'running') return '—';
    if (!status.relay?.query_server) return 'disabled (query-server: false)';
    if (ping?.server_running === false) return 'game server offline';
    if (ping?.ok === true) {
      const via = ping.protocol === 'nethernet' ? 'NetherNet' : ping.protocol === 'raknet' ? 'RakNet' : 'ok';
      const s = ping.server;
      const players = s && s.players != null ? ` · ${s.players}/${s.max_players ?? '?'}` : '';
      const version = s?.version ? ` · ${s.version}` : '';
      return `ok via ${via}${version}${players}`;
    }
    if (ping?.ok === false) return 'not answering';
    if ((ping?.consecutive_failures ?? 0) > 0) return 'retrying…';
    return 'checking…';
  });
  const pingClass = $derived(
    ping?.ok === false && ping?.server_running !== false ? 'text-red-400'
      : ping?.ok === true ? 'text-teal-400' : 'text-deep-300',
  );
  const pingTitle = $derived(
    [ping?.target ? `target ${ping.target}` : '', ping?.latency_ms != null ? `${ping.latency_ms} ms` : '',
     ping?.transport ? `transport=${ping.transport}` : '', ping?.error ?? ''].filter(Boolean).join(' · '),
  );

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
        <span class="text-deep-400">Server query</span>
        <span class="font-mono truncate {pingClass}" title={pingTitle}>{pingText}</span>
        <span class="text-deep-400">Jar build</span>
        <span class="font-mono text-strong flex items-center gap-2 min-w-0">
          <span class="truncate" title={jarInfo?.latest ? `latest release: build ${jarInfo.latest.tag} (${jarInfo.latest.published_at.slice(0, 10)})` : (jarInfo?.latest_error ?? '')}>{jarText}</span>
          {#if jarInfo?.update_available && jarInfo.latest}
            <span class="badge-warn !text-[9px] !py-0 shrink-0">{jarInfo.latest.tag} available</span>
          {/if}
        </span>
        <span class="text-deep-400">Dir</span>
        <span class="font-mono text-deep-300 truncate min-w-0" style="direction: rtl; text-align: left" title={status.dir}><bdi>{status.dir}</bdi></span>
        <span class="text-deep-400">Jar</span>
        <span class="font-mono text-deep-300 truncate min-w-0" style="direction: rtl; text-align: left" title={status.jar}><bdi>{status.jar}</bdi></span>
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
      {#if jarInfo?.update_available && jarInfo.latest && onupdatejar}
        <button onclick={onupdatejar} disabled={!!busy}
                class="btn-secondary w-full flex items-center justify-center gap-1.5 text-xs"
                title="Download the latest MCXboxBroadcastStandalone.jar from GitHub (SHA-256 verified; the old jar is kept as .bak)">
          <ArrowUpCircle size={13} /> {busy === 'jar' ? 'Downloading…' : `Update jar to build ${jarInfo.latest.tag}`}
        </button>
      {/if}
      {#if status?.external && status.status.status !== 'running'}
        <p class="text-[10px] text-yellow-400 leading-snug">
          An external broadcaster (PID {status.external.pid}) is running — use the banner above
          to stop it or take over.
        </p>
      {/if}
    {/if}
  </div>
</div>
