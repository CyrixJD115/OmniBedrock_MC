<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { formatBytes } from '$lib/utils';
  import { savedWorld, saveWorld } from '$lib/worldSync';
  import { theme, activeMode } from '$lib/theme';
  import type { CfProject, CfFile, StoreMeta, StoreSearchResponse } from '$types/index';
  import {
    getCachedSearch, isMetaFresh, loadCachedMeta, saveMeta, saveSearch, searchCacheKey,
  } from '$stores/storeCache';
  import {
    Search, Store as StoreIcon, Package, Loader2, X, ChevronLeft, ChevronRight,
    ExternalLink, Download, LayoutGrid, Puzzle, Map as MapIcon, Palette, Braces, Shirt, AlertTriangle, Globe
  } from '@lucide/svelte';

  const CLASS_TABS = [
    { value: 'all', label: 'All', icon: LayoutGrid },
    { value: 'addons', label: 'Addons', icon: Puzzle },
    { value: 'maps', label: 'Maps', icon: MapIcon },
    { value: 'texture-packs', label: 'Texture Packs', icon: Palette },
    { value: 'scripts', label: 'Scripts', icon: Braces },
    { value: 'skins', label: 'Skins', icon: Shirt },
  ] as const;

  const SORTS = [
    { value: 'relevancy', label: 'Relevancy' },
    { value: 'popularity', label: 'Popularity' },
    { value: 'latest-update', label: 'Latest Update' },
    { value: 'creation-date', label: 'Creation Date' },
    { value: 'total-downloads', label: 'Total Downloads' },
  ];

  // ─── state ────────────────────────────────────────────────────────────────
  let worlds = $state<string[]>([]);
  let selWorld = $state('');
  let meta = $state<StoreMeta | null>(null);
  let metaError = $state('');

  let query = $state('');
  let activeClass = $state<string>('all');
  let sort = $state('relevancy');
  let pageSize = $state(20);
  let version = $state('');
  let category = $state('');
  let page = $state(1);

  let results = $state<CfProject[]>([]);
  let totalCount = $state(0);
  let loading = $state(false);
  let error = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchAbort: AbortController | undefined;
  let brokenThumbs = $state<Set<number>>(new Set());

  // detail modal
  let detail = $state<CfProject | null>(null);
  let detailDescription = $state('');
  let detailLoading = $state(false);
  let detailFiles = $state<CfFile[]>([]);
  let detailFilesTotal = $state(0);
  let detailFilesPage = $state(1);
  let detailVersion = $state('');
  let filesLoading = $state(false);
  let installTarget = $state<'auto' | 'behavior_packs' | 'resource_packs' | 'world'>('auto');
  let installingFileId = $state<number | null>(null);

  const canInstall = $derived($userPermissions.includes('STORE_INSTALL'));
  const canEditSettings = $derived($userPermissions.includes('SETTINGS_EDIT'));
  const totalPages = $derived(Math.max(1, Math.ceil(totalCount / pageSize)));
  const subCategories = $derived.by(() => {
    if (activeClass === 'all' || !meta) return [];
    const classInfo = meta.classes.find((c) => c.slug === activeClass);
    return classInfo ? meta.categories[String(classInfo.id)] ?? [] : [];
  });
  const descriptionDoc = $derived.by(() => {
    // Rebuild when the palette changes so the sandboxed iframe matches.
    void $theme; void $activeMode;
    // The iframe cannot inherit CSS vars, so bake the resolved values in.
    // NOTE: no literal <style> tag may appear in this source — the Svelte
    // CSS pipeline would try to parse the (interpolated) rules as CSS.
    const cssVar = (name: string) =>
      `rgb(${getComputedStyle(document.documentElement).getPropertyValue(name).trim()})`;
    if (!detailDescription) return '';
    const body = detailDescription
      // Belt and braces on top of the script-less sandboxed iframe.
      .replace(/<script[\s\S]*?<\/script>/gi, '')
      .replace(/<style[\s\S]*?<\/style>/gi, '')
      .replace(/<iframe[\s\S]*?<\/iframe>/gi, '');
    const rules = [
      `body { background:${cssVar('--deep-900')}; color:${cssVar('--deep-100')}; font-family:'Montserrat',sans-serif; font-size:13px; line-height:1.55; padding:14px; margin:0; }`,
      'img { max-width:100%; height:auto; border-radius:4px; }',
      `a { color:${cssVar('--bedrock-400')}; }`,
      `h1,h2,h3,h4 { color:${cssVar('--color-strong')}; margin:0.8em 0 0.3em; }`,
      `code, pre { background:${cssVar('--deep-800')}; border:1px solid ${cssVar('--deep-600')}; border-radius:4px; padding:1px 4px; }`,
      'pre { padding:8px; overflow-x:auto; }',
      `table { border-collapse:collapse; } td,th { border:1px solid ${cssVar('--deep-600')}; padding:4px 8px; }`,
      `::-webkit-scrollbar { width:8px; height:8px; } ::-webkit-scrollbar-thumb { background:${cssVar('--deep-600')}; border-radius:4px; }`,
    ].join('\n');
    const styleTag = `<${'style'}>${rules}</${'style'}>`;
    return `<!DOCTYPE html><html><head><base target="_blank">${styleTag}</head><body>${body}</body></html>`;
  });

  // ─── helpers ──────────────────────────────────────────────────────────────
  function formatCount(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return String(n);
  }
  function fmtDate(iso?: string): string {
    if (!iso) return '—';
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString();
  }
  function releaseClass(type: number): string {
    if (type === 1) return 'text-green-400 border-green-500/30 bg-green-500/10';
    if (type === 2) return 'text-yellow-400 border-yellow-500/30 bg-yellow-500/10';
    return 'text-red-400 border-red-500/30 bg-red-500/10';
  }
  function releaseLabel(type: number): string {
    return type === 1 ? 'Release' : type === 2 ? 'Beta' : type === 3 ? 'Alpha' : 'Unknown';
  }
  function thumbUrl(p: CfProject): string | null {
    const url = p.logo?.thumbnailUrl || p.logo?.url;
    return url ? api.storeImageUrl(url) : null;
  }
  function needsWorld(): boolean {
    return installTarget !== 'world';
  }

  // ─── data loading ─────────────────────────────────────────────────────────
  async function loadMeta() {
    // Metadata (classes, categories, versions) barely changes: hydrate from
    // the sessionStorage cache instantly and only refresh it in the
    // background once it ages out.
    const cached = loadCachedMeta();
    if (cached) meta = cached;
    if (cached && isMetaFresh()) return;
    try {
      const fresh = await api.storeMeta();
      meta = fresh;
      saveMeta(fresh);
      metaError = '';
    } catch (e: any) {
      if (!cached) metaError = e.message ?? 'Failed to load Store metadata';
    }
  }

  function applySearch(response: StoreSearchResponse) {
    results = response.results;
    totalCount = response.pagination.totalCount;
  }

  async function load() {
    const params = {
      class: activeClass === 'all' ? '' : activeClass,
      q: query,
      page,
      pageSize,
      sort,
      version,
      category,
    };
    const key = searchCacheKey(params);
    const cached = getCachedSearch(key);
    if (cached) {
      error = '';
      applySearch(cached.response);
      if (cached.fresh) return;
    }
    // Only the latest filter state matters: abort whatever search is still
    // in flight so slow stale responses can't clobber fresh ones.
    const controller = new AbortController();
    searchAbort?.abort();
    searchAbort = controller;
    loading = true;
    try {
      const response = await api.storeSearch(params, controller.signal);
      saveSearch(key, response);
      if (searchAbort !== controller) return; // superseded mid-flight
      error = '';
      applySearch(response);
    } catch (e: any) {
      if (e?.name === 'AbortError' || searchAbort !== controller) return;
      error = e.message ?? 'Search failed';
      if (!cached) {
        results = [];
        totalCount = 0;
      }
    } finally {
      if (searchAbort === controller) loading = false;
    }
  }

  function debounceSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      page = 1;
      load();
    }, 400);
  }

  function setClass(value: string) {
    if (activeClass === value) return;
    activeClass = value;
    category = '';
    page = 1;
    load();
  }

  async function openDetail(p: CfProject) {
    detail = p;
    detailDescription = '';
    detailFiles = [];
    detailFilesTotal = 0;
    detailFilesPage = 1;
    detailVersion = '';
    installTarget = p.classId === 6913 ? 'world' : 'auto';
    detailLoading = true;
    const projectId = p.id;
    const [description, files] = await Promise.allSettled([
      api.storeDescription(projectId),
      api.storeFiles(projectId, 1, pageSize, ''),
    ]);
    if (detail?.id !== projectId) return; // modal switched mid-flight
    if (description.status === 'fulfilled') detailDescription = description.value.description;
    if (files.status === 'fulfilled') {
      detailFiles = files.value.files;
      detailFilesTotal = files.value.total;
    } else {
      addToast(`Failed to load files: ${files.reason?.message ?? 'unknown error'}`, 'error');
    }
    detailLoading = false;
  }

  async function loadDetailFiles() {
    if (!detail) return;
    filesLoading = true;
    try {
      const response = await api.storeFiles(detail.id, detailFilesPage, pageSize, detailVersion);
      detailFiles = response.files;
      detailFilesTotal = response.total;
    } catch (e: any) {
      addToast(`Failed to load files: ${e.message}`, 'error');
    } finally {
      filesLoading = false;
    }
  }

  async function installFile(file: CfFile) {
    if (!detail || installingFileId !== null) return;
    if (needsWorld() && (!selWorld || selWorld === '__all__')) {
      addToast('Select a world first (top bar) to install packs into', 'error');
      return;
    }
    installingFileId = file.id;
    try {
      const result = await api.storeInstall({
        project_id: detail.id,
        project_slug: detail.slug,
        file_id: file.id,
        world: selWorld || '',
        target: installTarget,
      });
      addToast(result.message, 'success');
    } catch (e: any) {
      const raw = e.message ?? 'Install failed';
      // request() wraps errors in "API Error <status>: <json>"; surface the detail.
      let message = raw;
      try {
        const jsonStart = raw.indexOf('{');
        if (jsonStart >= 0) message = JSON.parse(raw.slice(jsonStart)).detail ?? raw;
      } catch { /* keep raw */ }
      addToast(message, 'error', 9000);
    } finally {
      installingFileId = null;
    }
  }

  function closeDetail() {
    detail = null;
    detailDescription = '';
    detailFiles = [];
  }

  onMount(() => {
    // Worlds, metadata, and the first search are independent — fire them all
    // at once instead of serially awaiting the world list first.
    loadMeta();
    load();
    api.listWorlds()
      .then((list) => {
        worlds = list;
        if (list.length > 0 && !selWorld) {
          // Default to the world the user manages on the Addons page so Store
          // installs always land in that world's Available list.
          const saved = savedWorld();
          selWorld = saved && list.includes(saved) ? saved : list[0];
        }
      })
      .catch(() => { /* world list is optional for browsing */ });
  });
