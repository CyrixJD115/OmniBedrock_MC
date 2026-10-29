<!-- Details tab: full table of every addon with search/filter, column sorting,
     multi-select, and bulk version/UUID/hide/delete operations. -->
<script lang="ts">
  import {
    Eye, Search, Edit, Hash, Shuffle, AlertTriangle, Trash2, Power, PowerOff, Image as ImageIcon,
  } from '@lucide/svelte';
  import { authorOf, fmtDate, fmtSize, kindOf, packColorClass, versionStr, type DetailsSortKey } from '$lib/addons/display';
  import SourceBadge from './SourceBadge.svelte';
  import type { Addon } from '$types/index';

  let {
    addons,
    canManage,
    allWorlds,
    hiddenUuids,
    isActive,
    icon,
    oniconbroken,
    search = $bindable(''),
    packFilter = $bindable<'all' | 'bp' | 'rp'>('all'),
    stateFilter = $bindable<'all' | 'active' | 'hidden'>('all'),
    sortKey = $bindable<DetailsSortKey>('name'),
    sortDir = $bindable<'asc' | 'desc'>('asc'),
    selected = $bindable(new Set<string>()),
    onpreview,
    oneditmanifest,
    oneditversion,
    onrandomizeuuid,
    ontogglehidden,
    ondelete,
    onbulkenable,
    onbump,
    onrandomizebulk,
    ontogglehide,
    ondeletebulk,
  }: {
    addons: Addon[];
    canManage: boolean;
    allWorlds: boolean;
    hiddenUuids: Set<string>;
    isActive: (a: Addon) => boolean;
    icon: (p: Addon) => string;
    oniconbroken: (p: Addon) => void;
    search?: string;
    packFilter?: 'all' | 'bp' | 'rp';
    stateFilter?: 'all' | 'active' | 'hidden';
    sortKey?: DetailsSortKey;
    sortDir?: 'asc' | 'desc';
    selected?: Set<string>;
    onpreview: (a: Addon) => void;
    oneditmanifest: (a: Addon) => void;
    oneditversion: (a: Addon) => void;
    onrandomizeuuid: (a: Addon) => void;
    ontogglehidden: (a: Addon) => void;
    ondelete: (packs: Addon[]) => void;
    onbulkenable: (enabled: boolean) => void;
    onbump: (part: 'major' | 'minor' | 'patch') => void;
    onrandomizebulk: () => void;
    ontogglehide: () => void;
    ondeletebulk: () => void;
  } = $props();

  function setSort(key: DetailsSortKey) {
    if (sortKey === key) sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    else { sortKey = key; sortDir = 'asc'; }
  }
  function sortArrow(key: DetailsSortKey): string {
    if (sortKey !== key) return '';
    return sortDir === 'asc' ? ' ↑' : ' ↓';
  }

  function toggleDetailsSelect(uuid: string) {
    const next = new Set(selected);
    if (next.has(uuid)) next.delete(uuid);
    else next.add(uuid);
    selected = next;
  }

  function selectAllDetails() {
    selected = new Set(addons.map((a) => a.uuid));
  }

  function clearDetailsSelection() {
    selected = new Set();
  }
</script>

