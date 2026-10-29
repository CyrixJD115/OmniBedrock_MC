<!-- Active tab: the world's BP/RP load order with drag-and-drop reordering,
     per-row move/remove controls, organize actions, and missing-pack rows. -->
<script lang="ts">
  import {
    Package, ChevronUp, ChevronDown, X, GripVertical, CircleOff, AlertTriangle,
  } from '@lucide/svelte';
  import { authorOf, versionStr, type PackKind } from '$lib/addons/display';
  import PackIcon from './PackIcon.svelte';
  import SourceBadge from './SourceBadge.svelte';
  import TypeBadge from './TypeBadge.svelte';
  import type { Addon } from '$types/index';

  let {
    bpPacks,
    rpPacks,
    bpOrder,
    rpOrder,
    missingBP,
    missingRP,
    orderDirty,
    canManage,
    icon,
    oniconbroken,
    onpreview,
    onorganize,
    ondeactivateall,
    onmove,
    onremove,
    onreorder,
  }: {
    bpPacks: Addon[];
    rpPacks: Addon[];
    bpOrder: string[];
    rpOrder: string[];
    missingBP: string[];
    missingRP: string[];
    orderDirty: boolean;
    canManage: boolean;
    icon: (p: Addon) => string;
    oniconbroken: (p: Addon) => void;
    onpreview: (p: Addon) => void;
    onorganize: (mode: 'az' | 'za' | 'newest' | 'oldest') => void;
    ondeactivateall: () => void;
    onmove: (packType: PackKind, uuid: string, dir: -1 | 1) => void;
    onremove: (packType: PackKind, uuid: string) => void;
    onreorder: (packType: PackKind, order: string[], msg: string) => void;
  } = $props();

  // DnD state — reorder within an Active-tab section
  let dragSource = $state<{ packType: PackKind; uuid: string } | null>(null);
  let dragOverRow = $state<{ packType: PackKind; uuid: string } | null>(null);

  function orderList(packType: PackKind): string[] {
    return packType === 'bp' ? bpOrder : rpOrder;
  }

  function onOrganizeChange(e: Event) {
    const v = (e.target as HTMLSelectElement).value as 'az' | 'za' | 'newest' | 'oldest' | '';
    if (v) onorganize(v);
    (e.target as HTMLSelectElement).value = '';
  }

  function onRowDragStart(e: DragEvent, packType: PackKind, uuid: string) {
    dragSource = { packType, uuid };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', uuid);
    }
  }

  function onRowDragEnd() {
    dragSource = null;
    dragOverRow = null;
  }

  function onRowDragOver(e: DragEvent, packType: PackKind, uuid: string) {
    if (!dragSource || dragSource.packType !== packType) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverRow = { packType, uuid };
  }

  function onRowDrop(e: DragEvent, packType: PackKind, targetUuid: string) {
    e.preventDefault();
    e.stopPropagation();
    const ds = dragSource;
    onRowDragEnd();
    if (!ds || ds.packType !== packType || ds.uuid === targetUuid) return;
    const order = orderList(packType).filter((u) => u !== ds.uuid);
    const idx = order.indexOf(targetUuid);
    if (idx >= 0) order.splice(idx, 0, ds.uuid);
    else order.push(ds.uuid);
    onreorder(packType, order, `Reordered ${ds.uuid.slice(0, 8)} in ${packType === 'bp' ? 'BP' : 'RP'} order`);
  }

  function onZoneDrop(e: DragEvent, packType: PackKind) {
    e.preventDefault();
    const ds = dragSource;
    onRowDragEnd();
    if (!ds || ds.packType !== packType) return;
    const order = orderList(packType).filter((u) => u !== ds.uuid);
    order.push(ds.uuid);
    onreorder(packType, order, `Moved ${ds.uuid.slice(0, 8)} to end of ${packType === 'bp' ? 'BP' : 'RP'} order`);
  }

  function dropBefore(packType: PackKind, uuid: string): boolean {
    return !!dragOverRow && dragOverRow.packType === packType && dragOverRow.uuid === uuid;
  }
</script>

