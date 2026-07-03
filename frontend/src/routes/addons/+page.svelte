<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import type { Addon } from '$types/index';
  import {
    Package, Eye, RotateCw, Save, ChevronUp, ChevronDown, X, FileDown, FileUp,
    Edit, Shuffle, Hash, FolderOpen, Trash2, Search, GripVertical, Plus
  } from '@lucide/svelte';

  // ─── state ────────────────────────────────────────────────────────────────
  let worlds = $state<string[]>([]);
  let selWorld = $state('');
  let allBp = $state<Addon[]>([]);
  let allRp = $state<Addon[]>([]);
  let loading = $state(true);
  let actionLog = $state<string[]>([]);

  // tabs
  type AddonTab = 'bp' | 'rp' | 'details' | 'logs';
  let activeTab = $state<AddonTab>('bp');

  // order
  let orderBP = $state<string[]>([]);
  let orderRP = $state<string[]>([]);
  let orderDirty = $state(false);

  // filter / search
  let searchBP = $state('');
  let searchRP = $state('');
  let searchDetails = $state('');
  let detailsPackFilter = $state<'all' | 'bp' | 'rp'>('all');
  let detailsStateFilter = $state<'all' | 'active' | 'hidden'>('all');

  // hidden UUIDs (persisted to localStorage)
  let hiddenUuids = $state<Set<string>>(new Set());

  // manifest preview (slide-in drawer)
  let previewAddon = $state<Addon | null>(null);
  let manifestViewMode = $state<'structured' | 'raw'>('structured');
  let manifestWrap = $state(true);

  // modals
  let manifestEditorOpen = $state(false);
  let manifestEditText = $state('');
  let manifestEditValid = $state(true);
  let manifestEditPath = $state('');

  let versionEditorOpen = $state(false);
  let versionEditAddon = $state<Addon | null>(null);
  let versionMajor = $state(0);
  let versionMinor = $state(0);
  let versionPatch = $state(0);

  let uuidEditorOpen = $state(false);
  let uuidEditPath = $state('');
  let uuidEditValue = $state('');
  let uuidEditMode: 'change' | 'randomize' = $state('change');

  let renameOpen = $state(false);
  let renamePath = $state('');
  let renameValue = $state('');

  // DnD state (must be $state so visual feedback updates reactively)
  type DragSrc = { list: 'available' | 'current'; packType: 'bp' | 'rp'; uuid: string };
  let dragSource = $state<DragSrc | null>(null);
  let dragOverZone = $state<{ zone: 'available' | 'current'; packType: 'bp' | 'rp'; targetUuid?: string } | null>(null);
  let isDragging = $state(false);

  // ─── helpers ──────────────────────────────────────────────────────────────
  const STORAGE_KEY = 'omb_addon_hidden';
  const LOG_MAX = 200;

  function loadHidden(): Set<string> {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      return new Set<string>(raw ? JSON.parse(raw) : []);
    } catch { return new Set(); }
  }

  function saveHidden() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...hiddenUuids]));
  }

  function log(msg: string) {
    const ts = new Date().toLocaleTimeString();
    actionLog = [`[${ts}] ${msg}`, ...actionLog].slice(0, LOG_MAX);
  }

  const canManage = $derived($userPermissions.includes('ADDONS_MANAGE'));

  function esc(s: string): string {
    const d = document.createElement('div');
    d.textContent = s;
    return d.innerHTML;
  }

  function versionStr(v: number[] | undefined): string {
    if (!v || v.length === 0) return '—';
    return v.join('.');
  }

  function packColorClass(packType: 'bp' | 'rp'): string {
    return packType === 'bp' ? 'text-bedrock-400' : 'text-teal-400';
  }

  /** After mutations, refresh the preview to match the updated addon. */
  function syncPreview(path: string) {
    if (!previewAddon || previewAddon.path !== path) return;
    const matchPath = path === renamePath ? undefined : path;
    // For rename, match by new name since path changed
    const updated = allBp.concat(allRp).find((p) =>
      renameOpen ? p.name === renameValue.trim() : p.path === matchPath
    );
    if (updated) previewAddon = updated;
  }

  // ─── data ─────────────────────────────────────────────────────────────────
  onMount(async () => {
    hiddenUuids = loadHidden();
    try {
      worlds = await api.listWorlds();
      if (worlds.length > 0) selWorld = worlds[0];
      await loadAddons();
    } catch (e: any) {
      addToast(`Failed to load: ${e.message}`, 'error');
    }
    loading = false;
  });

  async function loadAddons() {
    try {
      const a = await api.listAddons();
      allBp = a.behavior_packs as Addon[];
      allRp = a.resource_packs as Addon[];
      await loadOrders();
    } catch (e: any) {
      addToast(`Failed to load addons: ${e.message}`, 'error');
    }
  }

  async function loadOrders() {
    if (!selWorld) { orderBP = []; orderRP = []; return; }
    try {
      const bpOrder = await api.getPackOrder(selWorld, 'behavior_packs');
      orderBP = (bpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
      const rpOrder = await api.getPackOrder(selWorld, 'resource_packs');
      orderRP = (rpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
    } catch {
      orderBP = [];
      orderRP = [];
    }
    orderDirty = false;
  }

  async function onWorldChange() {
    previewAddon = null;
    await loadOrders();
  }

  function addonsForWorld(packs: Addon[]): Addon[] {
    return packs.filter((p) => !selWorld || p.world === selWorld);
  }

  // ─── filtering ────────────────────────────────────────────────────────────
  function filtered(packs: Addon[], order: string[], search: string, hideHidden = true): { available: Addon[]; current: Addon[] } {
    const worldPacks = addonsForWorld(packs);
    const orderSet = new Set(order);
    const s = search.toLowerCase();

    let available = worldPacks.filter((p) => !orderSet.has(p.uuid) && !(hideHidden && hiddenUuids.has(p.uuid)));
    let current = worldPacks.filter((p) => orderSet.has(p.uuid));
    current = current.sort((a, b) => order.indexOf(a.uuid) - order.indexOf(b.uuid));

    if (s) {
      available = available.filter((p) => p.name.toLowerCase().includes(s) || p.uuid.toLowerCase().includes(s));
      current = current.filter((p) => p.name.toLowerCase().includes(s) || p.uuid.toLowerCase().includes(s));
    }
    return { available, current };
  }

  let filteredBP = $derived(filtered(allBp, orderBP, searchBP));
  let filteredRP = $derived(filtered(allRp, orderRP, searchRP));

  let detailsAddons = $derived.by(() => {
    let all = [...addonsForWorld(allBp).map((a) => ({ ...a, packTypeLabel: 'BP' as const })),
               ...addonsForWorld(allRp).map((a) => ({ ...a, packTypeLabel: 'RP' as const }))];
    if (detailsPackFilter === 'bp') all = all.filter((a) => a.pack_type === 'behavior_packs');
    if (detailsPackFilter === 'rp') all = all.filter((a) => a.pack_type === 'resource_packs');
    if (detailsStateFilter === 'active') all = all.filter((a) => !hiddenUuids.has(a.uuid));
    if (detailsStateFilter === 'hidden') all = all.filter((a) => hiddenUuids.has(a.uuid));
    const s = searchDetails.toLowerCase();
    if (s) all = all.filter((a) => a.name.toLowerCase().includes(s) || a.uuid.toLowerCase().includes(s));
    return all;
  });

  let detailsSelected = $state<Set<string>>(new Set());

  // ─── DnD ───────────────────────────────────────────────────────────────────
  function handleDragStart(e: DragEvent, list: 'available' | 'current', packType: 'bp' | 'rp', uuid: string) {
    dragSource = { list, packType, uuid };
    isDragging = true;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', uuid);
    }
  }

  function handleDragEnd() {
    dragSource = null;
    dragOverZone = null;
    isDragging = false;
  }

  function handleDragOverZone(e: DragEvent, zone: 'available' | 'current', packType: 'bp' | 'rp', targetUuid?: string) {
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    e.preventDefault();
    if (!dragSource || dragSource.packType !== packType) return;
    dragOverZone = { zone, packType, targetUuid };
  }

  function handleDragLeaveZone(e: DragEvent, packType: 'bp' | 'rp') {
    // Only clear if we're actually leaving the container (not entering a child)
    const rt = e.relatedTarget as HTMLElement | null;
    if (rt && (e.currentTarget as HTMLElement).contains(rt)) return;
    if (dragOverZone?.packType === packType) dragOverZone = null;
  }

  function handleDropOnAvailable(e: DragEvent, packType: 'bp' | 'rp') {
    e.preventDefault();
    const ds = dragSource;
    if (!ds || ds.packType !== packType) { handleDragEnd(); return; }
    const orderArr = packType === 'bp' ? orderBP : orderRP;
    const setOrderFn = packType === 'bp' ? (v: string[]) => orderBP = v : (v: string[]) => orderRP = v;
    if (ds.list === 'current') {
      setOrderFn(orderArr.filter((u) => u !== ds.uuid));
      orderDirty = true;
      log(`Removed ${ds.uuid.slice(0, 8)} from ${packType === 'bp' ? 'BP' : 'RP'} order`);
    }
    handleDragEnd();
  }

  function handleDropOnCurrent(e: DragEvent, packType: 'bp' | 'rp', targetUuid?: string) {
    e.preventDefault();
    const ds = dragSource;
    if (!ds || ds.packType !== packType) { handleDragEnd(); return; }
    const orderArr = packType === 'bp' ? [...orderBP] : [...orderRP];
    const setOrderFn = packType === 'bp' ? (v: string[]) => orderBP = v : (v: string[]) => orderRP = v;

    if (ds.list === 'available') {
      if (!orderArr.includes(ds.uuid)) {
        const idx = targetUuid ? orderArr.indexOf(targetUuid) : orderArr.length;
        if (idx >= 0) orderArr.splice(idx, 0, ds.uuid);
        else orderArr.push(ds.uuid);
        setOrderFn(orderArr);
        orderDirty = true;
        log(`Added ${ds.uuid.slice(0, 8)} to ${packType === 'bp' ? 'BP' : 'RP'} order`);
      }
    } else if (ds.list === 'current') {
      if (ds.uuid !== targetUuid) {
        const fromIdx = orderArr.indexOf(ds.uuid);
        if (fromIdx >= 0) orderArr.splice(fromIdx, 1);
        const toIdx = targetUuid ? orderArr.indexOf(targetUuid) : orderArr.length;
        if (toIdx >= 0) orderArr.splice(toIdx, 0, ds.uuid);
        else orderArr.push(ds.uuid);
        setOrderFn(orderArr);
        orderDirty = true;
        log(`Reordered ${ds.uuid.slice(0, 8)} in ${packType === 'bp' ? 'BP' : 'RP'} order`);
      }
    }
    handleDragEnd();
  }

  function showDropIndicatorBefore(packType: 'bp' | 'rp', zone: 'available' | 'current', uuid: string): boolean {
    if (!isDragging || !dragOverZone) return false;
    return dragOverZone.packType === packType && dragOverZone.zone === zone && dragOverZone.targetUuid === uuid;
  }

  function isDropZoneActive(packType: 'bp' | 'rp', zone: 'available' | 'current'): boolean {
    return isDragging && dragSource?.packType === packType && dragOverZone?.zone === zone && dragOverZone?.packType === packType;
  }

  function isDraggingItem(list: 'available' | 'current', packType: 'bp' | 'rp', uuid: string): boolean {
    return dragSource?.list === list && dragSource?.packType === packType && dragSource?.uuid === uuid;
  }

  // ─── save order ────────────────────────────────────────────────────────────
  async function saveOrder() {
    if (!selWorld || !canManage) return;
    try {
      await api.setPackOrder(selWorld, 'behavior_packs', orderBP);
      await api.setPackOrder(selWorld, 'resource_packs', orderRP);
      orderDirty = false;
      addToast('Pack order saved', 'success');
      log('Pack order saved');
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
    }
  }

  // ─── hide/unhide ───────────────────────────────────────────────────────────
  function toggleHidden(uuid: string) {
    const next = new Set(hiddenUuids);
    if (next.has(uuid)) next.delete(uuid);
    else next.add(uuid);
    hiddenUuids = next;
    saveHidden();
    log(`${next.has(uuid) ? 'Hidden' : 'Unhidden'} ${uuid.slice(0, 8)}`);
  }

  // ─── manifest preview ──────────────────────────────────────────────────────
  function openPreview(a: Addon) {
    previewAddon = a;
  }

  function closePreview() {
    previewAddon = null;
  }

  // ─── manifest editor ───────────────────────────────────────────────────────
  function openManifestEditor(a: Addon) {
    manifestEditPath = a.path;
    manifestEditText = JSON.stringify(a.manifest ?? {}, null, 2);
    manifestEditValid = true;
    manifestEditorOpen = true;
  }

  function validateManifestJson() {
    try {
      JSON.parse(manifestEditText);
      manifestEditValid = true;
    } catch {
      manifestEditValid = false;
    }
  }

  async function saveManifest() {
    if (!manifestEditValid) return;
    try {
      const obj = JSON.parse(manifestEditText);
      await api.updateManifest(manifestEditPath, obj);
      addToast('Manifest saved', 'success');
      log(`Manifest saved: ${manifestEditPath}`);
      manifestEditorOpen = false;
      await loadAddons();
      syncPreview(manifestEditPath);
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
    }
  }

  // ─── version editor ────────────────────────────────────────────────────────
  function openVersionEditor(a: Addon) {
    const v = a.version ?? [0, 0, 0];
    versionMajor = v[0] ?? 0;
    versionMinor = v[1] ?? 0;
    versionPatch = v[2] ?? 0;
    versionEditAddon = a;
    versionEditorOpen = true;
  }

  function bumpVersion(part: 'major' | 'minor' | 'patch') {
    if (part === 'major') { versionMajor++; versionMinor = 0; versionPatch = 0; }
    else if (part === 'minor') { versionMinor++; versionPatch = 0; }
    else if (part === 'patch') { versionPatch++; }
  }

  async function saveVersion() {
    if (!versionEditAddon) return;
    const manifest = versionEditAddon.manifest ? JSON.parse(JSON.stringify(versionEditAddon.manifest)) : {};
    if (!manifest.header) manifest.header = {};
    manifest.header.version = [versionMajor, versionMinor, versionPatch];
    const targetPath = versionEditAddon.path;
    try {
      await api.updateManifest(targetPath, manifest);
      addToast(`Version bumped to ${versionMajor}.${versionMinor}.${versionPatch}`, 'success');
      log(`Version bumped: ${versionEditAddon.name} → ${versionMajor}.${versionMinor}.${versionPatch}`);
      versionEditorOpen = false;
      await loadAddons();
      syncPreview(targetPath);
    } catch (e: any) {
      addToast(`Failed: ${e.message}`, 'error');
    }
  }

  // ─── UUID operations ──────────────────────────────────────────────────────
  function openUuidEditor(a: Addon, mode: 'change' | 'randomize') {
    uuidEditMode = mode;
    uuidEditPath = a.path;
    uuidEditValue = mode === 'randomize' ? crypto.randomUUID() : a.uuid;
    uuidEditorOpen = true;
  }

  async function saveUuid() {
    try {
      if (uuidEditMode === 'randomize') {
        await api.randomizeUuid(uuidEditPath);
      } else {
        await api.changeUuid(uuidEditPath, uuidEditValue);
      }
      addToast(`UUID ${uuidEditMode === 'randomize' ? 'randomized' : 'updated'}`, 'success');
      log(`UUID ${uuidEditMode} for ${uuidEditPath}`);
      uuidEditorOpen = false;
      await loadAddons();
      syncPreview(uuidEditPath);
    } catch (e: any) {
      addToast(`Failed: ${e.message}`, 'error');
    }
  }

  // ─── rename ────────────────────────────────────────────────────────────────
  function openRename(a: Addon) {
    renamePath = a.path;
    renameValue = a.name;
    renameOpen = true;
  }

  async function saveRename() {
    if (!renameValue.trim()) return;
    try {
      await api.renameAddon(renamePath, renameValue.trim());
      addToast(`Renamed to ${renameValue}`, 'success');
      log(`Renamed: ${renamePath} → ${renameValue}`);
      renameOpen = false;
      await loadAddons();
      syncPreview(renamePath);
    } catch (e: any) {
      addToast(`Rename failed: ${e.message}`, 'error');
    }
  }

  // ─── import/export ─────────────────────────────────────────────────────────
  function exportOrder() {
    if (!selWorld) return;
    const data = {
      world: selWorld,
      behavior_order: orderBP,
      resource_order: orderRP,
      hidden_uuids: [...hiddenUuids],
      exported_at: new Date().toISOString(),
    };
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `addon-order-${selWorld}-${new Date().toISOString().slice(0, 10)}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    log(`Exported order for ${selWorld}`);
  }

  function importOrder() {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        const data = JSON.parse(text);
        if (data.behavior_order) orderBP = data.behavior_order;
        if (data.resource_order) orderRP = data.resource_order;
        if (data.hidden_uuids) hiddenUuids = new Set(data.hidden_uuids);
        saveHidden();
        orderDirty = true;
        addToast('Order imported — click Save to apply', 'info');
        log(`Imported order from ${file.name}`);
      } catch (e: any) {
        addToast(`Import failed: ${e.message}`, 'error');
      }
    };
    input.click();
  }

  // ─── bulk operations (details tab) ────────────────────────────────────────
  function toggleDetailsSelect(uuid: string) {
    const next = new Set(detailsSelected);
    if (next.has(uuid)) next.delete(uuid);
    else next.add(uuid);
    detailsSelected = next;
  }

  function selectAllDetails() {
    detailsSelected = new Set(detailsAddons.map((a) => a.uuid));
  }

  function clearDetailsSelection() {
    detailsSelected = new Set();
  }

  async function bulkBumpVersion(part: 'major' | 'minor' | 'patch') {
    const selected = [...detailsSelected];
    if (selected.length === 0) return;
    let count = 0;
    for (const uuid of selected) {
      const a = allBp.concat(allRp).find((p) => p.uuid === uuid);
      if (!a || !a.manifest) continue;
      const manifest = JSON.parse(JSON.stringify(a.manifest));
      if (!manifest.header) manifest.header = {};
      const v = manifest.header.version ?? [0, 0, 0];
      if (part === 'major') { v[0] = (v[0] ?? 0) + 1; v[1] = 0; v[2] = 0; }
      else if (part === 'minor') { v[1] = (v[1] ?? 0) + 1; v[2] = 0; }
      else if (part === 'patch') { v[2] = (v[2] ?? 0) + 1; }
      manifest.header.version = v;
      try {
        await api.updateManifest(a.path, manifest);
        count++;
      } catch { /* skip */ }
    }
    addToast(`Bumped ${part} on ${count} addons`, 'success');
    log(`Bulk bump ${part}: ${count} addons`);
    await loadAddons();
  }

  async function bulkToggleHide() {
    const selected = [...detailsSelected];
    if (selected.length === 0) return;
    const next = new Set(hiddenUuids);
    let hiddenCount = 0;
    let unhiddenCount = 0;
    for (const uuid of selected) {
      if (next.has(uuid)) { next.delete(uuid); unhiddenCount++; }
      else { next.add(uuid); hiddenCount++; }
    }
    hiddenUuids = next;
    saveHidden();
    addToast(`${hiddenCount} hidden, ${unhiddenCount} unhidden`, 'info');
    log(`Bulk toggle hide: ${hiddenCount} hidden, ${unhiddenCount} unhidden`);
  }

  // ─── structured manifest HTML ─────────────────────────────────────────────
  function structuredHtml(a: Addon): string {
    const m = a.manifest as any;
    if (!m || !m.header) return '<p class="text-red-400 text-xs">Invalid or missing manifest</p>';
    const h = m.header;
    const row = (label: string, value: string, valClass = 'text-deep-200') =>
      `<div class="flex justify-between gap-2 py-1"><span class="text-deep-400 shrink-0">${label}</span><span class="${valClass} text-right break-all">${esc(value)}</span></div>`;
    let html = `<div class="space-y-0.5 text-xs">`;
    html += row('Name', h.name ?? '', 'text-white font-medium');
    html += row('UUID', h.uuid ?? '', 'text-green-400 font-mono');
    html += row('Version', (h.version ?? []).join('.'), 'text-blue-400 font-mono');
    html += row('Format', m.format_version ?? '');
    if (h.min_engine_version) html += row('Min Engine', h.min_engine_version.join('.'));
    if (h.description) {
      html += `<div class="bg-deep-800/60 rounded p-2 mt-2 text-deep-300 italic text-[11px]">${esc(h.description)}</div>`;
    }
    if (m.modules?.length) {
      html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Modules (${m.modules.length})</span>`;
      html += m.modules.map((mod: any) =>
        `<div class="flex justify-between text-deep-300 py-0.5 gap-2"><span class="min-w-0 truncate">${esc(mod.type ?? '')}${mod.description ? ' — <span class="text-deep-500">' + esc(mod.description) + '</span>' : ''}</span><span class="font-mono text-[10px] text-deep-400 shrink-0">${esc((mod.version ?? []).join('.'))}</span></div>`
      ).join('') + `</div>`;
    }
    if (m.dependencies?.length) {
      html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Dependencies (${m.dependencies.length})</span>`;
      html += m.dependencies.map((dep: any) =>
        `<div class="text-deep-300 py-0.5 font-mono text-[10px]">${esc(dep.uuid ?? dep.module_name ?? '')} ${dep.version ? '<span class="text-deep-500">v' + esc(dep.version.join('.')) + '</span>' : ''}</div>`
      ).join('') + `</div>`;
    }
    if (m.metadata) {
      html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Metadata</span>`;
      if (m.metadata.authors) html += `<div class="text-deep-300 text-[11px]">Authors: ${esc(m.metadata.authors.join(', '))}</div>`;
      if (m.metadata.product_type) html += `<div class="text-deep-300 text-[11px]">Product: ${esc(m.metadata.product_type)}</div>`;
      html += `</div>`;
    }
    if (m.capabilities?.length) {
      html += `<div class="mt-2"><span class="text-deep-400 text-[10px] uppercase tracking-wider">Capabilities:</span> <span class="text-deep-300">${m.capabilities.map((c: string) => esc(c)).join(', ')}</span></div>`;
    }
    html += `</div>`;
    return html;
  }
