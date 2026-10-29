<script lang="ts">
  /**
   * Broadcaster status banners: jar missing, externally-owned broadcaster,
   * public IP in the relay target, and query-ping failures. Renders nothing
   * when the matching condition is absent; lifecycle actions are delegated
   * to the page, which owns the broadcaster state.
   */
  import { ExternalLink, Radio, RotateCw, Square, TriangleAlert } from '@lucide/svelte';
  import type { BroadcasterStatus } from '$types/index';

  let { status, canManage, busy, onaction }: {
    status: BroadcasterStatus | null;
    canManage: boolean;
    busy: string;
    onaction: (kind: 'start' | 'stop' | 'restart' | 'takeover') => void;
  } = $props();
</script>

{#if status && !status.jar_exists}
  <div class="card border-red-500/40">
    <div class="flex items-start gap-3">
      <Radio size={18} class="text-red-400 mt-0.5 shrink-0" />
      <div class="text-xs text-deep-300 space-y-1">
        <p class="font-bold text-red-300 uppercase tracking-wider">Broadcaster jar not found</p>
        <p class="font-mono break-all">{status.jar}</p>
        <p>Place <span class="font-mono text-bedrock-300">MCXboxBroadcastStandalone.jar</span> there (download it from the
          <a href="https://github.com/MCXboxBroadcast/Broadcaster/releases" target="_blank" rel="noreferrer" class="text-bedrock-400 hover:underline inline-flex items-center gap-1">
            releases page <ExternalLink size={10} /></a>) or set <span class="font-mono">BROADCASTER_JAR</span>.</p>
      </div>
    </div>
  </div>
{/if}

{#if status?.external && status.status.status !== 'running'}
  <div class="card border-yellow-500/40">
    <div class="flex items-start justify-between gap-3 flex-wrap">
      <div class="flex items-start gap-3">
        <Radio size={18} class="text-yellow-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-yellow-300 uppercase tracking-wider">
            Running (external){status.external.orphaned ? ' — left over from a previous panel run' : ''}
          </p>
          <p>
            A broadcaster java is already running for this instance (PID
            <span class="font-mono text-strong">{status.external.pid}</span>) but the panel does not
            own it. Starting a second one would make both fight over the same Xbox session.
          </p>
        </div>
      </div>
      {#if canManage}
        <div class="flex gap-2">
          <button onclick={() => onaction('stop')} disabled={!!busy}
                  class="btn-ghost flex items-center gap-1.5 text-xs">
            <Square size={13} /> {busy === 'stop' ? 'Stopping…' : 'Stop external'}
          </button>
          <button onclick={() => onaction('takeover')} disabled={!!busy}
                  class="btn-primary flex items-center gap-1.5 text-xs">
            <RotateCw size={13} /> {busy === 'takeover' ? 'Taking over…' : 'Take over'}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if status?.relay && (status.relay.relay?.public || status.relay.legacy?.public)}
  <div class="card border-red-500/40">
    <div class="flex items-start gap-3">
      <TriangleAlert size={18} class="text-red-400 mt-0.5 shrink-0" />
      <div class="text-xs text-deep-300 space-y-1">
        <p class="font-bold text-red-300 uppercase tracking-wider">Public IP in the relay target</p>
        <p>
          The broadcaster runs on the same host as the server, so the relay target must be
          <span class="font-mono text-strong">127.0.0.1</span>. A public IP there loops through the
          router (hairpin NAT is refused) and every friends-list join fails with
          <span class="font-mono">"Failed to relay offer … connection refused"</span>
          (client-side InitialConnection-81).
        </p>
        <p class="font-mono break-all text-deep-400">
          {#if status.relay.relay?.public}session.remote-address: {status.relay.relay.text}{/if}
          {#if status.relay.relay?.public && status.relay.legacy?.public} · {/if}
          {#if status.relay.legacy?.public}session-info.ip: {status.relay.legacy.text} (legacy){/if}
        </p>
      </div>
    </div>
  </div>
{/if}

{#if status?.ping?.ok === false}
  <div class="card border-yellow-500/40">
    <div class="flex items-start gap-3">
      <TriangleAlert size={18} class="text-yellow-400 mt-0.5 shrink-0" />
      <div class="text-xs text-deep-300 space-y-1">
        <p class="font-bold text-yellow-300 uppercase tracking-wider">
          Broadcaster cannot reach the server ping target
        </p>
        <p>
          The query ping against <span class="font-mono text-strong">{status.ping.target ?? '—'}</span>
          fails, so the session advertises stale player/world values
          (<span class="font-mono">"Failed to ping server, falling back to config values"</span> in the console).
          Check that the server is running and that the relay target points at this host's server-port.
        </p>
        {#if status.ping.failing_since}
          <p class="text-deep-400">failing since {new Date(status.ping.failing_since * 1000).toLocaleTimeString()}</p>
        {/if}
      </div>
    </div>
  </div>
{/if}
