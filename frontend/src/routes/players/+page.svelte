<script lang="ts">
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import type { KnownPlayer, PlayerAction, PlayersOverview } from '$types/players';
  import PlayerTable from '$lib/components/players/PlayerTable.svelte';
  import PlayerActionDialog from '$lib/components/players/PlayerActionDialog.svelte';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import { Users, RotateCw, Search, ListChecks, Crown, Ban, History, Plus, X, Loader2 } from '@lucide/svelte';

  type Tab = 'online' | 'all' | 'allowlist' | 'operators' | 'bans';
  let tab = $state<Tab>('online');
  let data = $state<PlayersOverview | null>(null);
  let loading = $state(true);
  let query = $state('');
  let newName = $state('');
  let dialog = $state<{ action: PlayerAction; player: KnownPlayer } | null>(null);
  let confirm = $state<{ action: PlayerAction; target: string; title: string; message: string } | null>(null);
  let busy = $state(false);

  const can = $derived({
    kick: $userPermissions.includes('PLAYERS_KICK'),
    ban: $userPermissions.includes('PLAYERS_BAN'),
    op: $userPermissions.includes('PLAYERS_OP'),
  });

  let epoch = 0;
  async function load(quiet = false) {
    const mine = ++epoch;
    if (!quiet) loading = true;
    try {
      // /players reconciles the online set with `list` before the overview reads it.
      await api.listPlayers().catch(() => undefined);
      const r = await api.knownPlayers();
      if (mine === epoch) data = r;
    } catch (e: any) {
      if (mine === epoch && !quiet) addToast(`Failed to load players: ${e.message}`, 'error');
    } finally {
      if (mine === epoch) loading = false;
    }
  }

  // Reload on instance switch; poll quietly while the server runs.
  $effect(() => {
    $activeInstance?.id;
    data = null;
    void load();
    const t = setInterval(() => { if (data?.running && !document.hidden) void load(true); }, 20_000);
    return () => clearInterval(t);
  });

  const online = $derived(data?.players.filter((p) => p.online_since !== null) ?? []);
  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = tab === 'online' ? online : data?.players ?? [];
    return q ? list.filter((p) => p.name.toLowerCase().includes(q) || (p.xuid ?? '').includes(q)) : list;
  });

  async function run(action: PlayerAction, target: string, extra: { reason?: string; level?: string; xuid?: string } = {}) {
    busy = true;
    try {
      const r = await api.playerAction(action, target, extra);
      addToast(r.message, 'success');
      dialog = null;
      await load(true);
    } catch (e: any) {
      addToast(e.message.replace(/^API Error \d+: /, ''), 'error');
    } finally {
      busy = false;
    }
  }

  function onaction(action: PlayerAction, p: KnownPlayer) {
    if (action === 'kick' || action === 'ban' || action === 'tell' || action === 'permission') {
      dialog = { action, player: p };
    } else if (action === 'forget') {
      confirm = { action, target: p.name, title: `Forget ${p.name}?`, message: 'Removes their join history (sessions, playtime, xuid). Allowlist and permissions are not touched.' };
    } else {
      void run(action, p.name, { xuid: p.xuid ?? undefined });
    }
  }

  function addAllowlist(e: Event) {
    e.preventDefault();
    const name = newName.trim();
    if (!name) return;
    newName = '';
    void run('allowlist_add', name);
  }

  const tabs = $derived([
    { id: 'online' as const, label: 'Online', icon: Users, count: online.length },
    { id: 'all' as const, label: 'All players', icon: History, count: data?.players.length ?? 0 },
    { id: 'allowlist' as const, label: 'Allowlist', icon: ListChecks, count: data?.allowlist.length ?? 0 },
    { id: 'operators' as const, label: 'Permissions', icon: Crown, count: data?.operators.length ?? 0 },
    ...(data?.bans_supported ? [{ id: 'bans' as const, label: 'Bans', icon: Ban, count: data?.bans.length ?? 0 }] : []),
  ]);
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Players</h1>
      <div class="pixel-divider mt-2 w-24"></div>
    </div>
    <div class="flex items-center gap-3 text-xs">
      {#if data}
        <span class="text-deep-400">
          <span class="font-mono text-strong">{online.length}</span>{#if data.max_players} / {data.max_players}{/if} online
          {#if !data.running}<span class="badge-stopped !text-[9px] !py-0.5 ml-2">server stopped</span>{/if}
          {#if data.allowlist_enabled}<span class="badge-info !text-[9px] !py-0.5 ml-1">allowlist on</span>{/if}
        </span>
      {/if}
      <button onclick={() => load()} class="btn-secondary text-xs py-1.5 px-3 flex items-center gap-1.5" disabled={loading}>
        {#if loading}<Loader2 size={13} class="animate-spin" />{:else}<RotateCw size={13} />{/if} Refresh
      </button>
    </div>
  </div>

  <div class="flex flex-wrap gap-1 border-b border-deep-600/30 pb-px" role="tablist">
    {#each tabs as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}
              class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t flex items-center gap-1.5
                     {tab === t.id ? 'bg-deep-800/60 text-strong border-b-2 border-bedrock-500' : 'text-deep-400 hover:text-deep-200'}">
        <t.icon size={12} /> {t.label} <span class="font-normal text-deep-400">({t.count})</span>
      </button>
    {/each}
  </div>

  {#if !data && loading}
    <div class="card space-y-2">{#each [0, 1, 2] as j (j)}<div class="h-10 bg-deep-800/40 rounded animate-pulse"></div>{/each}</div>
  {:else if data}
    {#if tab === 'online' || tab === 'all'}
      <div class="card">
        <div class="relative mb-3 max-w-sm">
          <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400" />
          <input class="input w-full text-xs py-1.5 pl-7" placeholder="Search name or xuid…" bind:value={query} />
        </div>
        <PlayerTable players={filtered} running={data.running} bansSupported={data.bans_supported} {can} {onaction}
          empty={tab === 'online' ? (data.running ? 'Nobody is online' : 'The server is stopped') : 'No player has joined since tracking started'} />
        {#if tab === 'all'}
          <p class="mt-3 text-[10px] text-deep-400">History is recorded from the console as players join and leave (players.json in the instance folder).</p>
        {/if}
      </div>
    {:else if tab === 'allowlist'}
      <div class="card space-y-3">
        <p class="text-xs text-deep-300">
          {#if data.allowlist_enabled}The allowlist is <b class="text-teal-300">on</b>: only these players can join.{:else}The allowlist is <b>off</b> (allow-list=false in Properties): entries here have no effect until it is enabled.{/if}
          {#if !data.running} Changes are written to allowlist.json and apply on next start.{/if}
        </p>
        {#if can.op}
          <form class="flex gap-2 max-w-md" onsubmit={addAllowlist}>
            <input class="input flex-1 text-xs py-1.5" placeholder="Gamertag" bind:value={newName} maxlength="64" />
            <button class="btn-primary text-xs px-3 flex items-center gap-1" disabled={busy || !newName.trim()}><Plus size={12} /> Add</button>
          </form>
        {/if}
        <ul class="mc-slot divide-y divide-deep-600/20">
          {#each data.allowlist as a (a.name)}
            <li class="flex items-center justify-between gap-3 px-3 py-2 text-xs">
              <span><span class="text-strong">{a.name}</span>{#if a.xuid}<span class="font-mono text-[10px] text-deep-400"> · {a.xuid}</span>{/if}{#if a.ignoresPlayerLimit}<span class="text-[10px] text-bedrock-300"> · ignores player limit</span>{/if}</span>
              {#if can.op}<button class="btn-ghost p-1 text-deep-300" title="Remove" onclick={() => run('allowlist_remove', a.name)}><X size={12} /></button>{/if}
            </li>
          {:else}
            <li class="px-3 py-6 text-center text-xs text-deep-400">The allowlist is empty</li>
          {/each}
        </ul>
      </div>
    {:else if tab === 'operators'}
      <div class="card space-y-3">
        <p class="text-xs text-deep-300">permissions.json sets each player's level by xuid; everyone else gets <span class="font-mono">{data.default_permission ?? 'member'}</span>. Change a level from the player list (shield icon).</p>
        <ul class="mc-slot divide-y divide-deep-600/20">
          {#each data.operators as o (o.xuid)}
            <li class="flex items-center justify-between gap-3 px-3 py-2 text-xs">
              <span><span class="text-strong">{o.name ?? 'Unknown player'}</span><span class="font-mono text-[10px] text-deep-400"> · {o.xuid}</span></span>
              <span class="flex items-center gap-2">
                <span class="{o.permission === 'operator' ? 'badge-warn' : o.permission === 'visitor' ? 'badge-stopped' : 'badge-info'} !text-[9px] !py-0.5">{o.permission}</span>
                {#if can.op && o.name}<button class="btn-ghost p-1 text-deep-300" title="Reset to member" onclick={() => run('permission', o.name ?? '', { level: 'member' })}><X size={12} /></button>{/if}
              </span>
            </li>
          {:else}
            <li class="px-3 py-6 text-center text-xs text-deep-400">No explicit permissions</li>
          {/each}
        </ul>
      </div>
    {:else if tab === 'bans'}
      <div class="card">
        <ul class="mc-slot divide-y divide-deep-600/20">
          {#each data.bans as b (b.name)}
            <li class="flex items-center justify-between gap-3 px-3 py-2 text-xs">
              <span class="min-w-0">
                <span class="text-strong">{b.name}</span>
                <span class="block text-[10px] text-deep-400 truncate">{b.reason ?? 'No reason'}{b.source ? ` · by ${b.source}` : ''}{b.created ? ` · ${b.created}` : ''}{b.expires && b.expires !== 'forever' ? ` · until ${b.expires}` : ''}</span>
              </span>
              {#if can.ban && data.running}<button class="btn-ghost text-[10px] px-2 py-1 text-teal-300" onclick={() => run('pardon', b.name)}>Pardon</button>{/if}
            </li>
          {:else}
            <li class="px-3 py-6 text-center text-xs text-deep-400">Nobody is banned</li>
          {/each}
        </ul>
        {#if !data.running}<p class="mt-2 text-[10px] text-deep-400">Start the server to ban or pardon (Endstone handles bans).</p>{/if}
      </div>
    {/if}
  {/if}
</div>

<PlayerActionDialog action={dialog?.action ?? null} target={dialog?.player.name ?? ''} {busy}
  onsubmit={(extra) => dialog && run(dialog.action, dialog.player.name, { ...extra, xuid: dialog.player.xuid ?? undefined })}
  oncancel={() => (dialog = null)} />

<ConfirmDialog open={confirm !== null} title={confirm?.title ?? ''} message={confirm?.message ?? ''} confirmText="Confirm" variant="danger"
  onconfirm={() => { const c = confirm; confirm = null; if (c) void run(c.action, c.target); }}
  oncancel={() => (confirm = null)} />
