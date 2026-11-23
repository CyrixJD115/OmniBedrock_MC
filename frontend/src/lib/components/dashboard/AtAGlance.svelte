<script lang="ts">
  /**
   * Dashboard summary tiles: players online, last backup (and the next
   * scheduled one), pack health from the compatibility checker, and the
   * running version. Each tile links to its page and only loads what the
   * user may see.
   */
  import { api } from '$lib/api/client';
  import { userPermissions } from '$stores/auth';
  import { serverStatus } from '$stores/index';
  import { compact } from '$lib/compat/format';
  import { Users, HardDrive, ShieldCheck, ShieldAlert, Tag } from '@lucide/svelte';

  let { instanceId }: { instanceId: string } = $props();

  let players = $state<{ online: string[]; max: number | null } | null>(null);
  let backup = $state<{ at: string | null; verified: boolean | null; next: string | null; enabled: boolean } | null>(null);
  let health = $state<{ bad: number; critical: number; events: number } | null>(null);

  const can = (p: string) => $userPermissions.includes(p);

  function ago(iso: string): string {
    const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
    if (mins < 1) return 'just now';
    if (mins < 60) return `${mins} min ago`;
    if (mins < 1440) return `${Math.round(mins / 60)} h ago`;
    return `${Math.round(mins / 1440)} d ago`;
  }
  function until(iso: string): string {
    const mins = Math.round((new Date(iso).getTime() - Date.now()) / 60_000);
    return mins <= 0 ? 'due now' : mins < 60 ? `in ${mins} min` : `in ${Math.round(mins / 60)} h`;
  }

  let epoch = 0;
  $effect(() => {
    instanceId;
    $serverStatus.status;
    const mine = ++epoch;
    if (can('PLAYERS_VIEW')) {
      api.knownPlayers()
        .then((k) => { if (mine === epoch) players = { online: k.players.filter((p) => p.online_since !== null).map((p) => p.name), max: k.max_players }; })
        .catch(() => undefined);
    }
    if (can('BACKUPS_VIEW')) {
      Promise.all([api.listBackups(), api.getSchedulerConfig().catch(() => null)])
        .then(([list, sched]) => {
          if (mine !== epoch) return;
          const newest = list.backups[0];
          backup = {
            at: newest?.modified ?? null,
            verified: newest?.meta?.verified ?? null,
            next: sched?.enabled ? sched.status?.next_run ?? null : null,
            enabled: !!sched?.enabled,
          };
        })
        .catch(() => undefined);
    }
    if (can('ADDONS_VIEW')) {
      api.getCompat()
        .then((r) => { if (mine === epoch) health = { bad: r.packs.filter((p) => p.status === 'error').length, critical: r.summary.critical, events: r.summary.events }; })
        .catch(() => undefined);
    }
  });

  const version = $derived(($serverStatus as any)?.version as string | undefined);
</script>

<div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4">
  {#if can('PLAYERS_VIEW')}
    <a href="/players" class="card !p-4 hover:border-bedrock-500/40 transition-colors block">
      <div class="flex items-center justify-between"><span class="text-[10px] uppercase tracking-wider text-deep-400">Players online</span><Users size={15} class="text-bedrock-400" /></div>
      <p class="mt-2 text-xl font-bold font-mono text-strong">{players ? players.online.length : '–'}{#if players?.max}<span class="text-deep-400 text-sm"> / {players.max}</span>{/if}</p>
      <p class="text-[11px] text-deep-400 truncate">{players?.online.length ? players.online.join(', ') : $serverStatus.status === 'running' ? 'Nobody online' : 'Server stopped'}</p>
    </a>
  {/if}
  {#if can('BACKUPS_VIEW')}
    <a href="/backups" class="card !p-4 hover:border-bedrock-500/40 transition-colors block">
      <div class="flex items-center justify-between"><span class="text-[10px] uppercase tracking-wider text-deep-400">Last backup</span><HardDrive size={15} class="text-bedrock-400" /></div>
      <p class="mt-2 text-xl font-bold text-strong {backup && !backup.at ? 'text-yellow-300' : ''}">{backup ? (backup.at ? ago(backup.at) : 'Never') : '–'}</p>
      <p class="text-[11px] text-deep-400 truncate">
        {#if backup?.verified === false}<span class="text-red-300">failed verification · </span>{:else if backup?.verified}<span class="text-teal-300">verified · </span>{/if}
        {backup?.enabled ? (backup.next ? `next ${until(backup.next)}` : 'scheduled') : 'no schedule'}
      </p>
    </a>
  {/if}
  {#if can('ADDONS_VIEW')}
    <a href="/addons" class="card !p-4 hover:border-bedrock-500/40 transition-colors block {health?.critical ? 'border-l-4 border-l-red-400' : ''}">
      <div class="flex items-center justify-between"><span class="text-[10px] uppercase tracking-wider text-deep-400">Pack health</span>
        {#if health && (health.bad || health.critical)}<ShieldAlert size={15} class="text-red-400" />{:else}<ShieldCheck size={15} class="text-teal-400" />{/if}</div>
      <p class="mt-2 text-xl font-bold text-strong">{health ? (health.bad ? `${health.bad} pack${health.bad === 1 ? '' : 's'} failing` : 'Healthy') : '–'}</p>
      <p class="text-[11px] text-deep-400 truncate">{health ? `${health.critical} critical · ${compact(health.events)} log events` : 'Checking…'}</p>
    </a>
  {/if}
  <div class="card !p-4">
    <div class="flex items-center justify-between"><span class="text-[10px] uppercase tracking-wider text-deep-400">Version</span><Tag size={15} class="text-bedrock-400" /></div>
    <p class="mt-2 text-sm font-bold font-mono text-strong truncate">{version ?? 'unknown'}</p>
    <p class="text-[11px] text-deep-400"><a href="/settings#server" class="hover:text-deep-200">Change version →</a></p>
  </div>
</div>