<div class="flex flex-wrap items-center gap-2 mb-1">
  <p class="text-deep-400 text-[10px] uppercase tracking-wider">
    Packs applied in load order — top packs load first. Drag rows or use the arrows.
  </p>
  <div class="flex-1"></div>
  {#if canManage && bpPacks.length + rpPacks.length > 0}
    <select aria-label="Organize active lists"
            value=""
            onchange={onOrganizeChange}
            class="input text-xs py-1 w-36">
      <option value="">Organize…</option>
      <option value="az">Name A→Z</option>
      <option value="za">Name Z→A</option>
      <option value="newest">Newest first</option>
      <option value="oldest">Oldest first</option>
    </select>
    <button onclick={ondeactivateall}
            class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1"
            title="Deactivate every pack in this world">
      <CircleOff size={11} /> Deactivate All
    </button>
  {/if}
</div>
<div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
  {#each [{ kind: 'bp' as const, title: 'Behavior Packs', packs: bpPacks, missing: missingBP },
          { kind: 'rp' as const, title: 'Resource Packs', packs: rpPacks, missing: missingRP }] as section (section.kind)}
    <div class="card flex flex-col">
      <div class="flex items-center justify-between mb-2">
        <h2 class="card-header !mb-0">
          {section.title}
          <span class="text-deep-400 font-normal ml-1">({section.packs.length})</span>
        </h2>
        {#if orderDirty && canManage}
          <span class="text-yellow-400/70 text-[10px] uppercase tracking-wider">Modified</span>
        {/if}
      </div>
      <div role="list" aria-label="Active {section.title} order"
           class="flex-1 space-y-1 max-h-[32rem] overflow-y-auto rounded min-h-[8rem] p-1 -m-1"
           ondragover={(e) => { if (dragSource?.packType === section.kind) { e.preventDefault(); dragOverRow = null; } }}
           ondrop={(e) => onZoneDrop(e, section.kind)}>
        {#each section.packs as p, i (p.path)}
          {#if dropBefore(section.kind, p.uuid)}
            <div class="h-0.5 bg-bedrock-500 rounded-full mx-1 shadow-glow-accent"></div>
          {/if}
          <div role="listitem" draggable="true"
               ondragstart={(e) => onRowDragStart(e, section.kind, p.uuid)}
               ondragend={onRowDragEnd}
               ondragover={(e) => onRowDragOver(e, section.kind, p.uuid)}
               ondrop={(e) => onRowDrop(e, section.kind, p.uuid)}
               class="group flex items-center gap-2 p-2 border rounded transition-colors cursor-grab active:cursor-grabbing
                      {!p.valid ? 'opacity-50 border-red-500/30' : 'border-deep-600/30'}
                      hover:border-bedrock-500/40 hover:bg-deep-800/40
                      {dragSource?.uuid === p.uuid ? 'opacity-30 border-dashed' : ''}"
               title={!p.valid ? 'manifest.json missing or invalid' : p.name}>
            <GripVertical size={12} class="text-deep-400 group-hover:text-deep-400 shrink-0 transition-colors" />
            <span class="text-deep-400 text-[10px] font-mono w-5 text-right shrink-0 tabular-nums">{i + 1}</span>
            <PackIcon icon={icon(p)} pack={p} onbroken={() => oniconbroken(p)} />
            <button onclick={() => onpreview(p)} class="flex-1 min-w-0 text-left">
              <p class="text-xs font-medium truncate hover:text-strong transition-colors">{p.name}</p>
              <p class="text-[10px] text-deep-400 truncate">
                v{versionStr(p.version)}{authorOf(p) ? ` · by ${authorOf(p)}` : ''}
              </p>
            </button>
            <div class="flex items-center gap-1 shrink-0">
              <TypeBadge addon={p} />
              <SourceBadge addon={p} />
            </div>
            {#if canManage}
              <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                <button onclick={() => onmove(section.kind, p.uuid, -1)}
                        disabled={i === 0}
                        class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move up (loads earlier)"><ChevronUp size={12} /></button>
                <button onclick={() => onmove(section.kind, p.uuid, 1)}
                        disabled={i === section.packs.length - 1}
                        class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move down (loads later)"><ChevronDown size={12} /></button>
                <button onclick={() => onremove(section.kind, p.uuid)}
                        class="btn-ghost p-0.5 text-red-400 hover:text-red-300" title="Deactivate (remove from order)"><X size={12} /></button>
              </div>
            {/if}
          </div>
        {/each}
        <!-- order references a pack that is no longer on disk -->
        {#each section.missing as uuid (uuid)}
          <div class="flex items-center gap-2 p-2 border border-red-500/30 bg-red-500/5 rounded"
               title="The world's pack order references this UUID but no matching pack folder exists">
            <AlertTriangle size={12} class="text-red-400 shrink-0" />
            <p class="flex-1 min-w-0 text-[10px] font-mono text-red-400 truncate">missing pack {uuid.slice(0, 8)}…</p>
            {#if canManage}
              <button onclick={() => onremove(section.kind, uuid)}
                      class="btn-ghost p-0.5 text-red-400 hover:text-red-300 shrink-0" title="Remove from order"><X size={12} /></button>
            {/if}
          </div>
        {/each}
        {#if section.packs.length === 0 && section.missing.length === 0}
          <div class="flex items-center justify-center py-8 text-center border-2 border-dashed border-deep-700/40 rounded">
            <div>
              <Package size={20} class="text-deep-500 mx-auto mb-2" />
              <p class="text-deep-400 text-xs">No {section.kind === 'bp' ? 'behavior' : 'resource'} packs active</p>
              <p class="text-deep-400 text-[10px] mt-1">Activate packs from the Available tab</p>
            </div>
          </div>
        {/if}
        {#if dragOverRow === null && dragSource?.packType === section.kind && section.packs.length > 0}
          <div class="h-0.5 bg-bedrock-500 rounded-full mx-1 shadow-glow-accent"></div>
        {/if}
      </div>
    </div>
  {/each}
</div>