</script>

<div class="space-y-4">
  <!-- header -->
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">CurseForge Store</h1>
      <div class="pixel-divider mt-2 w-40"></div>
    </div>
    <div class="flex flex-wrap items-center gap-3">
      <label for="store-world" class="text-deep-400 text-xs uppercase tracking-wider">Install world</label>
      <select id="store-world" bind:value={selWorld} onchange={() => saveWorld(selWorld)} class="input w-56 text-xs py-1.5">
        {#if worlds.length === 0}<option value="">No worlds found</option>{/if}
        {#each worlds as w}<option value={w}>{w}</option>{/each}
      </select>
    </div>
  </div>

  <!-- search + filters -->
  <div class="card">
    <div class="flex flex-wrap items-center gap-2">
      <div class="relative flex-1 min-w-56">
        <Search size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
        <input bind:value={query}
               oninput={debounceSearch}
               onkeydown={(e) => { if (e.key === 'Enter') { clearTimeout(searchTimer); page = 1; load(); } }}
               class="input w-full text-sm py-2 pl-9"
               placeholder="Search CurseForge Bedrock content..."
               aria-label="Search CurseForge" />
      </div>
      <select bind:value={sort} onchange={() => { page = 1; load(); }} class="input w-40 text-xs py-2" aria-label="Sort by">
        {#each SORTS as s}<option value={s.value}>{s.label}</option>{/each}
      </select>
      <select bind:value={version} onchange={() => { page = 1; load(); }} class="input w-32 text-xs py-2" aria-label="Game version">
        <option value="">All versions</option>
        {#each meta?.versions ?? [] as v}<option value={v}>{v}</option>{/each}
      </select>
      {#if activeClass !== 'all'}
        <select bind:value={category} onchange={() => { page = 1; load(); }} class="input w-40 text-xs py-2" aria-label="Category">
          <option value="">All categories</option>
          {#each subCategories as c}<option value={String(c.id)}>{c.name}</option>{/each}
        </select>
      {/if}
      <select bind:value={pageSize} onchange={() => { page = 1; load(); }} class="input w-24 text-xs py-2" aria-label="Results per page">
        {#each [10, 20, 50] as size}<option value={size}>{size} / page</option>{/each}
      </select>
    </div>

    <!-- class tabs -->
    <div class="flex flex-wrap gap-1 mt-3">
      {#each CLASS_TABS as tab}
        <button onclick={() => setClass(tab.value)}
                class="flex items-center gap-1.5 text-xs px-4 py-2 uppercase tracking-wider font-semibold rounded-sm border-b-2 transition-colors
                       {activeClass === tab.value ? 'bg-deep-800/60 text-strong border-bedrock-500' : 'text-deep-400 border-transparent hover:text-deep-200 hover:bg-deep-800/40'}">
          <tab.icon size={13} />
          {tab.label}
        </button>
      {/each}
    </div>
  </div>

  <!-- configuration guidance -->
  {#if error}
    <div class="card border-yellow-600/40 bg-yellow-500/5">
      <div class="flex items-start gap-3">
        <AlertTriangle size={18} class="text-yellow-400 shrink-0 mt-0.5" />
        <div class="text-sm">
          <p class="font-semibold text-yellow-300 mb-1">Store unavailable</p>
          <p class="text-deep-300">{error}</p>
          {#if error.includes('not configured')}
            <p class="text-deep-400 text-xs mt-2">
              {#if canEditSettings}
                Enter your API key in <a href="/settings" class="text-bedrock-300 underline">Settings → CurseForge</a> — it applies immediately, no restart needed.
              {:else}
                Ask an admin to set the <code class="text-bedrock-300">CURSEFORGE_API_KEY</code> in Settings → CurseForge.
              {/if}
              Get a free key at <span class="text-bedrock-300">console.curseforge.com</span>.
            </p>
          {:else if error.includes('denied search')}
            <p class="text-deep-400 text-xs mt-2">
              Your key is valid for project and file lookups; CurseForge approves /mods/search access
              for developer keys from the <span class="text-bedrock-300">CurseForge developer console</span>. Until then,
              installs by project are still possible through a project's direct CurseForge link.
            </p>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <!-- results -->
  {#if loading && results.length === 0}
    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
      {#each Array(6) as _, i (i)}
        <div class="card !p-3 animate-pulse">
          <div class="flex gap-3">
            <div class="w-14 h-14 bg-deep-700/40 rounded-sm"></div>
            <div class="flex-1 space-y-2 py-1">
              <div class="h-3 bg-deep-700/40 rounded w-3/4"></div>
              <div class="h-2.5 bg-deep-700/30 rounded w-1/2"></div>
              <div class="h-2.5 bg-deep-700/30 rounded w-1/3"></div>
            </div>
          </div>
          <div class="mt-3 space-y-1.5">
            <div class="h-2.5 bg-deep-700/30 rounded"></div>
            <div class="h-2.5 bg-deep-700/30 rounded w-5/6"></div>
          </div>
        </div>
      {/each}
    </div>
  {:else if results.length > 0}
    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3 transition-opacity {loading ? 'opacity-40 pointer-events-none' : 'opacity-100'}">
      {#each results as p (p.id)}
        <button class="card !p-3 text-left group hover:border-bedrock-500/50 transition-colors flex flex-col cursor-pointer"
                onclick={() => openDetail(p)}>
          <div class="flex gap-3">
            {#if thumbUrl(p) && !brokenThumbs.has(p.id)}
              <img src={thumbUrl(p) ?? ''} alt="" loading="lazy" decoding="async"
                   class="w-14 h-14 rounded-sm object-cover bg-deep-900 border border-deep-700/50 shrink-0"
                   onerror={() => brokenThumbs = new Set([...brokenThumbs, p.id])} />
            {:else}
              <div class="w-14 h-14 rounded-sm bg-deep-900 border border-deep-700/50 shrink-0 flex items-center justify-center">
                <Package size={20} class="text-deep-400" />
              </div>
            {/if}
            <div class="min-w-0 flex-1">
              <p class="text-sm font-semibold text-strong truncate group-hover:text-bedrock-300 transition-colors">{p.name}</p>
              <p class="text-[11px] text-deep-400 truncate">{p.authors.map((a) => a.name).join(', ') || 'Unknown author'}</p>
              <p class="text-[11px] text-deep-400">{formatCount(p.downloadCount)} downloads · {fmtDate(p.dateModified)}</p>
            </div>
          </div>
          <p class="text-xs text-deep-400 line-clamp-2 mt-2 flex-1">{p.summary}</p>
          <div class="flex flex-wrap gap-1 mt-2">
            {#each p.categories.slice(0, 3) as c (c.id)}
              <span class="text-[9px] px-1.5 py-0.5 bg-deep-700/50 text-deep-300 rounded-sm uppercase tracking-wider">{c.name}</span>
            {/each}
          </div>
        </button>
      {/each}
    </div>

    <!-- pagination -->
    {#if totalPages > 1 || totalCount > 0}
      <div class="flex items-center justify-center gap-3 pt-1">
        <button onclick={() => { page -= 1; load(); }} disabled={page <= 1 || loading}
                class="btn-secondary text-xs px-3 py-1.5 rounded flex items-center gap-1 disabled:opacity-30 disabled:cursor-not-allowed">
          <ChevronLeft size={13} /> Prev
        </button>
        <span class="text-xs text-deep-400">Page {page} of {totalPages} · {formatCount(totalCount)} results{loading ? ' · updating…' : ''}</span>
        <button onclick={() => { page += 1; load(); }} disabled={page >= totalPages || loading}
                class="btn-secondary text-xs px-3 py-1.5 rounded flex items-center gap-1 disabled:opacity-30 disabled:cursor-not-allowed">
          Next <ChevronRight size={13} />
        </button>
      </div>
    {/if}
  {:else if !error}
    <div class="card flex flex-col items-center justify-center py-16 text-center">
      <StoreIcon size={32} class="text-deep-500 mb-3" />
      <p class="text-deep-400 text-sm">No results found</p>
      <p class="text-deep-400 text-xs mt-1">Try a different search, class, or version filter</p>
    </div>
  {/if}
</div>

<!-- ======================== DETAIL MODAL ======================== -->
{#if detail}
    <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       role="dialog" aria-modal="true" tabindex="-1" aria-label={`Details for ${detail.name}`}
       onclick={(e) => { if (e.target === e.currentTarget) closeDetail(); }}
       onkeydown={(e) => { if (e.key === 'Escape') closeDetail(); }}>
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-4xl shadow-block-lg max-h-[90vh] flex flex-col">
      <!-- header -->
      <div class="flex items-start gap-3 border-b border-deep-700/50 pb-3 shrink-0">
        {#if thumbUrl(detail) && !brokenThumbs.has(detail.id)}
          <img src={thumbUrl(detail) ?? ''} alt="" decoding="async"
               class="w-16 h-16 rounded object-cover bg-deep-800 border border-deep-700/50 shrink-0"
               onerror={() => brokenThumbs = new Set([...brokenThumbs, detail!.id])} />
        {:else}
          <div class="w-16 h-16 rounded bg-deep-800 border border-deep-700/50 flex items-center justify-center shrink-0">
            <Package size={24} class="text-deep-400" />
          </div>
        {/if}
        <div class="min-w-0 flex-1">
          <h2 class="text-base font-bold text-strong truncate">{detail.name}</h2>
          <p class="text-xs text-deep-400 mt-0.5">
            {detail.authors.map((a) => a.name).join(', ') || 'Unknown author'}
            · {formatCount(detail.downloadCount)} downloads
            · Updated {fmtDate(detail.dateModified)}
          </p>
          <div class="flex flex-wrap gap-1 mt-1.5">
            {#each detail.categories.slice(0, 6) as c (c.id)}
              <span class="text-[9px] px-1.5 py-0.5 bg-deep-700/50 text-deep-300 rounded-sm uppercase tracking-wider">{c.name}</span>
            {/each}
          </div>
        </div>
        {#if detail.links?.websiteUrl}
          <a href={detail.links.websiteUrl} target="_blank" rel="noopener noreferrer"
             class="btn-ghost p-1.5 shrink-0" title="View on CurseForge">
            <ExternalLink size={16} />
          </a>
        {/if}
        <button onclick={closeDetail} class="btn-ghost p-1.5 shrink-0" title="Close"><X size={16} /></button>
      </div>

      <!-- install controls -->
      <div class="flex flex-wrap items-center gap-2 py-3 border-b border-deep-700/30 shrink-0">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Install target</span>
        <select bind:value={installTarget} class="input w-52 text-xs py-1.5" disabled={!canInstall}>
          <option value="auto">Auto-detect (packs / world)</option>
          <option value="behavior_packs">Into world · behavior_packs</option>
          <option value="resource_packs">Into world · resource_packs</option>
          <option value="world">As new world (maps)</option>
        </select>
        {#if needsWorld()}
          <span class="text-[10px] text-deep-400 flex items-center gap-1">
            <Globe size={11} /> {selWorld ? `→ ${selWorld}` : 'no world selected'}
          </span>
        {/if}
      </div>

      <!-- scrollable body -->
      <div class="flex-1 overflow-y-auto min-h-0 space-y-4 pt-3">
        <!-- description -->
        <section>
          <h3 class="text-xs font-bold text-deep-300 uppercase tracking-wider mb-2">Description</h3>
          {#if detailLoading}
            <div class="space-y-2 animate-pulse">
              {#each Array(4) as _, i (i)}<div class="h-3 bg-deep-700/40 rounded w-full"></div>{/each}
            </div>
          {:else if descriptionDoc}
            <iframe title="Project description" sandbox="" class="w-full h-72 rounded border border-deep-700/50 bg-deep-950"
                    srcdoc={descriptionDoc}></iframe>
          {:else}
            <p class="text-xs text-deep-400">{detail.summary || 'No description available.'}</p>
          {/if}
        </section>

        <!-- files -->
        <section>
          <div class="flex flex-wrap items-center gap-2 mb-2">
            <h3 class="text-xs font-bold text-deep-300 uppercase tracking-wider">Files ({detailFilesTotal})</h3>
            <div class="flex-1"></div>
            <select bind:value={detailVersion} onchange={() => { detailFilesPage = 1; loadDetailFiles(); }}
                    class="input w-36 text-xs py-1" disabled={!canInstall} aria-label="Filter files by game version">
              <option value="">All versions</option>
              {#each meta?.versions ?? [] as v}<option value={v}>{v}</option>{/each}
            </select>
          </div>
          {#if filesLoading}
            <div class="space-y-2 animate-pulse">
              {#each Array(3) as _, i (i)}<div class="h-10 bg-deep-700/40 rounded"></div>{/each}
            </div>
          {:else if detailFiles.length === 0}
            <p class="text-xs text-deep-400 py-4 text-center">No files match the current filters</p>
          {:else}
            <div class="space-y-1.5">
              {#each detailFiles as file (file.id)}
                <div class="flex items-center gap-2 p-2 border border-deep-700/40 hover:border-deep-500/50 rounded bg-deep-800/30">
                  <div class="min-w-0 flex-1">
                    <p class="text-xs font-medium text-deep-100 truncate" title={file.displayName}>{file.displayName || file.fileName}</p>
                    <div class="flex flex-wrap items-center gap-1.5 mt-0.5">
                      <span class="text-[9px] px-1 py-px rounded-sm border uppercase tracking-wider {releaseClass(file.releaseType)}">{releaseLabel(file.releaseType)}</span>
                      {#each file.gameVersions.slice(0, 2) as gv}<span class="text-[9px] px-1 py-px bg-deep-700/50 text-deep-300 rounded-sm font-mono">{gv}</span>{/each}
                      <span class="text-[10px] text-deep-400">{formatBytes(file.fileLength)}</span>
                      <span class="text-[10px] text-deep-400">·</span>
                      <span class="text-[10px] text-deep-400">{formatCount(file.downloadCount)} downloads</span>
                      <span class="text-[10px] text-deep-400">·</span>
                      <span class="text-[10px] text-deep-400">{fmtDate(file.fileDate)}</span>
                    </div>
                  </div>
                  {#if canInstall}
                    <button onclick={() => installFile(file)}
                            disabled={installingFileId !== null}
                            class="btn-primary text-xs px-3 py-1.5 rounded flex items-center gap-1.5 shrink-0 disabled:opacity-40">
                      {#if installingFileId === file.id}
                        <Loader2 size={12} class="animate-spin" /> Installing…
                      {:else}
                        <Download size={12} /> Install
                      {/if}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
            {#if detailFilesTotal > pageSize}
              <div class="flex items-center justify-center gap-3 mt-3">
                <button onclick={() => { detailFilesPage -= 1; loadDetailFiles(); }} disabled={detailFilesPage <= 1}
                        class="btn-secondary text-xs px-3 py-1 rounded flex items-center gap-1 disabled:opacity-30">
                  <ChevronLeft size={12} /> Prev
                </button>
                <span class="text-xs text-deep-400">Files page {detailFilesPage} of {Math.max(1, Math.ceil(detailFilesTotal / pageSize))}</span>
                <button onclick={() => { detailFilesPage += 1; loadDetailFiles(); }} disabled={detailFilesPage >= Math.ceil(detailFilesTotal / pageSize)}
                        class="btn-secondary text-xs px-3 py-1 rounded flex items-center gap-1 disabled:opacity-30">
                  Next <ChevronRight size={12} />
                </button>
              </div>
            {/if}
          {/if}
        </section>
      </div>
    </div>
  </div>
{/if}
