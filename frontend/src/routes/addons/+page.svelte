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
    Package, Eye, RotateCw, Save, ChevronUp, ChevronDown, X, FileDown, FileUp,
    Edit, Shuffle, Hash, FolderOpen, Trash2, Search, GripVertical, Plus,
    Upload as UploadIcon, Image as ImageIcon, Power, PowerOff, AlertTriangle, Loader2,
    ArrowUp, ArrowDown, CircleOff
  } from '@lucide/svelte';

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

  // order (per-world pack activation lists read from world_*_packs.json)
  let orderBP = $state<string[]>([]);
  let orderRP = $state<string[]>([]);
  let orderDirty = $state(false);

  // Available-tab filter / search / sort
  type AvailSortKey = 'name' | 'date' | 'author' | 'type' | 'status' | 'version' | 'size';
  let searchAvail = $state('');
  let availType = $state<'all' | 'bp' | 'rp'>('all');
  let availSource = $state<'all' | 'curseforge' | 'manual'>('all');
  let availSortKey = $state<AvailSortKey>('name');
  let availSortDir = $state<'asc' | 'desc'>('asc');
  let availShowHidden = $state(false);
  let availSelected = $state<Set<string>>(new Set());

  // details tab
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
  let manifestEditMode = $state<'visual' | 'raw'>('visual');
  let manifestDraft = $state({
    name: '',
    description: '',
    v0: 0, v1: 0, v2: 0,
    m0: 0, m1: 0, m2: 0,
    uuid: '',
    authors: '',
  });

  function draftFromText(text: string): boolean {
    try {
      const m = JSON.parse(text);
      const h = m?.header ?? {};
      const v = Array.isArray(h.version) ? h.version : [0, 0, 0];
      const mv = Array.isArray(h.min_engine_version) ? h.min_engine_version : [1, 0, 0];
      manifestDraft = {
        name: typeof h.name === 'string' ? h.name : '',
        description: typeof h.description === 'string' ? h.description : '',
        v0: v[0] ?? 0, v1: v[1] ?? 0, v2: v[2] ?? 0,
        m0: mv[0] ?? 1, m1: mv[1] ?? 0, m2: mv[2] ?? 0,
        uuid: typeof h.uuid === 'string' ? h.uuid : '',
        authors: Array.isArray(m?.metadata?.authors) ? m.metadata.authors.join(', ') : '',
      };
      return true;
    } catch {
      return false;
    }
  }

  function draftToText(): boolean {
    try {
      const m = JSON.parse(manifestEditText);
      if (!m.header) m.header = {};
      m.header.name = manifestDraft.name;
      if (manifestDraft.description) m.header.description = manifestDraft.description;
      else delete m.header.description;
      m.header.version = [Number(manifestDraft.v0) || 0, Number(manifestDraft.v1) || 0, Number(manifestDraft.v2) || 0];
      m.header.min_engine_version = [Number(manifestDraft.m0) || 0, Number(manifestDraft.m1) || 0, Number(manifestDraft.m2) || 0];
      if (manifestDraft.uuid.trim()) m.header.uuid = manifestDraft.uuid.trim();
      const authors = manifestDraft.authors.split(',').map((a: string) => a.trim()).filter(Boolean);
      if (authors.length > 0) {
        if (!m.metadata) m.metadata = {};
        m.metadata.authors = authors;
      } else if (m.metadata) {
        delete m.metadata.authors;
      }
      manifestEditText = JSON.stringify(m, null, 2);
      manifestEditValid = true;
      return true;
    } catch (e: any) {
      addToast(`Cannot apply visual edits: ${e.message}`, 'error');
      return false;
    }
  }

  function setManifestMode(mode: 'visual' | 'raw') {
    if (mode === manifestEditMode) return;
    if (mode === 'visual') {
      if (!draftFromText(manifestEditText)) {
        addToast('Invalid JSON — fix the raw editor first', 'error');
        return;
      }
    } else if (!draftToText()) {
      return;
    }
    manifestEditMode = mode;
  }

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

  // DnD state — reorder within an Active-tab section
  type PackKind = 'bp' | 'rp';
  let dragSource = $state<{ packType: PackKind; uuid: string } | null>(null);
  let dragOverRow = $state<{ packType: PackKind; uuid: string } | null>(null);

  // ─── helpers ──────────────────────────────────────────────────────────────
  const STORAGE_KEY = 'omb_addon_hidden';
  const LOG_MAX = 200;
  const ALL_WORLDS = '__all__';

  // ─── upload / icons / delete / bulk ─────────────────────────────────────
  let uploadBusy = $state(false);
  let dragFilesOver = $state(false);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let brokenIcons = $state<Set<string>>(new Set());
  type SortKey = 'name' | 'type' | 'world' | 'author' | 'version' | 'uuid' | 'size' | 'modified' | 'status';
  let sortKey = $state<SortKey>('name');
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

  // ─── metadata display helpers ────────────────────────────────────────────
  function authorOf(p: Addon): string {
    const authors = (p.manifest as any)?.metadata?.authors;
    if (Array.isArray(authors) && authors.length > 0) return authors.join(', ');
    const st = p.store as any;
    if (Array.isArray(st?.authors) && st.authors.length > 0) return st.authors.join(', ');
    return '';
  }
  function dateOf(p: Addon): number {
    const st = p.store as any;
    if (st?.installed_at) {
      const t = Date.parse(String(st.installed_at));
      if (!Number.isNaN(t)) return t / 1000;
    }
    return p.modified ?? 0;
  }

  // ─── details sorting ────────────────────────────────────────────────────
  function setSort(key: SortKey) {
    if (sortKey === key) sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    else { sortKey = key; sortDir = 'asc'; }
  }
  function sortArrow(key: SortKey): string {
    if (sortKey !== key) return '';
    return sortDir === 'asc' ? ' ↑' : ' ↓';
  }
  function cmpVersion(a: number[], b: number[]): number {
    for (let i = 0; i < 4; i++) {
      const x = a[i] ?? 0;
      const y = b[i] ?? 0;
      if (x !== y) return x - y;
    }
    return 0;
  }
  const statusRank = (p: Addon, order: string[]): number => {
    if (order.includes(p.uuid)) return 0;
    if (!p.valid) return 2;
    return 1;
  };

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
  function kindOf(p: Addon): PackKind {
    return p.pack_type === 'behavior_packs' ? 'bp' : 'rp';
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
  onMount(() => {
    hiddenUuids = loadHidden();
  });

  // Bumps on every (re)load so a slow response for a previously selected
  // instance can never overwrite the freshly loaded one.
  let epoch = 0;

  /** Keep the tab bar consistent with the world scope (matches onWorldChange). */
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
  // the layout boot has resolved the instance list.
  $effect(() => {
    void $activeInstance?.id;
    // In-flight debounced order edits belong to the previous instance; the
    // scoped client already points at the new one, so drop them instead of
    // flushing them into the wrong instance.
    clearTimeout(saveTimer);
    saveTimer = undefined;
    pendingSave = null;
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
    if (!selWorld || selWorld === ALL_WORLDS) { orderBP = []; orderRP = []; return; }
    const mine = epoch;
    try {
      const bpOrder = await api.getPackOrder(selWorld, 'behavior_packs');
      if (mine !== epoch) return;
      orderBP = (bpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
      const rpOrder = await api.getPackOrder(selWorld, 'resource_packs');
      if (mine !== epoch) return;
      orderRP = (rpOrder as any[]).map((o: any) => o.pack_id).filter(Boolean);
    } catch {
      if (mine !== epoch) return;
      orderBP = [];
      orderRP = [];
    }
    orderDirty = false;
  }

  async function onWorldChange() {
    flushPendingSave();
    previewAddon = null;
    availSelected = new Set();
    detailsSelected = new Set();
    saveWorld(selWorld === ALL_WORLDS ? '' : selWorld);
    const worldScoped = selWorld !== ALL_WORLDS;
    normalizeTabsForWorld();
    if (!worldScoped) {
      orderBP = [];
      orderRP = [];
      orderDirty = false;
      return;
    }
    await loadOrders();
  }

  // ─── world-scoped lists ───────────────────────────────────────────────────
  let worldBP = $derived(allBp.filter((p) => selWorld !== ALL_WORLDS && p.world === selWorld));
  let worldRP = $derived(allRp.filter((p) => selWorld !== ALL_WORLDS && p.world === selWorld));
  let activeBPPacks = $derived(orderBP.map((u) => worldBP.find((p) => p.uuid === u)).filter((p): p is Addon => !!p));
  let activeRPPacks = $derived(orderRP.map((u) => worldRP.find((p) => p.uuid === u)).filter((p): p is Addon => !!p));
  let missingBP = $derived(orderBP.filter((u) => !worldBP.some((p) => p.uuid === u)));
  let missingRP = $derived(orderRP.filter((u) => !worldRP.some((p) => p.uuid === u)));
  let activeCount = $derived(activeBPPacks.length + activeRPPacks.length);
  let installedCount = $derived(worldBP.length + worldRP.length);

  function orderOf(t: PackKind): string[] {
    return t === 'bp' ? orderBP : orderRP;
  }
  function setOrderOf(t: PackKind, v: string[]) {
    if (t === 'bp') orderBP = v;
    else orderRP = v;
  }
  function isActive(p: Addon): boolean {
    return orderOf(kindOf(p)).includes(p.uuid);
  }

  // ─── order mutations + autosave ───────────────────────────────────────────
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingSave: { world: string; bp: string[]; rp: string[] } | null = null;

  function touchOrder() {
    if (!canManage || !selWorld || selWorld === ALL_WORLDS) return;
    orderDirty = true;
    pendingSave = { world: selWorld, bp: orderBP, rp: orderRP };
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => flushPendingSave(), 600);
  }

  function flushPendingSave() {
    clearTimeout(saveTimer);
    saveTimer = undefined;
    const p = pendingSave;
    pendingSave = null;
    if (p) void saveOrdersFor(p.world, p.bp, p.rp, true);
  }

  async function saveOrdersFor(world: string, bp: string[], rp: string[], auto: boolean) {
    try {
      await api.setPackOrder(world, 'behavior_packs', bp);
      await api.setPackOrder(world, 'resource_packs', rp);
      orderDirty = false;
      if (auto) log(`Pack order auto-saved (${world})`);
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
      log(`Order save failed: ${e.message}`);
    }
  }

  async function saveOrder() {
    if (!selWorld || selWorld === ALL_WORLDS || !canManage) return;
    clearTimeout(saveTimer);
    saveTimer = undefined;
    pendingSave = null;
    await saveOrdersFor(selWorld, orderBP, orderRP, false);
    addToast('Pack order saved', 'success');
    log('Pack order saved');
  }

  function activate(p: Addon) {
    if (!canManage) return;
    const t = kindOf(p);
    const order = orderOf(t);
    if (!order.includes(p.uuid)) {
      setOrderOf(t, [...order, p.uuid]);
      touchOrder();
      log(`Activated ${p.name}`);
      addToast(`Activated ${p.name}`, 'success');
    }
  }

  function deactivate(p: Addon) {
    if (!canManage) return;
    const t = kindOf(p);
    if (orderOf(t).includes(p.uuid)) {
      setOrderOf(t, orderOf(t).filter((u) => u !== p.uuid));
      touchOrder();
      log(`Deactivated ${p.name}`);
      addToast(`Deactivated ${p.name}`, 'info');
    }
  }

  function removeFromOrder(t: PackKind, uuid: string) {
    if (!canManage) return;
    setOrderOf(t, orderOf(t).filter((u) => u !== uuid));
    touchOrder();
  }

  function moveActive(packType: PackKind, uuid: string, dir: -1 | 1) {
    if (!canManage) return;
    const order = [...orderOf(packType)];
    const i = order.indexOf(uuid);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= order.length) return;
    [order[i], order[j]] = [order[j], order[i]];
    setOrderOf(packType, order);
    touchOrder();
  }

  function onOrganizeChange(e: Event) {
    const v = (e.target as HTMLSelectElement).value as 'az' | 'za' | 'newest' | 'oldest' | '';
    if (v) organizeActive(v);
    (e.target as HTMLSelectElement).value = '';
  }

  function deactivateAll() {
    if (!canManage) return;
    const n = orderBP.length + orderRP.length;
    if (n === 0) return;
    orderBP = [];
    orderRP = [];
    touchOrder();
    addToast(`Deactivated ${n} pack(s)`, 'info');
    log('Deactivated all packs');
  }

  function organizeActive(mode: 'az' | 'za' | 'newest' | 'oldest') {
    if (!canManage) return;
    for (const t of ['bp', 'rp'] as const) {
      const inOrder = orderOf(t)
        .map((u) => (t === 'bp' ? worldBP : worldRP).find((p) => p.uuid === u))
        .filter((p): p is Addon => !!p);
      const missing = orderOf(t).filter((u) => !inOrder.some((p) => p.uuid === u));
      const sorted = [...inOrder].sort((a, b) => {
        if (mode === 'az') return a.name.localeCompare(b.name);
        if (mode === 'za') return b.name.localeCompare(a.name);
        const d = dateOf(a) - dateOf(b);
        return mode === 'newest' ? -d : d;
      });
      setOrderOf(t, [...sorted.map((p) => p.uuid), ...missing]);
    }
    touchOrder();
    const label = { az: 'Name A→Z', za: 'Name Z→A', newest: 'Newest first', oldest: 'Oldest first' }[mode];
    addToast(`Active lists re-sorted (${label})`, 'success');
    log(`Organized active lists: ${label}`);
  }

  async function bulkSetEnabled(enabled: boolean, packs?: Addon[]) {
    if (!canManage || !selWorld || selWorld === ALL_WORLDS) return;
    const targets = packs ?? allBp.concat(allRp).filter((p) => detailsSelected.has(p.uuid) && p.world === selWorld);
    let changed = 0;
    for (const pack of targets) {
      const t = kindOf(pack);
      if (enabled && !orderOf(t).includes(pack.uuid)) {
        setOrderOf(t, [...orderOf(t), pack.uuid]);
        changed++;
      } else if (!enabled && orderOf(t).includes(pack.uuid)) {
        setOrderOf(t, orderOf(t).filter((u) => u !== pack.uuid));
        changed++;
      }
    }
    if (changed > 0) {
      touchOrder();
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
      status: (a, b) => Number(isActive(a)) - Number(isActive(b)),
      version: (a, b) => cmpVersion(a.version ?? [], b.version ?? []),
      size: (a, b) => (a.size_bytes ?? 0) - (b.size_bytes ?? 0),
    };
    return [...list].sort((a, b) => cmp[availSortKey](a, b) * dir);
  });

  let allAvailSelected = $derived(
    availableAddons.length > 0 && availableAddons.every((p) => availSelected.has(p.path))
  );

  function toggleAvailSelect(path: string) {
    const next = new Set(availSelected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    availSelected = next;
  }

  function toggleSelectAllAvail() {
    availSelected = allAvailSelected ? new Set() : new Set(availableAddons.map((p) => p.path));
  }

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
    const byKey: Record<SortKey, (a: Addon, b: Addon) => number> = {
      name: (a, b) => a.name.localeCompare(b.name),
      type: (a, b) => a.pack_type.localeCompare(b.pack_type),
      world: (a, b) => a.world.localeCompare(b.world),
      author: (a, b) => authorOf(a).localeCompare(authorOf(b)),
      version: (a, b) => cmpVersion(a.version ?? [], b.version ?? []),
      uuid: (a, b) => a.uuid.localeCompare(b.uuid),
      size: (a, b) => (a.size_bytes ?? 0) - (b.size_bytes ?? 0),
      modified: (a, b) => (a.modified ?? 0) - (b.modified ?? 0),
      status: (a, b) => statusRank(a, orderOf(kindOf(a))) - statusRank(b, orderOf(kindOf(b))),
    };
    return all.sort(byKey[sortKey] ? (a, b) => byKey[sortKey](a, b) * dir : (a, b) => a.name.localeCompare(b.name) * dir);
  });

  function fmtSize(bytes?: number): string {
    if (!bytes) return '—';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / 1048576).toFixed(1)} MiB`;
  }
  function fmtDate(secs?: number): string {
    if (!secs) return '—';
    return new Date(secs * 1000).toLocaleDateString();
  }

  let detailsSelected = $state<Set<string>>(new Set());

  // ─── DnD (reorder inside an Active section) ───────────────────────────────
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
    const order = orderOf(packType).filter((u) => u !== ds.uuid);
    const idx = order.indexOf(targetUuid);
    if (idx >= 0) order.splice(idx, 0, ds.uuid);
    else order.push(ds.uuid);
    setOrderOf(packType, order);
    touchOrder();
    log(`Reordered ${ds.uuid.slice(0, 8)} in ${packType === 'bp' ? 'BP' : 'RP'} order`);
  }

  function onZoneDrop(e: DragEvent, packType: PackKind) {
    e.preventDefault();
    const ds = dragSource;
    onRowDragEnd();
    if (!ds || ds.packType !== packType) return;
    const order = orderOf(packType).filter((u) => u !== ds.uuid);
    order.push(ds.uuid);
    setOrderOf(packType, order);
    touchOrder();
    log(`Moved ${ds.uuid.slice(0, 8)} to end of ${packType === 'bp' ? 'BP' : 'RP'} order`);
  }

  function dropBefore(packType: PackKind, uuid: string): boolean {
    return !!dragOverRow && dragOverRow.packType === packType && dragOverRow.uuid === uuid;
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
    manifestEditMode = 'visual';
    draftFromText(manifestEditText);
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
    if (manifestEditMode === 'visual' && !draftToText()) return;
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
        // Import is reviewed manually — cancel any pending autosave so it
        // cannot clobber the imported order before Save is clicked.
        clearTimeout(saveTimer);
        saveTimer = undefined;
        pendingSave = null;
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
    html += row('Name', h.name ?? '', 'text-strong font-medium');
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
        `<div class="flex justify-between text-deep-300 py-0.5 gap-2"><span class="min-w-0 truncate">${esc(mod.type ?? '')}${mod.description ? ' — <span class="text-deep-400">' + esc(mod.description) + '</span>' : ''}</span><span class="font-mono text-[10px] text-deep-400 shrink-0">${esc((mod.version ?? []).join('.'))}</span></div>`
      ).join('') + `</div>`;
    }
    if (m.dependencies?.length) {
      html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Dependencies (${m.dependencies.length})</span>`;
      html += m.dependencies.map((dep: any) =>
        `<div class="text-deep-300 py-0.5 font-mono text-[10px]">${esc(dep.uuid ?? dep.module_name ?? '')} ${dep.version ? '<span class="text-deep-400">v' + esc(dep.version.join('.')) + '</span>' : ''}</div>`
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

{#snippet sourceBadge(addon: Addon)}
  {#if addon.source === 'curseforge'}
    <span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-bedrock-500/15 text-bedrock-300 border border-bedrock-500/30"
          title="Installed from the CurseForge Store">CF</span>
  {:else}
    <span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-deep-700/40 text-deep-400 border border-deep-600/40"
          title="Manual upload">MAN</span>
  {/if}
{/snippet}

{#snippet typeBadge(addon: Addon)}
  <span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm border
               {kindOf(addon) === 'bp'
                 ? 'bg-bedrock-500/15 text-bedrock-300 border-bedrock-500/30'
                 : 'bg-teal-500/15 text-teal-300 border-teal-500/30'}"
        title={addon.pack_type === 'behavior_packs' ? 'Behavior pack' : 'Resource pack'}>
    {kindOf(addon) === 'bp' ? 'BP' : 'RP'}
  </span>
{/snippet}

{#snippet activePill(addon: Addon)}
  {#if isActive(addon)}
    <span class="shrink-0 px-1.5 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-green-500/15 text-green-400 border border-green-500/30"
          title="Active in this world's pack order">Active</span>
  {/if}
{/snippet}

{#snippet packIcon(p: Addon, size = 'w-10 h-10')}
  {#if iconFor(p)}
    <img src={iconFor(p)} alt="" class="pack-icon {size} rounded object-cover shrink-0 bg-deep-800 border border-deep-700/60"
         onerror={() => markIconBroken(p)} />
  {:else}
    <div class="{size} rounded bg-deep-800/80 border border-deep-700/60 flex items-center justify-center shrink-0">
      <Package size={16} class={packColorClass(kindOf(p))} />
    </div>
  {/if}
{/snippet}

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
      <div class="flex flex-wrap items-center gap-2 mb-1">
        <p class="text-deep-400 text-[10px] uppercase tracking-wider">
          Packs applied in load order — top packs load first. Drag rows or use the arrows.
        </p>
        <div class="flex-1"></div>
        {#if canManage && activeCount > 0}
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
          <button onclick={deactivateAll}
                  class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1"
                  title="Deactivate every pack in this world">
            <CircleOff size={11} /> Deactivate All
          </button>
        {/if}
      </div>
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
        {#each [{ kind: 'bp' as const, title: 'Behavior Packs', packs: activeBPPacks, missing: missingBP },
                { kind: 'rp' as const, title: 'Resource Packs', packs: activeRPPacks, missing: missingRP }] as section (section.kind)}
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
                  {@render packIcon(p)}
                  <button onclick={() => openPreview(p)} class="flex-1 min-w-0 text-left">
                    <p class="text-xs font-medium truncate hover:text-strong transition-colors">{p.name}</p>
                    <p class="text-[10px] text-deep-400 truncate">
                      v{versionStr(p.version)}{authorOf(p) ? ` · by ${authorOf(p)}` : ''}
                    </p>
                  </button>
                  <div class="flex items-center gap-1 shrink-0">
                    {@render typeBadge(p)}
                    {@render sourceBadge(p)}
                  </div>
                  {#if canManage}
                    <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                      <button onclick={() => moveActive(section.kind, p.uuid, -1)}
                              disabled={i === 0}
                              class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move up (loads earlier)"><ChevronUp size={12} /></button>
                      <button onclick={() => moveActive(section.kind, p.uuid, 1)}
                              disabled={i === section.packs.length - 1}
                              class="btn-ghost p-0.5 disabled:opacity-20 disabled:cursor-not-allowed" title="Move down (loads later)"><ChevronDown size={12} /></button>
                      <button onclick={() => removeFromOrder(section.kind, p.uuid)}
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
                    <button onclick={() => removeFromOrder(section.kind, uuid)}
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

    <!-- ======================== AVAILABLE TAB ======================== -->
    {:else if activeTab === 'available'}
      <div class="card">
        <!-- filter / sort bar -->
        <div class="flex flex-wrap items-center gap-2 mb-3">
          <div class="relative w-56">
            <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
            <input bind:value={searchAvail} class="input w-full text-xs py-1.5 pl-7" placeholder="Search name, author, UUID…" />
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
            {availableAddons.length} / {installedCount} shown
          </span>
        </div>

        <!-- bulk action bar -->
        {#if canManage && availSelected.size > 0}
          <div class="flex flex-wrap items-center gap-1.5 mb-3 p-2 rounded border border-bedrock-500/30 bg-bedrock-500/5">
            <span class="text-xs text-bedrock-300 font-medium">{availSelected.size} selected</span>
            <button onclick={() => bulkSetEnabled(true, worldBP.concat(worldRP).filter((p) => availSelected.has(p.path)))}
                    class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1"
                    title="Add every selected pack to the world's pack order">
              <Power size={11} /> Activate
            </button>
            <button onclick={() => bulkSetEnabled(false, worldBP.concat(worldRP).filter((p) => availSelected.has(p.path)))}
                    class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 text-deep-300 flex items-center gap-1"
                    title="Remove every selected pack from the order">
              <PowerOff size={11} /> Deactivate
            </button>
            <button onclick={() => askDelete(worldBP.concat(worldRP).filter((p) => availSelected.has(p.path)))}
                    class="btn-ghost text-xs px-2 py-1 rounded border border-red-500/30 text-red-400 flex items-center gap-1">
              <Trash2 size={11} /> Delete
            </button>
            <button onclick={() => (availSelected = new Set())} class="btn-ghost text-xs px-2 py-1">Clear</button>
          </div>
        {/if}

        <!-- the list -->
        <div role="list" aria-label="Available addons for the selected world" class="space-y-1">
          {#each availableAddons as p (p.path)}
            <div role="listitem"
                 class="group flex items-center gap-2.5 p-2 border border-deep-600/20 hover:border-deep-500/40 hover:bg-deep-800/40 rounded transition-colors {!p.valid ? 'opacity-50' : ''}"
                 title={!p.valid ? 'manifest.json missing or invalid' : p.name}>
              {#if canManage}
                <input type="checkbox" checked={availSelected.has(p.path)}
                       onchange={() => toggleAvailSelect(p.path)} class="accent-bedrock-500 shrink-0" aria-label="Select {p.name}" />
              {/if}
              {@render packIcon(p)}
              <button onclick={() => openPreview(p)} class="flex-1 min-w-0 text-left">
                <div class="flex items-center gap-1.5 flex-wrap">
                  <p class="text-xs font-medium truncate hover:text-strong transition-colors">{p.name}</p>
                  {@render typeBadge(p)}
                  {@render sourceBadge(p)}
                  {@render activePill(p)}
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
                    <button onclick={() => deactivate(p)}
                            class="btn-ghost p-1 text-deep-400 hover:text-deep-200" title="Deactivate (remove from the world's order)"><PowerOff size={13} /></button>
                  {:else}
                    <button onclick={() => activate(p)}
                            class="btn-ghost p-1 text-green-400 hover:text-green-300 opacity-60 group-hover:opacity-100 transition-opacity" title="Activate in this world"><Plus size={14} /></button>
                  {/if}
                  <button onclick={() => openPreview(p)}
                          class="btn-ghost p-1 opacity-0 group-hover:opacity-100 transition-opacity" title="Details"><Eye size={12} /></button>
                  <button onclick={() => askDelete([p])}
                          class="btn-ghost p-1 text-red-400 hover:text-red-300 opacity-0 group-hover:opacity-100 transition-opacity" title="Delete from disk"><Trash2 size={12} /></button>
                </div>
              {/if}
            </div>
          {:else}
            <div class="flex items-center justify-center py-10 text-center">
              <div>
                <Package size={22} class="text-deep-500 mx-auto mb-2" />
                <p class="text-deep-400 text-xs">
                  {#if searchAvail || availType !== 'all' || availSource !== 'all'}
                    No addons match the current filters
                  {:else}
                    No addons installed in “{selWorld}”
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

    <!-- ======================== DETAILS TAB ======================== -->
    {:else if activeTab === 'details'}
      <div class="card">
        <div class="flex flex-wrap items-center gap-3 mb-3">
          <div class="relative">
            <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
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
          <span class="text-deep-400 text-xs">{detailsSelected.size} / {detailsAddons.length}</span>
          {#if canManage && detailsSelected.size > 0}
            <div class="flex flex-wrap items-center gap-1 ml-auto">
              {#if selWorld !== ALL_WORLDS}
                <button onclick={() => bulkSetEnabled(true)} class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1" title="Add to the world's pack order">
                  <Power size={11} /> Enable
                </button>
                <button onclick={() => bulkSetEnabled(false)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 text-deep-300 flex items-center gap-1" title="Remove from the world's pack order">
                  <PowerOff size={11} /> Disable
                </button>
              {/if}
              <button onclick={() => bulkBumpVersion('major')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Major</button>
              <button onclick={() => bulkBumpVersion('minor')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Minor</button>
              <button onclick={() => bulkBumpVersion('patch')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Patch</button>
              <button onclick={askBulkRandomize} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
                <Shuffle size={11} /> Randomize UUIDs
              </button>
              <button onclick={() => bulkToggleHide()} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Toggle Hide</button>
              <button onclick={() => askDelete(allBp.concat(allRp).filter((p) => detailsSelected.has(p.uuid)))}
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
              {#if selWorld === ALL_WORLDS}
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
              {#each detailsAddons as a (a.path)}
                <tr class="border-b border-deep-700/20 hover:bg-deep-800/30 {!a.valid ? 'opacity-50' : ''}">
                  <td class="py-1.5 px-2">
                    <input type="checkbox" checked={detailsSelected.has(a.uuid)}
                           onchange={() => toggleDetailsSelect(a.uuid)} class="accent-bedrock-500" />
                  </td>
                  <td class="py-1.5 px-2">
                    {#if iconFor(a)}
                      <img src={iconFor(a)} alt="" class="pack-icon w-7 h-7 rounded-sm object-cover bg-deep-800"
                           onerror={() => markIconBroken(a)} />
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
                    <button onclick={() => openPreview(a)} class="hover:text-bedrock-300 transition-colors text-left">
                      {a.name}
                      {#if !a.valid}<span class="text-red-500 text-[10px] ml-1">(invalid)</span>{/if}
                    </button>
                    <span class="ml-1.5 inline-flex align-middle">{@render sourceBadge(a)}</span>
                  </td>
                  <td class="py-1.5 px-3 text-deep-300">{authorOf(a) || '—'}</td>
                  {#if selWorld === ALL_WORLDS}
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
                      <button onclick={() => openPreview(a)} class="btn-ghost p-0.5" title="Preview"><Eye size={11} /></button>
                      {#if canManage}
                        <button onclick={() => openManifestEditor(a)} class="btn-ghost p-0.5" title="Edit manifest"><Edit size={11} /></button>
                        <button onclick={() => openVersionEditor(a)} class="btn-ghost p-0.5" title="Version"><Hash size={11} /></button>
                        <button onclick={() => openUuidEditor(a, 'randomize')} class="btn-ghost p-0.5" title="Randomize UUID"><Shuffle size={11} /></button>
                        <button onclick={() => toggleHidden(a.uuid)} class="btn-ghost p-0.5" title="Toggle hide"><AlertTriangle size={11} /></button>
                        <button onclick={() => askDelete([a])} class="btn-ghost p-0.5 text-red-400 hover:text-red-300" title="Delete from disk"><Trash2 size={11} /></button>
                      {/if}
                    </div>
                  </td>
                </tr>
              {:else}
                <tr><td colspan={selWorld === ALL_WORLDS ? 11 : 10} class="text-center py-8 text-deep-400">No addons match the current filter</td></tr>
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
  <!-- backdrop on mobile -->
  <button type="button" class="fixed inset-0 z-40 bg-black/40 lg:hidden cursor-pointer" onclick={closePreview} aria-label="Close preview"></button>
  <!-- drawer -->
  <aside class="fixed top-0 right-0 z-50 h-full w-full max-w-md bg-deep-900 border-l-2 border-deep-600/50 shadow-2xl flex flex-col animate-slide-in">
    <!-- drawer header -->
    <div class="flex items-center justify-between p-4 border-b border-deep-700/50 shrink-0">
      {#if iconFor(a)}
        <img src={iconFor(a)} alt="" class="pack-icon w-10 h-10 rounded object-cover bg-deep-800 mr-3 shrink-0 border border-deep-700/60"
             onerror={() => markIconBroken(a)} />
      {:else}
        <div class="w-10 h-10 rounded bg-deep-800/80 flex items-center justify-center mr-3 shrink-0">
          <Package size={18} class="text-deep-400" />
        </div>
      {/if}
      <div class="min-w-0">
        <p class="text-[10px] text-deep-400 font-mono uppercase tracking-wider">
          {a.pack_type === 'behavior_packs' ? 'Behavior Pack' : 'Resource Pack'} · v{versionStr(a.version)}
        </p>
        <div class="mt-0.5 flex items-center gap-1.5 flex-wrap">
          {@render sourceBadge(a)}
          {@render activePill(a)}
          {#if a.source === 'curseforge' && a.store?.project_name}
            <span class="text-[10px] text-deep-400 truncate" title={`CurseForge project ${String(a.store.project_id)}`}>{String(a.store.project_name)}</span>
          {/if}
        </div>
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
        {#if isActive(a)}
          <button onclick={() => deactivate(a)} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Remove from the world's pack order">
            <PowerOff size={12} /> Deactivate
          </button>
        {:else}
          <button onclick={() => activate(a)} class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1" title="Add to the world's pack order">
            <Power size={12} /> Activate
          </button>
        {/if}
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
          <AlertTriangle size={11} /> {hiddenUuids.has(a.uuid) ? 'Unhide' : 'Hide'}
        </button>
        <button onclick={() => { closePreview(); askDelete([a]); }}
                class="btn-ghost text-xs px-2 py-1 rounded border border-red-500/30 text-red-400 flex items-center gap-1 ml-auto">
          <Trash2 size={11} /> Delete
        </button>
      </div>
    {/if}
  </aside>
{/if}

<!-- ======================== MODALS ======================== -->

<!-- Manifest Editor Modal -->
{#if manifestEditorOpen}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) manifestEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (manifestEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-2xl shadow-block-lg">
      <div class="flex items-center justify-between mb-3">
        <h2 class="text-sm font-bold text-strong uppercase tracking-widest">Edit Manifest</h2>
        <div class="flex border border-deep-600/40 rounded overflow-hidden">
          <button onclick={() => setManifestMode('visual')}
                  class="px-3 py-1 text-[10px] uppercase tracking-wider {manifestEditMode === 'visual' ? 'bg-bedrock-500/20 text-bedrock-300' : 'text-deep-400 hover:text-strong'}">Visual</button>
          <button onclick={() => setManifestMode('raw')}
                  class="px-3 py-1 text-[10px] uppercase tracking-wider border-l border-deep-600/40 {manifestEditMode === 'raw' ? 'bg-bedrock-500/20 text-bedrock-300' : 'text-deep-400 hover:text-strong'}">Raw</button>
        </div>
      </div>

      {#if manifestEditMode === 'visual'}
        <div class="space-y-3 max-h-96 overflow-y-auto pr-1">
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Name</span>
            <input bind:value={manifestDraft.name} class="input w-full text-xs" />
          </label>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Description</span>
            <textarea bind:value={manifestDraft.description} class="input w-full h-16 text-xs" ></textarea>
          </label>
          <div class="grid grid-cols-2 gap-3">
            <div>
              <span class="text-deep-400 text-[10px] uppercase tracking-wider block mb-1">Version</span>
              <div class="grid grid-cols-3 gap-1.5">
                <input type="number" min="0" bind:value={manifestDraft.v0} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={manifestDraft.v1} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={manifestDraft.v2} class="input text-xs text-center" />
              </div>
            </div>
            <div>
              <span class="text-deep-400 text-[10px] uppercase tracking-wider block mb-1">Min engine version</span>
              <div class="grid grid-cols-3 gap-1.5">
                <input type="number" min="0" bind:value={manifestDraft.m0} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={manifestDraft.m1} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={manifestDraft.m2} class="input text-xs text-center" />
              </div>
            </div>
          </div>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">UUID</span>
            <div class="flex gap-2">
              <input bind:value={manifestDraft.uuid} class="input flex-1 text-xs font-mono" />
              <button onclick={() => (manifestDraft.uuid = crypto.randomUUID())}
                      class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1 shrink-0">
                <Shuffle size={11} /> New
              </button>
            </div>
          </label>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Authors <span class="text-deep-400">(comma-separated)</span></span>
            <input bind:value={manifestDraft.authors} class="input w-full text-xs" placeholder="Jane Doe, DevCo" />
          </label>
          <p class="text-[10px] text-deep-400">Visual edits rewrite the JSON document; other fields (modules, dependencies, capabilities) are preserved as-is.</p>
        </div>
      {:else}
        <textarea bind:value={manifestEditText} oninput={validateManifestJson}
                  class="w-full h-80 bg-deep-950 text-xs font-mono p-3 border rounded resize-y
                         {manifestEditValid ? 'border-deep-600/30' : 'border-red-500/50'}"
                  spellcheck="false"></textarea>
        {#if !manifestEditValid}
          <p class="text-red-400 text-xs mt-1">Invalid JSON — cannot save</p>
        {/if}
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
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) versionEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (versionEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-1">Version Editor</h2>
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
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) uuidEditorOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (uuidEditorOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-lg shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-3">
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
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) renameOpen = false; }}
       onkeydown={(e) => e.key === 'Escape' && (renameOpen = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-3">Rename Addon</h2>
      <label for="rename-input" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">New Name</label>
      <input id="rename-input" bind:value={renameValue} class="input w-full text-xs py-1.5 mb-4" placeholder="Addon folder name" />
      <div class="flex justify-end gap-2">
        <button onclick={() => renameOpen = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={() => saveRename()} class="btn-primary text-xs px-4 py-1.5">Rename</button>
      </div>
    </div>
  </div>
{/if}

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

<style>
  @keyframes slide-in {
    from { transform: translateX(100%); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }
  .animate-slide-in {
    animation: slide-in 0.2s ease-out;
  }
  /* Minecraft pack icons are pixel art — keep them crisp when scaled up. */
  .pack-icon {
    image-rendering: pixelated;
  }
</style>
