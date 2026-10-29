<!-- Available tab: every installed pack in the selected world with search,
     filter/sort controls, bulk activation/deletion, and per-row actions. -->
<script lang="ts">
  import {
    Package, Eye, Search, Trash2, Plus, Power, PowerOff, ArrowUp, ArrowDown,
  } from '@lucide/svelte';
  import { authorOf, dateOf, fmtDate, fmtSize, versionStr, type AvailSortKey } from '$lib/addons/display';
  import ActivePill from './ActivePill.svelte';
  import PackIcon from './PackIcon.svelte';
  import SourceBadge from './SourceBadge.svelte';
  import TypeBadge from './TypeBadge.svelte';
  import type { Addon } from '$types/index';

  let {
    addons,
    installedCount,
    canManage,
    hiddenUuids,
    worldPacks,
    worldLabel,
    icon,
    oniconbroken,
    isActive,
    search = $bindable(''),
    availType = $bindable<'all' | 'bp' | 'rp'>('all'),
    availSource = $bindable<'all' | 'curseforge' | 'manual'>('all'),
    availSortKey = $bindable<AvailSortKey>('name'),
    availSortDir = $bindable<'asc' | 'desc'>('asc'),
    availShowHidden = $bindable(false),
    selected = $bindable(new Set<string>()),
    onactivate,
    ondeactivate,
    onpreview,
    ondelete,
    onbulkenable,
    ondeletepacks,
  }: {
    addons: Addon[];
    installedCount: number;
    canManage: boolean;
    hiddenUuids: Set<string>;
    worldPacks: Addon[];
    worldLabel: string;
    icon: (p: Addon) => string;
    oniconbroken: (p: Addon) => void;
    isActive: (p: Addon) => boolean;
    search?: string;
    availType?: 'all' | 'bp' | 'rp';
    availSource?: 'all' | 'curseforge' | 'manual';
    availSortKey?: AvailSortKey;
    availSortDir?: 'asc' | 'desc';
    availShowHidden?: boolean;
    selected?: Set<string>;
    onactivate: (p: Addon) => void;
    ondeactivate: (p: Addon) => void;
    onpreview: (p: Addon) => void;
    ondelete: (packs: Addon[]) => void;
    onbulkenable: (enabled: boolean, packs: Addon[]) => void;
    ondeletepacks: (packs: Addon[]) => void;
  } = $props();

  let allAvailSelected = $derived(
    addons.length > 0 && addons.every((p) => selected.has(p.path))
  );

  function toggleAvailSelect(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  function toggleSelectAllAvail() {
    selected = allAvailSelected ? new Set() : new Set(addons.map((p) => p.path));
  }
</script>

<div class="card">
  <!-- filter / sort bar -->
  <div class="flex flex-wrap items-center gap-2 mb-3">
    <div class="relative w-56">
      <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
      <input bind:value={search} class="input w-full text-xs py-1.5 pl-7" placeholder="Search name, author, UUID…" />
    </div>
    <select bind:value={availType} class="input text-xs py-1.5 w-28" aria-label="Filter by pack type">
      <option value="all">All Types</option>
      <option value="bp">Behavior</option>
      <option value="rp">Resource</option>
    </select>
    <select bind:value={availSource} class="input text-xs py-1.5 w-32" aria-label="Filter by source">
      <option value="all">All Sources</option>
      <option value="curseforge">CurseForge</option>
      <option value="manual">Manual</option>
    </select>
    <div class="flex items-center gap-1">
      <select bind:value={availSortKey} class="input text-xs py-1.5 w-32" aria-label="Sort by">
        <option value="name">Name</option>
        <option value="date">Date added</option>
        <option value="author">Author</option>
        <option value="type">Type</option>
        <option value="status">Status</option>
        <option value="version">Version</option>
        <option value="size">Size</option>
      </select>
      <button onclick={() => (availSortDir = availSortDir === 'asc' ? 'desc' : 'asc')}
              class="btn-ghost p-1.5 border border-deep-600/30 rounded"
              title={availSortDir === 'asc' ? 'Sort ascending — click for descending' : 'Sort descending — click for ascending'}>
        {#if availSortDir === 'asc'}<ArrowUp size={12} />{:else}<ArrowDown size={12} />{/if}
      </button>
    </div>
    <label class="flex items-center gap-1.5 text-[10px] text-deep-400 cursor-pointer select-none uppercase tracking-wider">
      <input type="checkbox" bind:checked={availShowHidden} class="accent-bedrock-500" />
      Show hidden
    </label>
    <span class="text-deep-400 text-[10px] font-mono ml-auto">
      {addons.length} / {installedCount} shown
    </span>
  </div>

  <!-- bulk action bar -->
  {#if canManage && selected.size > 0}
    <div class="flex flex-wrap items-center gap-1.5 mb-3 p-2 rounded border border-bedrock-500/30 bg-bedrock-500/5">
      <span class="text-xs text-bedrock-300 font-medium">{selected.size} selected</span>
      <button onclick={() => onbulkenable(true, worldPacks.filter((p) => selected.has(p.path)))}
              class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1"
              title="Add every selected pack to the world's pack order">
        <Power size={11} /> Activate
      </button>
      <button onclick={() => onbulkenable(false, worldPacks.filter((p) => selected.has(p.path)))}
              class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 text-deep-300 flex items-center gap-1"
              title="Remove every selected pack from the order">
        <PowerOff size={11} /> Deactivate
      </button>
      <button onclick={() => ondeletepacks(worldPacks.filter((p) => selected.has(p.path)))}
              class="btn-ghost text-xs px-2 py-1 rounded border border-red-500/30 text-red-400 flex items-center gap-1">
        <Trash2 size={11} /> Delete
      </button>
      <button onclick={() => (selected = new Set())} class="btn-ghost text-xs px-2 py-1">Clear</button>
    </div>
  {/if}

  <!-- the list -->
  <div role="list" aria-label="Available addons for the selected world" class="space-y-1">
    {#each addons as p (p.path)}
      <div role="listitem"
           class="group flex items-center gap-2.5 p-2 border border-deep-600/20 hover:border-deep-500/40 hover:bg-deep-800/40 rounded transition-colors {!p.valid ? 'opacity-50' : ''}"
           title={!p.valid ? 'manifest.json missing or invalid' : p.name}>
        {#if canManage}
          <input type="checkbox" checked={selected.has(p.path)}
                 onchange={() => toggleAvailSelect(p.path)} class="accent-bedrock-500 shrink-0" aria-label="Select {p.name}" />
        {/if}
        <PackIcon icon={icon(p)} pack={p} onbroken={() => oniconbroken(p)} />
        <button onclick={() => onpreview(p)} class="flex-1 min-w-0 text-left">
          <div class="flex items-center gap-1.5 flex-wrap">
            <p class="text-xs font-medium truncate hover:text-strong transition-colors">{p.name}</p>
            <TypeBadge addon={p} />
            <SourceBadge addon={p} />
            <ActivePill active={isActive(p)} />
            {#if hiddenUuids.has(p.uuid)}<span class="text-red-500 text-[8px] uppercase tracking-wider font-bold">Hidden</span>{/if}
            {#if !p.valid}<span class="text-red-500 text-[8px] uppercase tracking-wider font-bold">Invalid</span>{/if}
          </div>
          <p class="text-[10px] text-deep-400 truncate mt-0.5">
            v{versionStr(p.version)}{authorOf(p) ? ` · by ${authorOf(p)}` : ''} · {fmtDate(dateOf(p))} · {fmtSize(p.size_bytes)}
          </p>
        </button>
        {#if canManage}
          <div class="flex items-center gap-0.5 shrink-0">
            {#if isActive(p)}
              <button onclick={() => ondeactivate(p)}
                      class="btn-ghost p-1 text-deep-400 hover:text-deep-200" title="Deactivate (remove from the world's order)"><PowerOff size={13} /></button>
            {:else}
              <button onclick={() => onactivate(p)}
                      class="btn-ghost p-1 text-green-400 hover:text-green-300 opacity-60 group-hover:opacity-100 transition-opacity" title="Activate in this world"><Plus size={14} /></button>
            {/if}
            <button onclick={() => onpreview(p)}
                    class="btn-ghost p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Details"><Eye size={12} /></button>
            <button onclick={() => ondelete([p])}
                    class="btn-ghost p-1 text-red-400 hover:text-red-300 opacity-0 group-hover:opacity-100 transition-opacity" title="Delete from disk"><Trash2 size={12} /></button>
          </div>
        {/if}
      </div>
    {:else}
      <div class="flex items-center justify-center py-10 text-center">
        <div>
          <Package size={22} class="text-deep-500 mx-auto mb-2" />
          <p class="text-deep-400 text-xs">
            {#if search || availType !== 'all' || availSource !== 'all'}
              No addons match the current filters
            {:else}
              No addons installed in “{worldLabel}”
            {/if}
          </p>
          {#if canManage}
            <p class="text-deep-400 text-[10px] mt-1">Drop a .mcpack / .mcaddon anywhere, use Upload, or install from the CurseForge Store — new packs always land here</p>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</div>
