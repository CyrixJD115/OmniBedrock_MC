<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { savedWorld, saveWorld } from '$lib/worldSync';
  import type { Addon } from '$types/index';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import {
    Save, RotateCw, FileDown, FileUp,
    Upload as UploadIcon, Loader2, Puzzle, Blocks, ShieldCheck,
  } from '@lucide/svelte';
  import {
    ALL_WORLDS, PackOrders,
  } from '$lib/addons/packOrders.svelte';
  import {
    authorOf, cmpVersion, dateOf, kindOf, statusRank,
    type AvailSortKey, type DetailsSortKey,
  } from '$lib/addons/display';
  import { downloadOrderFile, pickOrderFile } from '$lib/addons/orderFile';
  import { bulkBumpPacks, toggleHiddenBulk } from '$lib/addons/bulk';
  import ActiveTab from '$lib/components/addons/ActiveTab.svelte';
  import AvailableTab from '$lib/components/addons/AvailableTab.svelte';
  import DetailsTab from '$lib/components/addons/DetailsTab.svelte';
  import PackDeliveryCard from '$lib/components/addons/PackDeliveryCard.svelte';
  import PackCdnCard from '$lib/components/addons/PackCdnCard.svelte';
  import PreviewDrawer from '$lib/components/addons/PreviewDrawer.svelte';
  import PluginsCard from '$lib/components/plugins/PluginsCard.svelte';
  import CompatPanel from '$lib/components/compat/CompatPanel.svelte';
  import UploadPanel from '$lib/components/addons/UploadPanel.svelte';
  import AddonLogTab from '$lib/components/addons/AddonLogTab.svelte';
  import PackEditors from '$lib/components/addons/PackEditors.svelte';
  import PackHealthStrip from '$lib/components/addons/PackHealthStrip.svelte';
  import { UploadQueue } from '$lib/addons/uploadQueue.svelte';

  // ─── state ────────────────────────────────────────────────────────────────
  let worlds = $state<string[]>([]);
  let selWorld = $state('');
  let allBp = $state<Addon[]>([]);
  let allRp = $state<Addon[]>([]);
  let loading = $state(true);
  let actionLog = $state<string[]>([]);

  // tabs — Minecraft-client style dual tab plus management extras
  type AddonTab = 'active' | 'available' | 'details' | 'logs';
  let activeTab = $state<AddonTab>('active');
  let lastWorldTab = $state<'active' | 'available'>('active');

  // hidden UUIDs (persisted to localStorage)
  let hiddenUuids = $state<Set<string>>(new Set());

  // manifest preview (slide-in drawer)
  let previewAddon = $state<Addon | null>(null);
  let manifestViewMode = $state<'structured' | 'raw'>('structured');
  let manifestWrap = $state(true);

  // manifest editor modal
  let editors: ReturnType<typeof PackEditors> | undefined = $state();
  let packHealth = $state<Record<string, 'error' | 'warning'>>({});

  // ─── helpers ──────────────────────────────────────────────────────────────
  const STORAGE_KEY = 'omb_addon_hidden';
  const LOG_MAX = 200;

  // ─── upload / icons / delete / bulk ─────────────────────────────────────
  let dragFilesOver = $state(false);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let brokenIcons = $state<Set<string>>(new Set());
  let sortKey = $state<DetailsSortKey>('name');
  let sortDir = $state<'asc' | 'desc'>('asc');
  let confirmState = $state<null | { title: string; message: string; confirmText?: string; run: () => Promise<void> }>(null);

  function iconFor(p: Addon): string {
    return p.icon && !brokenIcons.has(p.path)
      ? api.addonIconUrl(p.path, p.icon_version)
      : '';
  }
  function markIconBroken(p: Addon) {
    const next = new Set(brokenIcons);
    next.add(p.path);
    brokenIcons = next;
  }

  const uploads = new UploadQueue();
  /** Files dropped while "All worlds" was selected wait for a world pick. */
  let pendingFiles = $state<File[] | null>(null);
  let pickWorld = $state('');

  function handleFiles(files: File[]) {
    if (!canManage || files.length === 0) return;
    if (!selWorld || selWorld === ALL_WORLDS) {
      pendingFiles = files;
      pickWorld = worlds[0] ?? '';
      return;
    }
    for (const f of files) log(`Upload queued: ${f.name}`);
    uploads.add(files, selWorld, () => void loadAddons());
  }

  async function confirmPickWorld() {
    const files = pendingFiles;
    pendingFiles = null;
    if (!files || !pickWorld) return;
    selWorld = pickWorld;
    await onWorldChange();
    handleFiles(files);
  }

  function activateUploaded(uuid: string) {
    const pack = worldBP.concat(worldRP).find((p) => p.uuid === uuid);
    if (pack) orders.activate(pack);
    else addToast('Pack not found yet; reload the list', 'info');
  }

  function onFileDrop(e: DragEvent) {
    e.preventDefault();
    dragFilesOver = false;
    if (e.dataTransfer?.types.includes('Files')) {
      handleFiles([...e.dataTransfer.files]);
    }
  }
  function onFileDragOver(e: DragEvent) {
    if (e.dataTransfer?.types.includes('Files')) {
      e.preventDefault();
      dragFilesOver = true;
    }
  }

  function askDelete(packs: Addon[]) {
    if (!canManage || packs.length === 0) return;
    const names = packs.map((p) => p.name).join(', ');
    confirmState = {
      title: packs.length === 1 ? 'Delete addon?' : `Delete ${packs.length} addons?`,
      message: `This permanently removes ${names} from disk and clears them from the world's pack orders.`,
      confirmText: 'Delete',
      run: async () => {
        let ok = 0;
        for (const pack of packs) {
          try {
            await api.deleteAddon(pack.path);
            log(`Deleted ${pack.name}`);
            ok++;
          } catch (e: any) {
            addToast(`${pack.name}: ${e.message}`, 'error');
          }
        }
        addToast(`Deleted ${ok} addon(s)`, ok ? 'success' : 'error');
        await loadAddons();
      },
    };
  }

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

  // Mods page top level: pack organizer vs the Endstone plugin manager.
  // The plugins sub-tab only exists on endstone-backend instances held by a
  // PLUGIN_MANAGE user (enforced server-side as well).
  let modTab = $state<'addons' | 'plugins' | 'compat'>('addons');
  const pluginsAvailable = $derived(
    $activeInstance?.backend === 'endstone' && $userPermissions.includes('PLUGIN_MANAGE')
  );
  $effect(() => {
    // Losing the active instance's backend (or the permission) must not
    // strand the user on a hidden tab.
    if (!pluginsAvailable && modTab === 'plugins') modTab = 'addons';
  });
  function modTabClass(tab: 'addons' | 'plugins' | 'compat'): string {
    return 'flex items-center gap-1.5 px-3 py-1.5 text-xs uppercase tracking-wider font-semibold transition-all border-b-2 ' +
      (modTab === tab
        ? 'border-bedrock-400 text-bedrock-300 bg-bedrock-500/10'
        : 'border-transparent text-deep-400 hover:text-deep-200');
  }

  // World pack orders: activation state + debounced autosave engine.
  const orders = new PackOrders({
    log,
    world: () => selWorld,
    canManage: () => canManage,
  });

  /** After an edit, reload and keep the preview on the (possibly renamed) pack. */
  async function afterEdit(oldPath: string, newPath: string) {
    await loadAddons();
    if (!previewAddon || previewAddon.path !== oldPath) return;
    const updated = allBp.concat(allRp).find((p) => p.path === newPath);
    previewAddon = updated ?? null;
  }

  // ─── data ─────────────────────────────────────────────────────────────────
  onMount(() => {
    hiddenUuids = loadHidden();
  });

  // Bumps on every (re)load so a slow response for a previously selected
  // instance can never overwrite the freshly loaded one.
  let epoch = 0;

  /** Keep the tab bar consistent with the world scope. */
  function normalizeTabsForWorld() {
    if (selWorld === ALL_WORLDS) {
      if (activeTab === 'active' || activeTab === 'available') {
        lastWorldTab = activeTab;
        activeTab = 'details';
      }
    } else if (activeTab === 'details') {
      activeTab = lastWorldTab;
    }
  }

  async function loadWorldsAndAddons() {
    const mine = ++epoch;
    loading = true;
    // Per-instance UI state: a stale preview or selection must not carry into
    // another instance (e.g. activating a foreign pack into its order).
    previewAddon = null;
    availSelected = new Set();
    detailsSelected = new Set();
    try {
      const list = await api.listWorlds();
      if (mine !== epoch) return;
      worlds = list;
      // Keep the current world when this instance has it; otherwise restore
      // the globally saved world, or fall back to the instance's first world.
      if (!selWorld || selWorld === ALL_WORLDS || !worlds.includes(selWorld)) {
        const saved = savedWorld();
        selWorld = saved && worlds.includes(saved) ? saved : (worlds.length > 0 ? worlds[0] : ALL_WORLDS);
      }
      normalizeTabsForWorld();
      saveWorld(selWorld === ALL_WORLDS ? '' : selWorld);
      await loadAddons();
    } catch (e: any) {
      if (mine !== epoch) return;
      addToast(`Failed to load: ${e.message}`, 'error');
    }
    if (mine === epoch) loading = false;
  }

  // Addons are instance-scoped: reload worlds, packs, and orders whenever the
  // active instance changes. The first run also covers the initial load once
  // the layout boot has resolved the instance list. Keyed on the instance id:
  // the layout's periodic list refresh re-emits activeInstance with a fresh
  // object even when the selection is unchanged, and re-running the whole
  // load on every emission would visibly reload the page and bounce the tabs.
  let loadedInstanceId: string | null = null;
  $effect(() => {
    const id = $activeInstance?.id ?? 'default';
    if (id === loadedInstanceId) return;
    loadedInstanceId = id;
    // In-flight debounced order edits belong to the previous instance; the
    // scoped client already points at the new one, so drop them instead of
    // flushing them into the wrong instance.
    orders.cancelPending();
    void loadWorldsAndAddons();
  });

  async function loadAddons(refresh = false) {
    const mine = epoch;
    try {
      const a = await api.listAddons(refresh);
      if (mine !== epoch) return;
      allBp = a.behavior_packs as Addon[];
      allRp = a.resource_packs as Addon[];
      await loadOrders();
    } catch (e: any) {
      if (mine !== epoch) return;
      addToast(`Failed to load addons: ${e.message}`, 'error');
    }
  }

  async function loadOrders() {
    if (!selWorld || selWorld === ALL_WORLDS) { orders.orderBP = []; orders.orderRP = []; return; }
    const mine = epoch;
    // Land any pending/in-flight save first, or the reload would undo the
    // user's last drag with the server's older copy.
    orders.flushPendingSave();
    await orders.settled();
    if (mine !== epoch) return;
    try {
      const bpOrder = await api.getPackOrder(selWorld, 'behavior_packs');
      if (mine !== epoch) return;
      orders.orderBP = (bpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
      const rpOrder = await api.getPackOrder(selWorld, 'resource_packs');
      if (mine !== epoch) return;
      orders.orderRP = (rpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
    } catch {
      if (mine !== epoch) return;
      orders.orderBP = [];
      orders.orderRP = [];
    }
    orders.orderDirty = false;
  }

  async function onWorldChange() {
    orders.flushPendingSave();
    previewAddon = null;
    availSelected = new Set();
    detailsSelected = new Set();
    saveWorld(selWorld === ALL_WORLDS ? '' : selWorld);
    const worldScoped = selWorld !== ALL_WORLDS;
    normalizeTabsForWorld();
    if (!worldScoped) {
      orders.orderBP = [];
      orders.orderRP = [];
      orders.orderDirty = false;
      return;
    }
    await loadOrders();
  }

  // ─── world-scoped lists ───────────────────────────────────────────────────
  let worldBP = $derived(allBp.filter((p) => selWorld !== ALL_WORLDS && p.world === selWorld));
  let worldRP = $derived(allRp.filter((p) => selWorld !== ALL_WORLDS && p.world === selWorld));
  let activeBPPacks = $derived(orders.orderBP.map((u) => worldBP.find((p) => p.uuid === u)).filter((p): p is Addon => !!p));
  let activeRPPacks = $derived(orders.orderRP.map((u) => worldRP.find((p) => p.uuid === u)).filter((p): p is Addon => !!p));
  let missingBP = $derived(orders.orderBP.filter((u) => !worldBP.some((p) => p.uuid === u)));
  let missingRP = $derived(orders.orderRP.filter((u) => !worldRP.some((p) => p.uuid === u)));
  let activeCount = $derived(activeBPPacks.length + activeRPPacks.length);
  let installedCount = $derived(worldBP.length + worldRP.length);

  // ─── Available tab list ───────────────────────────────────────────────────
  let availableAddons = $derived.by(() => {
    let list = [...worldBP, ...worldRP];
    if (!availShowHidden) list = list.filter((p) => !hiddenUuids.has(p.uuid));
    if (availType !== 'all') list = list.filter((p) => kindOf(p) === availType);
    if (availSource !== 'all') list = list.filter((p) => (p.source ?? 'manual') === availSource);
    const s = searchAvail.toLowerCase();
    if (s) {
      list = list.filter((p) =>
        p.name.toLowerCase().includes(s)
        || p.uuid.toLowerCase().includes(s)
        || authorOf(p).toLowerCase().includes(s)
      );
    }
    const dir = availSortDir === 'asc' ? 1 : -1;
    const cmp: Record<AvailSortKey, (a: Addon, b: Addon) => number> = {
      name: (a, b) => a.name.localeCompare(b.name),
      date: (a, b) => dateOf(a) - dateOf(b),
      author: (a, b) => authorOf(a).localeCompare(authorOf(b)),
      type: (a, b) => a.pack_type.localeCompare(b.pack_type),
      status: (a, b) => Number(orders.isActive(a)) - Number(orders.isActive(b)),
      version: (a, b) => cmpVersion(a.version ?? [], b.version ?? []),
      size: (a, b) => (a.size_bytes ?? 0) - (b.size_bytes ?? 0),
    };
    return [...list].sort((a, b) => cmp[availSortKey](a, b) * dir);
  });

  let detailsAddons = $derived.by(() => {
    const inScope = (a: Addon) => selWorld === ALL_WORLDS || a.world === selWorld;
    let all = [...allBp, ...allRp].filter(inScope);
    if (detailsPackFilter === 'bp') all = all.filter((a) => a.pack_type === 'behavior_packs');
    if (detailsPackFilter === 'rp') all = all.filter((a) => a.pack_type === 'resource_packs');
    if (detailsStateFilter === 'active') all = all.filter((a) => !hiddenUuids.has(a.uuid));
    if (detailsStateFilter === 'hidden') all = all.filter((a) => hiddenUuids.has(a.uuid));
    const s = searchDetails.toLowerCase();
    if (s) all = all.filter((a) => a.name.toLowerCase().includes(s) || a.uuid.toLowerCase().includes(s));
    const dir = sortDir === 'asc' ? 1 : -1;
    const byKey: Record<DetailsSortKey, (a: Addon, b: Addon) => number> = {
      name: (a, b) => a.name.localeCompare(b.name),
      type: (a, b) => a.pack_type.localeCompare(b.pack_type),
      world: (a, b) => a.world.localeCompare(b.world),
      author: (a, b) => authorOf(a).localeCompare(authorOf(b)),
      version: (a, b) => cmpVersion(a.version ?? [], b.version ?? []),
      uuid: (a, b) => a.uuid.localeCompare(b.uuid),
      size: (a, b) => (a.size_bytes ?? 0) - (b.size_bytes ?? 0),
      modified: (a, b) => (a.modified ?? 0) - (b.modified ?? 0),
      status: (a, b) => statusRank(a, orders.orderOf(kindOf(a))) - statusRank(b, orders.orderOf(kindOf(b))),
    };
    return all.sort(byKey[sortKey] ? (a, b) => byKey[sortKey](a, b) * dir : (a, b) => a.name.localeCompare(b.name) * dir);
  });

  // Available-tab filter / search / sort state (bound into AvailableTab)
  let searchAvail = $state('');
  let availType = $state<'all' | 'bp' | 'rp'>('all');
  let availSource = $state<'all' | 'curseforge' | 'manual'>('all');
  let availSortKey = $state<AvailSortKey>('name');
  let availSortDir = $state<'asc' | 'desc'>('asc');
  let availShowHidden = $state(false);
  let availSelected = $state<Set<string>>(new Set());

  // details tab filters (bound into DetailsTab)
  let searchDetails = $state('');
  let detailsPackFilter = $state<'all' | 'bp' | 'rp'>('all');
  let detailsStateFilter = $state<'all' | 'active' | 'hidden'>('all');
  let detailsSelected = $state<Set<string>>(new Set());

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

  const openManifestEditor = (a: Addon) => editors?.openManifest(a);
  const openVersionEditor = (a: Addon) => editors?.openVersion(a);
  const openUuidEditor = (a: Addon, mode: 'change' | 'randomize') => editors?.openUuid(a, mode);
  const openRename = (a: Addon) => editors?.openRename(a);

  // ─── import/export ─────────────────────────────────────────────────────────
  function exportOrder() {
    if (!selWorld) return;
    downloadOrderFile({
      world: selWorld,
      behavior_order: orders.orderBP,
      resource_order: orders.orderRP,
      hidden_uuids: [...hiddenUuids],
      exported_at: new Date().toISOString(),
    });
    log(`Exported order for ${selWorld}`);
  }

  async function importOrder() {
    try {
      const picked = await pickOrderFile();
      if (!picked) return;
      const data = JSON.parse(picked.text);
      // Import is reviewed manually — cancel any pending autosave so it
      // cannot clobber the imported order before Save is clicked.
      orders.cancelPending();
      if (data.behavior_order) orders.orderBP = data.behavior_order;
      if (data.resource_order) orders.orderRP = data.resource_order;
      if (data.hidden_uuids) hiddenUuids = new Set(data.hidden_uuids);
      saveHidden();
      orders.orderDirty = true;
      addToast('Order imported — click Save to apply', 'info');
      log(`Imported order from ${picked.fileName}`);
    } catch (e: any) {
      addToast(`Import failed: ${e.message}`, 'error');
    }
  }

  // ─── bulk operations (details tab) ────────────────────────────────────────
  async function bulkSetEnabled(enabled: boolean, packs?: Addon[]) {
    if (!canManage || !selWorld || selWorld === ALL_WORLDS) return;
    const targets = packs ?? allBp.concat(allRp).filter((p) => detailsSelected.has(p.uuid) && p.world === selWorld);
    let changed = 0;
    for (const pack of targets) {
      const t = kindOf(pack);
      if (enabled && !orders.orderOf(t).includes(pack.uuid)) {
        orders.setOrderOf(t, [...orders.orderOf(t), pack.uuid]);
        changed++;
      } else if (!enabled && orders.orderOf(t).includes(pack.uuid)) {
        orders.setOrderOf(t, orders.orderOf(t).filter((u) => u !== pack.uuid));
        changed++;
      }
    }
    if (changed > 0) {
      orders.touchOrder();
      addToast(`${enabled ? 'Activated' : 'Deactivated'} ${changed} pack(s) in ${selWorld}`, 'success');
      log(`Bulk ${enabled ? 'activate' : 'deactivate'}: ${changed} pack(s)`);
    } else {
      addToast('Nothing to change', 'info');
    }
  }

  function askBulkRandomize() {
    if (detailsSelected.size === 0) return;
    const packs = allBp.concat(allRp).filter((p) => detailsSelected.has(p.uuid) && p.valid);
    if (packs.length === 0) return;
    confirmState = {
      title: `Randomize ${packs.length} UUID(s)?`,
      message: 'New UUIDs break dependency references in other packs. Use this to restore duplicate packs.',
      confirmText: 'Randomize',
      run: async () => {
        let ok = 0;
        for (const pack of packs) {
          try {
            await api.randomizeUuid(pack.path);
            log(`Randomized UUID: ${pack.name}`);
            ok++;
          } catch (e: any) {
            addToast(`${pack.name}: ${e.message}`, 'error');
          }
        }
        addToast(`Randomized ${ok} UUID(s)`, ok ? 'success' : 'error');
        await loadAddons();
      },
    };
  }

  async function bulkBumpVersion(part: 'major' | 'minor' | 'patch') {
    const selected = [...detailsSelected];
    if (selected.length === 0) return;
    const packs = allBp.concat(allRp).filter((p) => selected.includes(p.uuid));
    const count = await bulkBumpPacks(packs, part);
    addToast(`Bumped ${part} on ${count} addons`, 'success');
    log(`Bulk bump ${part}: ${count} addons`);
    await loadAddons();
  }

  async function bulkToggleHide() {
    const selected = [...detailsSelected];
    if (selected.length === 0) return;
    const { next, hiddenCount, unhiddenCount } = toggleHiddenBulk(selected, hiddenUuids);
    hiddenUuids = next;
    saveHidden();
    addToast(`${hiddenCount} hidden, ${unhiddenCount} unhidden`, 'info');
    log(`Bulk toggle hide: ${hiddenCount} hidden, ${unhiddenCount} unhidden`);
  }
</script>

<!-- main layout: shifts left when drawer is open -->
<div class="transition-[padding] duration-200 {previewAddon ? 'lg:pr-96' : ''}"
     role="region" aria-label="Mod organizer; drop archives here to upload"
     ondragover={modTab === 'addons' ? onFileDragOver : undefined}
     ondragleave={modTab === 'addons' ? (e) => { if (!e.currentTarget.contains(e.relatedTarget as Node)) dragFilesOver = false; } : undefined}
     ondrop={modTab === 'addons' ? onFileDrop : undefined}>
  {#if modTab === 'addons' && dragFilesOver && canManage}
    <div class="fixed inset-0 z-[70] flex items-center justify-center bg-deep-950/80 border-4 border-dashed border-bedrock-400 pointer-events-none">
      <div class="text-center">
        <UploadIcon size={40} class="text-bedrock-400 mx-auto mb-3 animate-pulse" />
        <p class="text-sm uppercase tracking-widest text-strong">Drop .mcpack / .mcaddon / .zip</p>
        <p class="text-xs text-deep-400 mt-1">{selWorld === ALL_WORLDS ? 'You will pick the world next' : `Installs into “${selWorld}”`}</p>
      </div>
    </div>
  {/if}
  <div class="space-y-4">
    <!-- header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Mods</h1>
        <div class="pixel-divider mt-2 w-40"></div>
        <div class="flex gap-1 mt-3">
          <button onclick={() => (modTab = 'addons')} class={modTabClass('addons')} aria-selected={modTab === 'addons'} role="tab">
            <Puzzle size={13} /> Addons
          </button>
          {#if pluginsAvailable}
            <button onclick={() => (modTab = 'plugins')} class={modTabClass('plugins')} aria-selected={modTab === 'plugins'} role="tab">
              <Blocks size={13} /> Endstone Plugins
            </button>
          {/if}
          <button onclick={() => (modTab = 'compat')} class={modTabClass('compat')} aria-selected={modTab === 'compat'} role="tab">
            <ShieldCheck size={13} /> Compatibility
          </button>
        </div>
      </div>
      {#if modTab === 'addons'}
      <div class="flex items-center gap-2">
        {#if orders.orderDirty && canManage}
          <span class="text-yellow-400 text-xs uppercase tracking-wider animate-pulse">Unsaved</span>
        {/if}
        {#if canManage}
          <button onclick={() => orders.saveOrder()} disabled={!orders.orderDirty || !selWorld}
                  class="btn-primary text-xs py-1.5 px-3 flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed">
            <Save size={13} /> Save Order
          </button>
        {/if}
        <button onclick={exportOrder} disabled={!selWorld}
                class="btn-ghost p-1.5 disabled:opacity-30" title="Export order"><FileDown size={14} /></button>
        <button onclick={importOrder}
                class="btn-ghost p-1.5" title="Import order"><FileUp size={14} /></button>
        <button onclick={async () => { await loadAddons(true); addToast('Reloaded', 'success'); }}
                class="btn-ghost p-2" title="Reload (re-scan packs, bypassing the cache)"><RotateCw size={14} /></button>
        {#if canManage}
          <input type="file" accept=".mcpack,.mcaddon,.zip" multiple bind:this={fileInput}
                 class="hidden" onchange={(e) => {
                   const files = [...((e.target as HTMLInputElement).files ?? [])];
                   handleFiles(files);
                   (e.target as HTMLInputElement).value = '';
                 }} />
          <button onclick={() => fileInput?.click()} disabled={!selWorld}
                  class="btn-primary text-xs py-1.5 px-3 flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
                  title="Upload .mcpack / .mcaddon / .zip into the selected world">
            {#if uploads.busy}<Loader2 size={13} class="animate-spin" />{:else}<UploadIcon size={13} />{/if}
            Upload
          </button>
        {/if}
      </div>
      {/if}
    </div>

    {#if modTab === 'addons'}
    <!-- world selector -->
    <div class="flex flex-wrap items-center gap-3">
      <label for="addon-world" class="text-deep-400 text-xs uppercase tracking-wider">World</label>
      <select id="addon-world" bind:value={selWorld} onchange={onWorldChange} class="input w-56 text-xs py-1.5">
        <option value={ALL_WORLDS}>All worlds</option>
        {#each worlds as w}<option value={w}>{w}</option>{/each}
      </select>
      <span class="text-deep-400 text-[10px] font-mono">
        {#if selWorld === ALL_WORLDS}
          {allBp.length} BP · {allRp.length} RP (all worlds)
        {:else}
          {installedCount} installed · {activeCount} active · {worldBP.length} BP · {worldRP.length} RP
        {/if}
      </span>
    </div>

    {#if loading}
      <div class="grid grid-cols-1 gap-4" aria-hidden="true">
        <div class="card space-y-2">
          <div class="h-4 w-32 bg-deep-700/40 rounded animate-pulse"></div>
          {#each [0, 1, 2, 3] as j (j)}
            <div class="h-12 bg-deep-800/40 rounded animate-pulse" style="animation-delay: {j * 90}ms"></div>
          {/each}
        </div>
      </div>
    {/if}

    {#if selWorld && selWorld !== ALL_WORLDS}
      <PackHealthStrip world={selWorld} instanceId={$activeInstance?.id ?? 'default'} bind:health={packHealth} onopen={() => (modTab = 'compat')} />
      <PackDeliveryCard world={selWorld} />
    {/if}
    <PackCdnCard canManage={canManage} instanceId={$activeInstance?.id ?? 'default'} />

    <!-- tabs: Minecraft-client style dual tab -->
    <div class="flex gap-1 border-b border-deep-600/30 pb-px">
      {#each [
        { id: 'active' as const, label: 'Active', count: activeCount, worldOnly: true },
        { id: 'available' as const, label: 'Available', count: installedCount, worldOnly: true },
        { id: 'details' as const, label: 'Details', count: -1, worldOnly: false },
        { id: 'logs' as const, label: 'Logs', count: -1, worldOnly: false },
      ] as tab (tab.id)}
        <button onclick={() => (activeTab = tab.id)}
          disabled={tab.worldOnly && selWorld === ALL_WORLDS}
          title={tab.worldOnly && selWorld === ALL_WORLDS ? 'Pick a world to use this tab' : tab.label}
          class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t
                 {activeTab === tab.id
                   ? 'bg-deep-800/60 text-strong border-b-2 border-bedrock-500'
                   : 'text-deep-400 hover:text-deep-200'}
                 {tab.worldOnly && selWorld === ALL_WORLDS ? 'opacity-40 cursor-not-allowed' : ''}">
          {tab.label}
          {#if tab.count >= 0}<span class="text-deep-400 font-normal ml-1">({tab.count})</span>{/if}
        </button>
      {/each}
    </div>

    <!-- ======================== ACTIVE TAB ======================== -->
    {#if activeTab === 'active'}
      <ActiveTab
        bpPacks={activeBPPacks}
        rpPacks={activeRPPacks}
        bpOrder={orders.orderBP}
        rpOrder={orders.orderRP}
        missingBP={missingBP}
        missingRP={missingRP}
        orderDirty={orders.orderDirty}
        canManage={canManage}
        icon={iconFor}
        oniconbroken={markIconBroken}
        onpreview={openPreview}
        onorganize={(mode) => orders.organizeActive(mode, (t, u) => (t === 'bp' ? worldBP : worldRP).find((p) => p.uuid === u))}
        ondeactivateall={() => orders.deactivateAll()}
        onmove={(t, u, d) => orders.moveActive(t, u, d)}
        onremove={(t, u) => orders.removeFromOrder(t, u)}
        onreorder={(t, order, msg) => { orders.setOrderOf(t, order); orders.touchOrder(); log(msg); }}
        health={packHealth}
      />

    <!-- ======================== AVAILABLE TAB ======================== -->
    {:else if activeTab === 'available'}
      <AvailableTab
        addons={availableAddons}
        installedCount={installedCount}
        canManage={canManage}
        hiddenUuids={hiddenUuids}
        worldPacks={worldBP.concat(worldRP)}
        worldLabel={selWorld}
        icon={iconFor}
        oniconbroken={markIconBroken}
        isActive={(p) => orders.isActive(p)}
        bind:search={searchAvail}
        bind:availType
        bind:availSource
        bind:availSortKey
        bind:availSortDir
        bind:availShowHidden
        bind:selected={availSelected}
        onactivate={(p) => orders.activate(p)}
        ondeactivate={(p) => orders.deactivate(p)}
        onpreview={openPreview}
        ondelete={(packs) => askDelete(packs)}
        onbulkenable={(enabled, packs) => bulkSetEnabled(enabled, packs)}
        ondeletepacks={(packs) => askDelete(packs)}
      />

    <!-- ======================== DETAILS TAB ======================== -->
    {:else if activeTab === 'details'}
      <DetailsTab
        addons={detailsAddons}
        canManage={canManage}
        allWorlds={selWorld === ALL_WORLDS}
        hiddenUuids={hiddenUuids}
        isActive={(a) => orders.isActive(a)}
        icon={iconFor}
        oniconbroken={markIconBroken}
        bind:search={searchDetails}
        bind:packFilter={detailsPackFilter}
        bind:stateFilter={detailsStateFilter}
        bind:sortKey
        bind:sortDir
        bind:selected={detailsSelected}
        onpreview={openPreview}
        oneditmanifest={openManifestEditor}
        oneditversion={openVersionEditor}
        onrandomizeuuid={(a) => openUuidEditor(a, 'randomize')}
        ontogglehidden={(a) => toggleHidden(a.uuid)}
        ondelete={(packs) => askDelete(packs)}
        onbulkenable={(enabled) => bulkSetEnabled(enabled)}
        onbump={bulkBumpVersion}
        onrandomizebulk={askBulkRandomize}
        ontogglehide={bulkToggleHide}
        ondeletebulk={() => askDelete(allBp.concat(allRp).filter((p) => detailsSelected.has(p.uuid)))}
      />

    <!-- ======================== LOGS TAB ======================== -->
    {:else if activeTab === 'logs'}
      <AddonLogTab bind:actionLog instanceId={$activeInstance?.id ?? 'default'} />
    {/if}

    {:else if modTab === 'compat'}
      <CompatPanel world={selWorld && selWorld !== ALL_WORLDS ? selWorld : undefined} canManage={canManage} instanceId={$activeInstance?.id ?? 'default'} />
    {:else if pluginsAvailable}
      <PluginsCard instanceId={$activeInstance?.id ?? 'default'} />
    {/if}
  </div>
</div>

<!-- ======================== MANIFEST PREVIEW DRAWER ======================== -->
{#if previewAddon}
  {@const a = previewAddon}
  <PreviewDrawer
    addon={a}
    canManage={canManage}
    active={orders.isActive(a)}
    hidden={hiddenUuids.has(a.uuid)}
    icon={iconFor(a)}
    oniconbroken={() => markIconBroken(a)}
    bind:viewMode={manifestViewMode}
    bind:wrap={manifestWrap}
    onclose={closePreview}
    onactivate={() => orders.activate(a)}
    ondeactivate={() => orders.deactivate(a)}
    oneditmanifest={() => openManifestEditor(a)}
    oneditversion={() => openVersionEditor(a)}
    onrename={() => openRename(a)}
    onchangeuuid={() => openUuidEditor(a, 'change')}
    onrandomizeuuid={() => openUuidEditor(a, 'randomize')}
    ontogglehidden={() => toggleHidden(a.uuid)}
    ondelete={() => { closePreview(); askDelete([a]); }}
  />
{/if}

<!-- ======================== MODALS ======================== -->

<PackEditors bind:this={editors} {log} onchanged={afterEdit} />

<!-- ======================== CONFIRM DIALOG ======================== -->
<ConfirmDialog
  open={confirmState !== null}
  title={confirmState?.title ?? ''}
  message={confirmState?.message ?? ''}
  confirmText={confirmState?.confirmText ?? 'Confirm'}
  variant="danger"
  onconfirm={async () => { const run = confirmState?.run; confirmState = null; await run?.(); }}
  oncancel={() => (confirmState = null)}
/>

<UploadPanel queue={uploads} isActive={(uuid) => orders.orderBP.includes(uuid) || orders.orderRP.includes(uuid)} onactivate={activateUploaded} />

{#if pendingFiles}
  <div class="fixed inset-0 z-[70] flex items-center justify-center p-4 modal-backdrop" role="presentation"
       onclick={(e) => { if (e.target === e.currentTarget) pendingFiles = null; }}
       onkeydown={(e) => e.key === 'Escape' && (pendingFiles = null)}>
    <div class="card w-full max-w-sm space-y-3 text-xs" role="dialog" aria-modal="true" aria-label="Choose a world">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest">Install into which world?</h2>
      <p class="text-deep-300">{pendingFiles.length} file(s): {pendingFiles.map((f) => f.name).join(', ')}</p>
      <select bind:value={pickWorld} class="input w-full text-xs">
        {#each worlds as w (w)}<option value={w}>{w}</option>{/each}
      </select>
      <div class="flex justify-end gap-2">
        <button class="btn-secondary text-xs" onclick={() => (pendingFiles = null)}>Cancel</button>
        <button class="btn-primary text-xs" onclick={confirmPickWorld} disabled={!pickWorld}>Upload</button>
      </div>
    </div>
  </div>
{/if}
