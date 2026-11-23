<script lang="ts">
  import { onMount } from 'svelte';
  import { api, authsrc } from '$lib/api/client';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { savedWorld, saveWorld } from '$lib/worldSync';
  import type { CfProject, StoreMeta, StoreSearchResponse } from '$types/index';
  import { formatCount, fmtDate, thumbUrl } from '$lib/store/format';
  import ProjectDetailModal from '$lib/components/store/ProjectDetailModal.svelte';
  import {
    getCachedSearch, isMetaFresh, loadCachedMeta, saveMeta, saveSearch, searchCacheKey,
  } from '$stores/storeCache';
  import {
    Search, Store as StoreIcon, Package, ChevronLeft, ChevronRight,
    LayoutGrid, Puzzle, Map as MapIcon, Palette, Braces, Shirt, AlertTriangle
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

  const canInstall = $derived($userPermissions.includes('STORE_INSTALL'));
  const canEditSettings = $derived($userPermissions.includes('SETTINGS_EDIT'));
  const totalPages = $derived(Math.max(1, Math.ceil(totalCount / pageSize)));
  const subCategories = $derived.by(() => {
    if (activeClass === 'all' || !meta) return [];
    const classInfo = meta.classes.find((c) => c.slug === activeClass);
    return classInfo ? meta.categories[String(classInfo.id)] ?? [] : [];
  });
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

  function openDetail(p: CfProject) {
    detail = p;
  }



  onMount(() => {
    // Worlds, metadata, and the first search are independent — fire them all
    // at once instead of serially awaiting the world list first.
    loadMeta();
    load();
  });

  // The world list is instance-scoped (installs land in that instance's
  // worlds); reload it on instance switch. Search results and metadata are
  // CurseForge-global and stay as they are.
  $effect(() => {
    $activeInstance?.id;
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
      <select bind:value={pageSize} onchange={() => { page = 1; load(); }} class="input w-28 text-xs py-2" aria-label="Results per page">
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
                Enter your API key in <a href="/settings#integrations" class="text-bedrock-300 underline">Settings → Integrations</a> — it applies immediately, no restart needed.
              {:else}
                Ask an admin to set the <code class="text-bedrock-300">CURSEFORGE_API_KEY</code> in Settings → Integrations.
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
              <!-- Thumbnails load via the authsrc action: header-authenticated
                   fetch, blob object URL — no token in the image URL. -->
              <img use:authsrc={{ url: thumbUrl(p), onerror: () => brokenThumbs = new Set([...brokenThumbs, p.id]) }}
                   alt="" loading="lazy" decoding="async"
                   class="w-14 h-14 rounded-sm object-cover bg-deep-900 border border-deep-700/50 shrink-0" />
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

{#if detail}
  <ProjectDetailModal project={detail} selWorld={selWorld} {pageSize} {canInstall} versions={meta?.versions ?? []} onclose={() => (detail = null)} />
{/if}