</script>

<!-- main layout: shifts left when drawer is open -->
<div class="transition-[padding] duration-200 {previewAddon ? 'lg:pr-96' : ''}">
  <div class="space-y-4">
    <!-- header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-lg font-bold text-white uppercase tracking-widest">Addon Organizer</h1>
        <div class="pixel-divider mt-2 w-40"></div>
      </div>
      <div class="flex items-center gap-2">
        {#if orderDirty && canManage}
          <span class="text-yellow-400 text-xs uppercase tracking-wider animate-pulse">Unsaved</span>
        {/if}
        {#if canManage}
          <button onclick={() => saveOrder()} disabled={!orderDirty || !selWorld}
                  class="btn-primary text-xs py-1.5 px-3 flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed">
            <Save size={13} /> Save Order
          </button>
        {/if}
        <button onclick={exportOrder} disabled={!selWorld}
                class="btn-ghost p-1.5 disabled:opacity-30" title="Export order"><FileDown size={14} /></button>
        <button onclick={importOrder}
                class="btn-ghost p-1.5" title="Import order"><FileUp size={14} /></button>
        <button onclick={async () => { await loadAddons(); addToast('Reloaded', 'success'); }}
                class="btn-ghost p-2" title="Reload"><RotateCw size={14} /></button>
      </div>
    </div>

    <!-- world selector -->
    <div class="flex items-center gap-3">
      <label for="addon-world" class="text-deep-400 text-xs uppercase tracking-wider">World</label>
      <select id="addon-world" bind:value={selWorld} onchange={onWorldChange} class="input w-48 text-xs py-1.5">
        {#each worlds as w}<option value={w}>{w}</option>{/each}
      </select>
    </div>

    <!-- tabs -->
    <div class="flex gap-1 border-b border-deep-600/30 pb-px">
      {#each [{id: 'bp' as const, label: 'Behavior Packs'}, {id: 'rp' as const, label: 'Resource Packs'}, {id: 'details' as const, label: 'Details'}, {id: 'logs' as const, label: 'Logs'}] as tab}
        <button onclick={() => activeTab = tab.id}
          class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t
                 {activeTab === tab.id
                   ? 'bg-deep-800/60 text-white border-b-2 border-bedrock-500'
                   : 'text-deep-400 hover:text-deep-200'}">
          {tab.label}
        </button>
      {/each}
    </div>

    <!-- ======================== BP / RP TABS ======================== -->
    {#if activeTab === 'bp' || activeTab === 'rp'}
      {@const packType = activeTab}
      {@const order = packType === 'bp' ? orderBP : orderRP}
      {@const setOrder = packType === 'bp' ? (v: string[]) => orderBP = v : (v: string[]) => orderRP = v}
      {@const f = packType === 'bp' ? filteredBP : filteredRP}
      {@const search = packType === 'bp' ? searchBP : searchRP}
      {@const setSearch = packType === 'bp' ? (v: string) => searchBP = v : (v: string) => searchRP = v}

      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <!-- Available -->
        <div class="card flex flex-col">
          <div class="flex items-center justify-between mb-2">
            <h2 class="card-header !mb-0">
              Available
              <span class="text-deep-500 font-normal ml-1">({f.available.length})</span>
            </h2>
          </div>
          <div class="mb-2">
            <div class="relative">
              <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-500 pointer-events-none" />
              <input value={search} oninput={(e) => setSearch((e.target as HTMLInputElement).value)}
                     class="input w-full text-xs py-1.5 pl-7" placeholder="Filter by name or UUID..." />
            </div>
          </div>
          <div role="region" aria-label="Available addons drop zone"
               class="flex-1 space-y-1 max-h-[28rem] overflow-y-auto rounded min-h-[8rem] p-1 -m-1 transition-colors duration-150
                      {isDropZoneActive(packType, 'available') ? 'bg-bedrock-500/10 ring-2 ring-bedrock-500/30 ring-inset' : ''}"
               ondragover={(e) => handleDragOverZone(e, 'available', packType)}
               ondragleave={(e) => handleDragLeaveZone(e, packType)}
               ondrop={(e) => handleDropOnAvailable(e, packType)}>
            {#each f.available as p (p.uuid)}
              <div role="listitem" draggable="true"
                   ondragstart={(e) => handleDragStart(e, 'available', packType, p.uuid)}
                   ondragend={handleDragEnd}
                   class="group flex items-center gap-1.5 p-1.5 border border-deep-600/20 hover:border-deep-500/40 hover:bg-deep-800/40 rounded transition-colors
                          {!p.valid ? 'opacity-50' : ''}
                          {isDraggingItem('available', packType, p.uuid) ? 'opacity-30' : ''}"
                   title={!p.valid ? 'manifest.json missing or invalid' : p.name}>
                <GripVertical size={11} class="text-deep-600 group-hover:text-deep-400 cursor-grab active:cursor-grabbing shrink-0 transition-colors" />
                <Package size={14} class={packColorClass(packType) + ' shrink-0'} />
                <button onclick={() => openPreview(p)} class="flex-1 min-w-0 text-left">
                  <p class="text-xs font-medium truncate hover:text-white transition-colors">{p.name}</p>
                  <p class="text-[10px] text-deep-500 font-mono truncate">
                    {versionStr(p.version)}
                    {#if hiddenUuids.has(p.uuid)}<span class="text-red-500 ml-1">[HIDDEN]</span>{/if}
                    {#if !p.valid}<span class="text-red-500 ml-1">[INVALID]</span>{/if}
                  </p>
                </button>
                {#if canManage && !order.includes(p.uuid)}
                  <button onclick={() => { setOrder([...order, p.uuid]); orderDirty = true; log(`Added ${p.name}`); }}
                          class="btn-ghost p-0.5 text-bedrock-400 hover:text-bedrock-300 opacity-0 group-hover:opacity-100 transition-opacity"
                          title="Add to current order"><Plus size={13} /></button>
                {/if}
                <button onclick={() => openPreview(p)}
                        class="btn-ghost p-0.5 opacity-0 group-hover:opacity-100 transition-opacity" title="Preview"><Eye size={12} /></button>
              </div>
            {:else}
              <div class="flex items-center justify-center py-8 text-center">
                <div>
                  <Package size={20} class="text-deep-700 mx-auto mb-2" />
                  <p class="text-deep-600 text-xs">{search ? 'No matches' : 'No addons available'}</p>
                  {#if isDragging && dragSource?.list === 'current' && dragSource?.packType === packType}
                    <p class="text-bedrock-500 text-[10px] mt-1 animate-pulse">↓ Drop here to remove from order</p>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        </div>

        <!-- Current Order -->
        <div class="card flex flex-col">
          <div class="flex items-center justify-between mb-2">
            <h2 class="card-header !mb-0">
              Current Order
              <span class="text-deep-500 font-normal ml-1">({f.current.length})</span>
            </h2>
            {#if canManage && orderDirty}
              <span class="text-yellow-400/70 text-[10px] uppercase tracking-wider">Modified</span>
            {/if}
          </div>
          <div role="region" aria-label="Current order drop zone"
               class="flex-1 space-y-1 max-h-[28rem] overflow-y-auto rounded min-h-[8rem] p-1 -m-1 transition-colors duration-150
                      {isDropZoneActive(packType, 'current') ? 'bg-bedrock-500/10 ring-2 ring-bedrock-500/30 ring-inset' : ''}"
               ondragover={(e) => handleDragOverZone(e, 'current', packType)}
               ondragleave={(e) => handleDragLeaveZone(e, packType)}
               ondrop={(e) => handleDropOnCurrent(e, packType)}>
            {#each f.current as p, i (p.uuid)}
              <!-- drop indicator before this item -->
              {#if showDropIndicatorBefore(packType, 'current', p.uuid)}
                <div class="h-0.5 bg-bedrock-500 rounded-full mx-1 shadow-[0_0_8px_rgba(6,182,212,0.5)]"></div>
              {/if}
              <div role="listitem" draggable="true"
                   ondragstart={(e) => handleDragStart(e, 'current', packType, p.uuid)}
                   ondragend={handleDragEnd}
                   ondragover={(e) => { e.preventDefault(); if (dragSource) dragOverZone = { zone: 'current', packType, targetUuid: p.uuid }; }}
                   class="group flex items-center gap-1.5 p-1.5 border border-deep-600/30 hover:border-bedrock-500/40 hover:bg-deep-800/40 rounded transition-colors
                          {!p.valid ? 'opacity-50' : ''}
                          {isDraggingItem('current', packType, p.uuid) ? 'opacity-30 border-dashed' : ''}"
                   title={!p.valid ? 'manifest.json missing or invalid' : p.name}>
                <GripVertical size={11} class="text-deep-600 group-hover:text-deep-400 cursor-grab active:cursor-grabbing shrink-0 transition-colors" />
                <span class="text-deep-500 text-[10px] font-mono w-4 text-right shrink-0 tabular-nums">{i + 1}</span>
                <Package size={14} class={packColorClass(packType) + ' shrink-0'} />
                <button onclick={() => openPreview(p)} class="flex-1 min-w-0 text-left">
                  <p class="text-xs font-medium truncate hover:text-white transition-colors">{p.name}</p>
                  <p class="text-[10px] text-deep-500 font-mono truncate">{versionStr(p.version)}</p>
                </button>
                {#if canManage}
                  <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                    <button onclick={() => { const o = [...order]; if (i > 0) { [o[i-1], o[i]] = [o[i], o[i-1]]; setOrder(o); orderDirty = true; } }}
                            disabled={i === 0}
                            class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move up"><ChevronUp size={11} /></button>
                    <button onclick={() => { const o = [...order]; if (i < o.length - 1) { [o[i], o[i+1]] = [o[i+1], o[i]]; setOrder(o); orderDirty = true; } }}
                            disabled={i === f.current.length - 1}
                            class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move down"><ChevronDown size={11} /></button>
                    <button onclick={() => { setOrder(order.filter((u) => u !== p.uuid)); orderDirty = true; log(`Removed ${p.name}`); }}
                            class="btn-ghost p-0.5 text-red-400 hover:text-red-300" title="Remove from order"><X size={11} /></button>
                  </div>
                {/if}
              </div>
            {:else}
              <div class="flex items-center justify-center py-8 text-center border-2 border-dashed border-deep-700/40 rounded">
                <div>
                  <Package size={20} class="text-deep-700 mx-auto mb-2" />
                  <p class="text-deep-600 text-xs">{search ? 'No matches' : 'No addons in order'}</p>
                  {#if isDragging && dragSource?.list === 'available' && dragSource?.packType === packType}
                    <p class="text-bedrock-500 text-[10px] mt-1 animate-pulse">↓ Drop here to add to order</p>
                  {/if}
                </div>
              </div>
            {/each}
            <!-- drop indicator at end -->
            {#if f.current.length > 0 && dragOverZone?.zone === 'current' && dragOverZone?.packType === packType && !dragOverZone?.targetUuid}
              <div class="h-0.5 bg-bedrock-500 rounded-full mx-1 shadow-[0_0_8px_rgba(6,182,212,0.5)]"></div>
            {/if}
          </div>
        </div>
      </div>

    <!-- ======================== DETAILS TAB ======================== -->
    {:else if activeTab === 'details'}
      <div class="card">
        <div class="flex flex-wrap items-center gap-3 mb-3">
          <div class="relative">
            <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-500 pointer-events-none" />
            <input bind:value={searchDetails} class="input text-xs py-1.5 pl-7 w-40" placeholder="Search..." />
          </div>
          <select bind:value={detailsPackFilter} class="input text-xs py-1.5 w-28">
            <option value="all">All Types</option>
            <option value="bp">Behavior Packs</option>
            <option value="rp">Resource Packs</option>
          </select>
          <select bind:value={detailsStateFilter} class="input text-xs py-1.5 w-28">
            <option value="all">All States</option>
            <option value="active">Active</option>
            <option value="hidden">Hidden</option>
          </select>
          <button onclick={selectAllDetails} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Select All</button>
          <button onclick={clearDetailsSelection} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Clear</button>
          <span class="text-deep-500 text-xs">{detailsSelected.size} / {detailsAddons.length}</span>
          {#if canManage && detailsSelected.size > 0}
            <div class="flex items-center gap-1 ml-auto">
              <button onclick={() => bulkBumpVersion('major')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Major</button>
              <button onclick={() => bulkBumpVersion('minor')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Minor</button>
              <button onclick={() => bulkBumpVersion('patch')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Patch</button>
              <button onclick={() => bulkToggleHide()} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Toggle Hide</button>
            </div>
          {/if}
        </div>

        <div class="overflow-x-auto">
          <table class="w-full text-xs">
            <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
              <th class="py-2 px-2 w-6"></th>
              <th class="text-left py-2 px-3 font-medium">Type</th>
              <th class="text-left py-2 px-3 font-medium">Name</th>
              <th class="text-left py-2 px-3 font-medium">UUID</th>
              <th class="text-right py-2 px-3 font-medium">Version</th>
              <th class="text-center py-2 px-3 font-medium">Status</th>
              <th class="text-right py-2 px-3 font-medium"></th>
            </tr></thead>
            <tbody>
              {#each detailsAddons as a (a.uuid)}
                <tr class="border-b border-deep-700/20 hover:bg-deep-800/30 {!a.valid ? 'opacity-50' : ''}">
                  <td class="py-1.5 px-2">
                    <input type="checkbox" checked={detailsSelected.has(a.uuid)}
                           onchange={() => toggleDetailsSelect(a.uuid)} class="accent-bedrock-500" />
                  </td>
                  <td class="py-1.5 px-3">
                    <span class={packColorClass(a.pack_type === 'behavior_packs' ? 'bp' : 'rp') + ' text-[10px] uppercase font-bold'}>
                      {a.pack_type === 'behavior_packs' ? 'BP' : 'RP'}
                    </span>
                  </td>
                  <td class="py-1.5 px-3 font-medium">
                    <button onclick={() => openPreview(a)} class="hover:text-bedrock-300 transition-colors text-left">
                      {a.name}
                      {#if !a.valid}<span class="text-red-500 text-[10px] ml-1">(invalid)</span>{/if}
                    </button>
                  </td>
                  <td class="py-1.5 px-3 font-mono text-[10px] text-deep-400">{a.uuid.slice(0, 8)}...</td>
                  <td class="py-1.5 px-3 text-right font-mono">{versionStr(a.version)}</td>
                  <td class="py-1.5 px-3 text-center">
                    {#if hiddenUuids.has(a.uuid)}
                      <span class="text-red-400 text-[10px]">HIDDEN</span>
                    {:else}
                      <span class="text-green-400 text-[10px]">ACTIVE</span>
                    {/if}
                  </td>
                  <td class="py-1.5 px-3 text-right">
                    <div class="flex items-center gap-1 justify-end">
                      <button onclick={() => openPreview(a)} class="btn-ghost p-0.5" title="Preview"><Eye size={11} /></button>
                      {#if canManage}
                        <button onclick={() => openManifestEditor(a)} class="btn-ghost p-0.5" title="Edit manifest"><Edit size={11} /></button>
                        <button onclick={() => openVersionEditor(a)} class="btn-ghost p-0.5" title="Version"><Hash size={11} /></button>
                        <button onclick={() => openUuidEditor(a, 'randomize')} class="btn-ghost p-0.5" title="Randomize UUID"><Shuffle size={11} /></button>
                        <button onclick={() => toggleHidden(a.uuid)} class="btn-ghost p-0.5" title="Toggle hide"><Trash2 size={11} /></button>
                      {/if}
                    </div>
                  </td>
                </tr>
              {:else}
                <tr><td colspan="7" class="text-center py-8 text-deep-500">No addons match the current filter</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>

    <!-- ======================== LOGS TAB ======================== -->
    {:else if activeTab === 'logs'}
      <div class="card">
        <div class="flex items-center justify-between mb-2">
          <h2 class="card-header !mb-0">Action Log</h2>
          <button onclick={() => actionLog = []} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Clear</button>
        </div>
        <div class="h-80 overflow-y-auto bg-deep-900/60 border border-deep-600/30 rounded p-3 font-mono text-xs space-y-1">
          {#each actionLog as entry}
            <div class="text-deep-300">{entry}</div>
          {:else}
            <p class="text-deep-500 text-center py-8">No actions logged yet</p>
          {/each}
        </div>
      </div>
    {/if}
  </div>
</div>

<!-- ======================== MANIFEST PREVIEW DRAWER ======================== -->
{#if previewAddon}
  {@const a = previewAddon}
  <!-- backdrop on mobile -->
  <button type="button" class="fixed inset-0 z-40 bg-black/40 lg:hidden cursor-pointer" onclick={closePreview} aria-label="Close preview"></button>
  <!-- drawer -->
  <aside class="fixed top-0 right-0 z-50 h-full w-full max-w-md bg-deep-900 border-l-2 border-deep-600/50 shadow-2xl flex flex-col animate-slide-in">
    <!-- drawer header -->
    <div class="flex items-center justify-between p-4 border-b border-deep-700/50 shrink-0">
      <div class="min-w-0">
        <h2 class="text-sm font-bold text-white truncate">{a.name}</h2>
        <p class="text-[10px] text-deep-500 font-mono uppercase tracking-wider">
          {a.pack_type === 'behavior_packs' ? 'Behavior Pack' : 'Resource Pack'} · v{versionStr(a.version)}
        </p>
      </div>
      <button onclick={closePreview} class="btn-ghost p-1.5 shrink-0" title="Close"><X size={16} /></button>
    </div>

    <!-- drawer toolbar -->
    <div class="flex items-center gap-2 p-3 border-b border-deep-700/30 shrink-0">
      <select bind:value={manifestViewMode} class="input text-xs py-1 w-28 shrink-0">
        <option value="structured">Structured</option>
        <option value="raw">Raw JSON</option>
      </select>
      <label class="flex items-center gap-1 text-xs text-deep-400 cursor-pointer select-none shrink-0">
        <input type="checkbox" bind:checked={manifestWrap} class="accent-bedrock-500" />
        Wrap
      </label>
      <div class="flex-1"></div>
      {#if canManage}
        <button onclick={() => openManifestEditor(a)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Edit manifest">
          <Edit size={12} /> Edit
        </button>
        <button onclick={() => openVersionEditor(a)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Edit version">
          <Hash size={12} /> Ver
        </button>
        <button onclick={() => openRename(a)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Rename">
          <FolderOpen size={12} /> Rename
        </button>
      {/if}
    </div>

    <!-- drawer body -->
    <div class="flex-1 overflow-y-auto p-4">
      {#if manifestViewMode === 'structured'}
        <div class="bg-deep-950/50 rounded border border-deep-600/30 p-3">
          {@html structuredHtml(a)}
        </div>
      {:else}
        <pre class="text-xs font-mono bg-deep-950/50 p-3 border border-deep-600/30 rounded
                   {manifestWrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre overflow-x-auto'}">{JSON.stringify(a.manifest ?? {}, null, 2)}</pre>
      {/if}
    </div>

    <!-- drawer footer with quick actions -->
    {#if canManage}
      <div class="p-3 border-t border-deep-700/50 shrink-0 flex flex-wrap gap-1.5">
        <button onclick={() => openUuidEditor(a, 'change')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
          <Edit size={11} /> Change UUID
        </button>
        <button onclick={() => openUuidEditor(a, 'randomize')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
          <Shuffle size={11} /> Randomize
        </button>
        <button onclick={() => toggleHidden(a.uuid)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1"
                title="Toggle hidden state">
          <Trash2 size={11} /> {hiddenUuids.has(a.uuid) ? 'Unhide' : 'Hide'}
        </button>
      </div>
    {/if}
  </aside>
{/if}

<!-- ======================== MODALS ======================== -->

<!-- Manifest Editor Modal -->
{#if manifestEditorOpen}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) manifestEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (manifestEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-2xl shadow-block-lg shadow-black/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-3">Edit Manifest</h2>
      <textarea bind:value={manifestEditText} oninput={validateManifestJson}
                class="w-full h-80 bg-deep-950 text-xs font-mono p-3 border rounded resize-y
                       {manifestEditValid ? 'border-deep-600/30' : 'border-red-500/50'}"
                spellcheck="false"></textarea>
      {#if !manifestEditValid}
        <p class="text-red-400 text-xs mt-1">Invalid JSON — cannot save</p>
      {/if}
      <div class="flex justify-end gap-2 mt-4">
        <button onclick={() => manifestEditorOpen = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => saveManifest()} disabled={!manifestEditValid}
                class="btn-primary text-xs px-4 py-1.5 disabled:opacity-40">Save</button>
      </div>
    </div>
  </div>
{/if}

<!-- Version Editor Modal -->
{#if versionEditorOpen && versionEditAddon}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) versionEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (versionEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg shadow-black/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-1">Version Editor</h2>
      <p class="text-xs text-deep-400 mb-4 truncate">{versionEditAddon.name}</p>

      <div class="flex items-center gap-2 mb-4">
        <span class="text-deep-400 text-xs">Quick:</span>
        <button onclick={() => bumpVersion('major')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Major</button>
        <button onclick={() => bumpVersion('minor')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Minor</button>
        <button onclick={() => bumpVersion('patch')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Patch</button>
      </div>

      <div class="grid grid-cols-3 gap-3 mb-4">
        <div>
          <label for="version-major" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Major</label>
          <input id="version-major" type="number" min="0" max="9999" bind:value={versionMajor} class="input w-full text-xs py-1.5 text-center" />
        </div>
        <div>
          <label for="version-minor" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Minor</label>
          <input id="version-minor" type="number" min="0" max="9999" bind:value={versionMinor} class="input w-full text-xs py-1.5 text-center" />
        </div>
        <div>
          <label for="version-patch" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Patch</label>
          <input id="version-patch" type="number" min="0" max="9999" bind:value={versionPatch} class="input w-full text-xs py-1.5 text-center" />
        </div>
      </div>

      <p class="text-center text-base font-mono text-bedrock-400 mb-4">{versionMajor}.{versionMinor}.{versionPatch}</p>

      <div class="flex justify-end gap-2">
        <button onclick={() => versionEditorOpen = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => saveVersion()} class="btn-primary text-xs px-4 py-1.5">Save</button>
      </div>
    </div>
  </div>
{/if}

<!-- UUID Editor Modal -->
{#if uuidEditorOpen}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) uuidEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (uuidEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-lg shadow-block-lg shadow-black/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-3">
        {uuidEditMode === 'randomize' ? 'Randomize UUID' : 'Change UUID'}
      </h2>
      <label for="uuid-input" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">UUID</label>
      <input id="uuid-input" bind:value={uuidEditValue} class="input w-full text-xs py-1.5 font-mono mb-3" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
      {#if uuidEditMode === 'randomize'}
        <button onclick={() => uuidEditValue = crypto.randomUUID()}
                class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 mb-3">Generate New</button>
      {/if}
      <div class="flex justify-end gap-2">
        <button onclick={() => uuidEditorOpen = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => saveUuid()} class="btn-primary text-xs px-4 py-1.5">Save</button>
      </div>
    </div>
  </div>
{/if}

<!-- Rename Modal -->
{#if renameOpen}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) renameOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (renameOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg shadow-black/50">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-3">Rename Addon</h2>
      <label for="rename-input" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">New Name</label>
      <input id="rename-input" bind:value={renameValue} class="input w-full text-xs py-1.5 mb-4" placeholder="Addon folder name" />
      <div class="flex justify-end gap-2">
        <button onclick={() => renameOpen = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => saveRename()} class="btn-primary text-xs px-4 py-1.5">Rename</button>
      </div>
    </div>
  </div>
{/if}

<style>
  @keyframes slide-in {
    from { transform: translateX(100%); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }
  .animate-slide-in {
    animation: slide-in 0.2s ease-out;
  }
</style>
