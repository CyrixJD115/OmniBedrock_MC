import type { StoreMeta, StoreSearchResponse } from '$types/index';

/**
 * Client-side caches for the CurseForge Store page.
 *
 * The CurseForge API is slow (seconds per call), so the Store keeps what it
 * already fetched: filter metadata in sessionStorage (survives reloads) and
 * search results in memory (survives tab navigation). Cached data is served
 * instantly and refreshed in the background — stale-while-revalidate.
 */

const META_KEY = 'omnibedrock.store.meta.v1';
/** Metadata barely changes; treat cached copies fresh for 6 hours. */
const META_FRESH_MS = 6 * 60 * 60 * 1000;
/** Reuse a cached search result without a background refresh for 60s. */
const SEARCH_FRESH_MS = 60 * 1000;
const SEARCH_CACHE_CAP = 24;

interface CachedSearch {
  response: StoreSearchResponse;
  cachedAt: number;
}

const searchCache = new Map<string, CachedSearch>();

export function loadCachedMeta(): StoreMeta | null {
  try {
    const raw = sessionStorage.getItem(META_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as StoreMeta;
    if (!parsed?.classes?.length || !parsed?.versions) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function isMetaFresh(): boolean {
  try {
    const raw = sessionStorage.getItem(META_KEY);
    if (!raw) return false;
    const { cachedAt } = JSON.parse(raw) as { cachedAt: number };
    return Date.now() - cachedAt < META_FRESH_MS;
  } catch {
    return false;
  }
}

export function saveMeta(meta: StoreMeta): void {
  try {
    sessionStorage.setItem(META_KEY, JSON.stringify({ ...meta, cachedAt: Date.now() }));
  } catch {
    // Storage full or unavailable — caching is best-effort.
  }
}

export function searchCacheKey(params: {
  class: string; q: string; page: number; pageSize: number;
  sort: string; version: string; category: string;
}): string {
  return JSON.stringify([params.class, params.q.trim().toLowerCase(), params.page, params.pageSize, params.sort, params.version, params.category]);
}

export function getCachedSearch(key: string): { response: StoreSearchResponse; fresh: boolean } | null {
  const entry = searchCache.get(key);
  if (!entry) return null;
  return { response: entry.response, fresh: Date.now() - entry.cachedAt < SEARCH_FRESH_MS };
}

export function saveSearch(key: string, response: StoreSearchResponse): void {
  // Insert-then-evict oldest insertion order keeps paging around snappy
  // without letting the cache grow unbounded.
  searchCache.delete(key);
  searchCache.set(key, { response, cachedAt: Date.now() });
  while (searchCache.size > SEARCH_CACHE_CAP) {
    const oldest = searchCache.keys().next().value;
    if (oldest === undefined) break;
    searchCache.delete(oldest);
  }
}