<div class="card">
  <div class="flex flex-wrap items-center gap-3 mb-3">
    <div class="relative">
      <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
      <input bind:value={search} class="input text-xs py-1.5 pl-7 w-40" placeholder="Search..." />
    </div>
    <select bind:value={packFilter} class="input text-xs py-1.5 w-28">
      <option value="all">All Types</option>
      <option value="bp">Behavior Packs</option>
      <option value="rp">Resource Packs</option>
    </select>
    <select bind:value={stateFilter} class="input text-xs py-1.5 w-28">
      <option value="all">All States</option>
      <option value="active">Active</option>
      <option value="hidden">Hidden</option>
    </select>
    <button onclick={selectAllDetails} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Select All</button>
    <button onclick={clearDetailsSelection} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Clear</button>
    <span class="text-deep-400 text-xs">{selected.size} / {addons.length}</span>
    {#if canManage && selected.size > 0}
      <div class="flex flex-wrap items-center gap-1 ml-auto">
        {#if !allWorlds}
          <button onclick={() => onbulkenable(true)} class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1" title="Add to the world's pack order">
            <Power size={11} /> Enable
          </button>
          <button onclick={() => onbulkenable(false)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 text-deep-300 flex items-center gap-1" title="Remove from the world's pack order">
            <PowerOff size={11} /> Disable
          </button>
        {/if}
        <button onclick={() => onbump('major')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Major</button>
        <button onclick={() => onbump('minor')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Minor</button>
        <button onclick={() => onbump('patch')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Patch</button>
        <button onclick={onrandomizebulk} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
          <Shuffle size={11} /> Randomize UUIDs
        </button>
        <button onclick={ontogglehide} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Toggle Hide</button>
        <button onclick={ondeletebulk}
                class="btn-ghost text-xs px-2 py-1 rounded border border-red-500/30 text-red-400 flex items-center gap-1">
          <Trash2 size={11} /> Delete
        </button>
      </div>
    {/if}
  </div>

  <div class="overflow-x-auto">
    <table class="w-full text-xs">
      <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
        <th class="py-2 px-2 w-6"></th>
        <th class="text-left py-2 px-2 font-medium w-10">Icon</th>
        <th onclick={() => setSort('type')} class="text-left py-2 px-2 font-medium cursor-pointer hover:text-strong select-none">Type{sortArrow('type')}</th>
        <th onclick={() => setSort('name')} class="text-left py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Name{sortArrow('name')}</th>
        <th onclick={() => setSort('author')} class="text-left py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Author{sortArrow('author')}</th>
        {#if allWorlds}
          <th onclick={() => setSort('world')} class="text-left py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">World{sortArrow('world')}</th>
        {/if}
        <th onclick={() => setSort('uuid')} class="text-left py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">UUID{sortArrow('uuid')}</th>
        <th onclick={() => setSort('version')} class="text-right py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Version{sortArrow('version')}</th>
        <th onclick={() => setSort('size')} class="text-right py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Size{sortArrow('size')}</th>
        <th onclick={() => setSort('modified')} class="text-right py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Date{sortArrow('modified')}</th>
        <th onclick={() => setSort('status')} class="text-center py-2 px-3 font-medium cursor-pointer hover:text-strong select-none">Status{sortArrow('status')}</th>
        <th class="text-right py-2 px-3 font-medium"></th>
      </tr></thead>
      <tbody>
        {#each addons as a (a.path)}
          <tr class="border-b border-deep-700/20 hover:bg-deep-800/30 {!a.valid ? 'opacity-50' : ''}">
            <td class="py-1.5 px-2">
              <input type="checkbox" checked={selected.has(a.uuid)}
                     onchange={() => toggleDetailsSelect(a.uuid)} class="accent-bedrock-500" />
            </td>
            <td class="py-1.5 px-2">
              {#if icon(a)}
                <img src={icon(a)} alt="" class="pack-icon w-7 h-7 rounded-sm object-cover bg-deep-800"
                     onerror={() => oniconbroken(a)} />
              {:else}
                <div class="w-7 h-7 rounded-sm bg-deep-800/80 flex items-center justify-center">
                  <ImageIcon size={13} class="text-deep-400" />
                </div>
              {/if}
            </td>
            <td class="py-1.5 px-2">
              <span class={packColorClass(kindOf(a)) + ' text-[10px] uppercase font-bold'}>
                {kindOf(a) === 'bp' ? 'BP' : 'RP'}
              </span>
            </td>
            <td class="py-1.5 px-3 font-medium">
              <button onclick={() => onpreview(a)} class="hover:text-bedrock-300 transition-colors text-left">
                {a.name}
                {#if !a.valid}<span class="text-red-500 text-[10px] ml-1">(invalid)</span>{/if}
              </button>
              <span class="ml-1.5 inline-flex align-middle"><SourceBadge addon={a} /></span>
            </td>
            <td class="py-1.5 px-3 text-deep-300">{authorOf(a) || '—'}</td>
            {#if allWorlds}
              <td class="py-1.5 px-3 text-deep-300">{a.world}</td>
            {/if}
            <td class="py-1.5 px-3 font-mono text-[10px] text-deep-400">{a.uuid.slice(0, 8)}…</td>
            <td class="py-1.5 px-3 text-right font-mono">{versionStr(a.version)}</td>
            <td class="py-1.5 px-3 text-right font-mono text-deep-300">{fmtSize(a.size_bytes)}</td>
            <td class="py-1.5 px-3 text-right text-deep-400">{fmtDate(a.modified)}</td>
            <td class="py-1.5 px-3 text-center">
              {#if !a.valid}
                <span class="text-red-400 text-[10px]">INVALID</span>
              {:else if isActive(a)}
                <span class="text-green-400 text-[10px]">ACTIVE</span>
              {:else}
                <span class="text-deep-400 text-[10px]">{hiddenUuids.has(a.uuid) ? 'HIDDEN' : 'AVAILABLE'}</span>
              {/if}
            </td>
            <td class="py-1.5 px-3 text-right">
              <div class="flex items-center gap-1 justify-end">
                <button onclick={() => onpreview(a)} class="btn-ghost p-0.5" title="Preview"><Eye size={11} /></button>
                {#if canManage}
                  <button onclick={() => oneditmanifest(a)} class="btn-ghost p-0.5" title="Edit manifest"><Edit size={11} /></button>
                  <button onclick={() => oneditversion(a)} class="btn-ghost p-0.5" title="Version"><Hash size={11} /></button>
                  <button onclick={() => onrandomizeuuid(a)} class="btn-ghost p-0.5" title="Randomize UUID"><Shuffle size={11} /></button>
                  <button onclick={() => ontogglehidden(a)} class="btn-ghost p-0.5" title="Toggle hide"><AlertTriangle size={11} /></button>
                  <button onclick={() => ondelete([a])} class="btn-ghost p-0.5 text-red-400 hover:text-red-300" title="Delete from disk"><Trash2 size={11} /></button>
                {/if}
              </div>
            </td>
          </tr>
        {:else}
          <tr><td colspan={allWorlds ? 11 : 10} class="text-center py-8 text-deep-400">No addons match the current filter</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  /* Minecraft pack icons are pixel art — keep them crisp when scaled up. */
  .pack-icon {
    image-rendering: pixelated;
  }
</style>
