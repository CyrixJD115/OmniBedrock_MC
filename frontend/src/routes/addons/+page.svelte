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
    Upload as UploadIcon, Loader2,
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
  import PreviewDrawer from '$lib/components/addons/PreviewDrawer.svelte';
  import ManifestEditorModal from '$lib/components/addons/ManifestEditorModal.svelte';
  import VersionEditorModal from '$lib/components/addons/VersionEditorModal.svelte';
  import UuidEditorModal from '$lib/components/addons/UuidEditorModal.svelte';
  import RenameModal from '$lib/components/addons/RenameModal.svelte';

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
  let manifestEditorOpen = $state(false);
  let manifestEditPath = $state('');
  let manifestEditor: ReturnType<typeof ManifestEditorModal> | undefined = $state();

  // version editor
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

  // ─── helpers ──────────────────────────────────────────────────────────────
  const STORAGE_KEY = 'omb_addon_hidden';
  const LOG_MAX = 200;

  // ─── upload / icons / delete / bulk ─────────────────────────────────────
  let uploadBusy = $state(false);
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

  async function handleFiles(files: File[]) {
    if (!canManage || uploadBusy || files.length === 0) return;
    if (!selWorld || selWorld === ALL_WORLDS) {
      addToast('Select a world to upload into', 'error');
      return;
    }
    uploadBusy = true;
    let installed = 0;
    let failed = 0;
    for (const f of files) {
      if (!/\.(mcpack|mcaddon|zip)$/i.test(f.name)) {
        addToast(`${f.name}: only .mcpack, .mcaddon, and .zip are supported`, 'error');
        failed++;
        continue;
      }
      if (f.size > 512 * 1024 * 1024) {
        addToast(`${f.name}: exceeds the 512 MiB limit`, 'error');
        failed++;
        continue;
      }
      try {
        const r = await api.uploadAddon(selWorld, f);
        installed += r.installed?.length ?? 1;
        log(r.message);
        addToast(r.message, 'success');
      } catch (e: any) {
        failed++;
        addToast(`${f.name}: ${e.message}`, 'error');
        log(`Upload failed: ${f.name} — ${e.message}`);
      }
    }
    uploadBusy = false;
    if (installed > 0) await loadAddons();
    if (files.length > 1) addToast(`${installed} pack(s) installed, ${failed} file(s) failed`, failed ? 'info' : 'success');
  }

  function onFileDrop(e: DragEvent) {
    e.preventDefault();
    dragFilesOver = false;
    if (e.dataTransfer?.types.includes('Files')) {
      void handleFiles([...e.dataTransfer.files]);
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

  // World pack orders: activation state + debounced autosave engine.
  const orders = new PackOrders({
    log,
    world: () => selWorld,
    canManage: () => canManage,
  });

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

  // ─── manifest editor ───────────────────────────────────────────────────────
  function openManifestEditor(a: Addon) {
    manifestEditPath = a.path;
    manifestEditor?.openWith(a.manifest);
  }

  async function saveManifest(raw: string) {
    try {
      const obj = JSON.parse(raw);
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
     role="region" aria-label="Addon organizer; drop archives here to upload"
     ondragover={onFileDragOver}
     ondragleave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node)) dragFilesOver = false; }}
     ondrop={onFileDrop}>
  {#if dragFilesOver && canManage}
    <div class="fixed inset-0 z-[70] flex items-center justify-center bg-deep-950/80 border-4 border-dashed border-bedrock-400 pointer-events-none">
      <div class="text-center">
        <UploadIcon size={40} class="text-bedrock-400 mx-auto mb-3 animate-pulse" />
        <p class="text-sm uppercase tracking-widest text-strong">Drop .mcpack / .mcaddon / .zip</p>
        <p class="text-xs text-deep-400 mt-1">Installs into “{selWorld === ALL_WORLDS ? '— pick a world first' : selWorld}”</p>
      </div>
    </div>
  {/if}
  <div class="space-y-4">
    <!-- header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Addon Organizer</h1>
        <div class="pixel-divider mt-2 w-40"></div>
      </div>
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
                   void handleFiles(files);
                   (e.target as HTMLInputElement).value = '';
                 }} />
          <button onclick={() => fileInput?.click()} disabled={uploadBusy || !selWorld || selWorld === ALL_WORLDS}
                  class="btn-primary text-xs py-1.5 px-3 flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
                  title="Upload .mcpack / .mcaddon / .zip into the selected world">
            {#if uploadBusy}<Loader2 size={13} class="animate-spin" />{:else}<UploadIcon size={13} />{/if}
            Upload
          </button>
        {/if}
      </div>
    </div>

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
      <div class="card">
        <div class="flex items-center justify-between mb-2">
          <h2 class="card-header !mb-0">Action Log</h2>
          <button onclick={() => actionLog = []} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Clear</button>
        </div>
        <div class="h-80 overflow-y-auto bg-deep-900/60 border border-deep-600/30 rounded p-3 font-mono text-xs space-y-1">
          {#each actionLog as entry}
            <div class="text-deep-300">{entry}</div>
          {:else}
            <p class="text-deep-400 text-center py-8">No actions logged yet</p>
          {/each}
        </div>
      </div>
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

<!-- Manifest Editor Modal -->
<ManifestEditorModal
  bind:open={manifestEditorOpen}
  bind:this={manifestEditor}
  onsave={saveManifest}
/>

<!-- Version Editor Modal -->
{#if versionEditorOpen && versionEditAddon}
  <VersionEditorModal
    bind:open={versionEditorOpen}
    name={versionEditAddon.name}
    bind:major={versionMajor}
    bind:minor={versionMinor}
    bind:patch={versionPatch}
    onsave={saveVersion}
  />
{/if}

<!-- UUID Editor Modal -->
<UuidEditorModal
  bind:open={uuidEditorOpen}
  mode={uuidEditMode}
  bind:value={uuidEditValue}
  onsave={saveUuid}
/>

<!-- Rename Modal -->
<RenameModal
  bind:open={renameOpen}
  bind:value={renameValue}
  onsave={saveRename}
/>

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
