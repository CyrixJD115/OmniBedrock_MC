<script lang="ts">
  /** Player history table with per-row moderation actions. */
  import type { KnownPlayer, PlayerAction } from '$types/players';
  import { PERMISSION_CLASS, ago, dateTime, duration } from '$lib/players/format';
  import { Ban, Copy, Crown, ListChecks, ListX, LogOut, MessageSquare, ShieldHalf, Trash2, Undo2 } from '@lucide/svelte';
  import { addToast } from '$stores/toast';

  let { players, running, bansSupported, can, onaction, empty }: {
    players: KnownPlayer[];
    running: boolean;
    bansSupported: boolean;
    can: { kick: boolean; ban: boolean; op: boolean };
    onaction: (action: PlayerAction, p: KnownPlayer) => void;
    empty: string;
  } = $props();

  let now = $state(Date.now() / 1000);
  $effect(() => {
    const t = setInterval(() => (now = Date.now() / 1000), 30_000);
    return () => clearInterval(t);
  });

  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); addToast('Copied', 'success'); } catch { /* ignore */ }
  }
</script>

<div class="overflow-x-auto">
  <table class="w-full text-xs">
    <thead>
      <tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider text-[10px]">
        <th class="text-left py-2 px-3 font-medium">Player</th>
        <th class="text-left py-2 px-3 font-medium">Status</th>
        <th class="text-right py-2 px-3 font-medium hidden md:table-cell">Sessions</th>
        <th class="text-right py-2 px-3 font-medium hidden md:table-cell">Playtime</th>
        <th class="text-left py-2 px-3 font-medium hidden lg:table-cell">First seen</th>
        <th class="text-right py-2 px-3 font-medium">Actions</th>
      </tr>
    </thead>
    <tbody>
      {#each players as p (p.name)}
        {@const online = p.online_since !== null}
        <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
          <td class="py-2 px-3">
            <div class="flex items-center gap-2.5">
              <span class="w-7 h-7 shrink-0 flex items-center justify-center font-bold text-[11px] uppercase
                           {online ? 'bg-teal-500/20 text-teal-300' : 'bg-deep-700/50 text-deep-300'}">{p.name.slice(0, 2)}</span>
              <div class="min-w-0">
                <p class="text-strong font-medium truncate flex items-center gap-1.5">
                  {p.name}
                  {#if p.permission === 'operator'}<Crown size={11} class="text-yellow-400" />{/if}
                </p>
                <p class="text-[10px] text-deep-400 flex flex-wrap items-center gap-1.5">
                  {#if p.xuid}
                    <button class="font-mono hover:text-deep-200 flex items-center gap-1" onclick={() => copy(p.xuid ?? '')} title="Copy xuid">{p.xuid}<Copy size={9} /></button>
                  {:else}<span>xuid unknown</span>{/if}
                  <span class="{PERMISSION_CLASS[p.permission]} !text-[8px] !py-0 !px-1">{p.permission}</span>
                  {#if p.allowlisted}<span class="badge-running !text-[8px] !py-0 !px-1">allowlist</span>{/if}
                  {#if p.banned}<span class="badge-stopped !text-[8px] !py-0 !px-1">banned</span>{/if}
                </p>
              </div>
            </div>
          </td>
          <td class="py-2 px-3 whitespace-nowrap">
            {#if online}
              <span class="flex items-center gap-1.5 text-teal-300"><span class="status-dot bg-teal-400"></span>online {duration(now - (p.online_since ?? now))}</span>
            {:else}
              <span class="text-deep-400" title={dateTime(p.last_seen)}>seen {ago(p.last_seen, now)}</span>
            {/if}
          </td>
          <td class="py-2 px-3 text-right font-mono hidden md:table-cell">{p.sessions}</td>
          <td class="py-2 px-3 text-right font-mono hidden md:table-cell">{duration(p.playtime_secs + (online ? now - (p.online_since ?? now) : 0))}</td>
          <td class="py-2 px-3 text-deep-400 hidden lg:table-cell" title={dateTime(p.first_seen)}>{new Date(p.first_seen * 1000).toLocaleDateString()}</td>
          <td class="py-2 px-3">
            <div class="flex justify-end gap-0.5">
              {#if can.kick && online && running}
                <button onclick={() => onaction('tell', p)} class="btn-ghost p-1.5" title="Message"><MessageSquare size={12} /></button>
                <button onclick={() => onaction('kick', p)} class="btn-ghost p-1.5 text-yellow-400" title="Kick"><LogOut size={12} /></button>
              {/if}
              {#if can.op}
                <button onclick={() => onaction('permission', p)} class="btn-ghost p-1.5 text-bedrock-300" title="Permission level"><ShieldHalf size={12} /></button>
                {#if p.allowlisted}
                  <button onclick={() => onaction('allowlist_remove', p)} class="btn-ghost p-1.5 text-deep-300" title="Remove from allowlist"><ListX size={12} /></button>
                {:else}
                  <button onclick={() => onaction('allowlist_add', p)} class="btn-ghost p-1.5 text-teal-300" title="Add to allowlist"><ListChecks size={12} /></button>
                {/if}
              {/if}
              {#if can.ban && bansSupported && running}
                {#if p.banned}
                  <button onclick={() => onaction('pardon', p)} class="btn-ghost p-1.5 text-teal-300" title="Pardon"><Undo2 size={12} /></button>
                {:else}
                  <button onclick={() => onaction('ban', p)} class="btn-ghost p-1.5 text-red-400" title="Ban"><Ban size={12} /></button>
                {/if}
              {/if}
              {#if can.op && !online}
                <button onclick={() => onaction('forget', p)} class="btn-ghost p-1.5 text-deep-400" title="Forget history"><Trash2 size={12} /></button>
              {/if}
            </div>
          </td>
        </tr>
      {:else}
        <tr><td colspan="6" class="text-center py-10 text-deep-400">{empty}</td></tr>
      {/each}
    </tbody>
  </table>
</div>
