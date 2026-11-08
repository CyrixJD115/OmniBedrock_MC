/**
 * Pure path/breadcrumb helpers for the instance file explorer. '/' is the
 * separator the backend reports (its confinement rejects backslash and '..'
 * forms), so everything here is string shaping only — no svelte stores, no
 * API client — which keeps the unit tests bootstrap-free:
 *
 *   npm test
 */

/**
 * Normalize a user-entered path into the canonical explorer form: no
 * leading/trailing/duplicated slashes, no `.` segments, backslashes folded
 * to `/`. `..` segments make the whole path invalid (the backend refuses
 * them outright, so the UI never offers them) — returns null for those.
 * An empty/blank input is the valid root and normalizes to ''.
 */
export function normalizePath(input: string): string | null {
  const parts = input
    .replace(/\\/g, '/')
    .split('/')
    .filter((s) => s !== '' && s !== '.');
  if (parts.some((p) => p === '..')) return null;
  return parts.join('/');
}

/** Clickable breadcrumb segments, root first; each carries its full path. */
export function breadcrumbs(path: string): { name: string; path: string }[] {
  const crumbs = [{ name: 'root', path: '' }];
  let acc = '';
  const norm = normalizePath(path) ?? '';
  if (norm !== '') {
    for (const part of norm.split('/')) {
      acc = acc ? `${acc}/${part}` : part;
      crumbs.push({ name: part, path: acc });
    }
  }
  return crumbs;
}

/** Join a directory path with a listing entry name ('' tolerates the root).
 * Names come from the backend's listing (always single components). */
export function joinPath(dir: string, name: string): string {
  const d = normalizePath(dir) ?? '';
  return d ? `${d}/${name}` : name;
}

/** Parent path ('' for the root or an invalid path). */
export function parentPath(path: string): string {
  const norm = normalizePath(path);
  if (!norm) return '';
  const idx = norm.lastIndexOf('/');
  return idx === -1 ? '' : norm.slice(0, idx);
}

/** Basename of a path ('' for the root or an invalid path). */
export function baseName(path: string): string {
  const norm = normalizePath(path);
  if (!norm) return '';
  return norm.slice(norm.lastIndexOf('/') + 1);
}

/** Extensions the inline editor may open; everything else is download-only
 * (the backend also guards against NUL-bearing content at read time). */
const TEXT_EXTENSIONS = [
  'txt', 'ini', 'yaml', 'yml', 'json', 'properties', 'cfg', 'conf', 'md',
  'py', 'js', 'ts', 'html', 'css', 'xml', 'csv', 'log', 'sh', 'toml', 'env',
];

export function isTextFile(name: string): boolean {
  const lower = name.toLowerCase();
  const dot = lower.lastIndexOf('.');
  if (dot <= 0) return false;
  return TEXT_EXTENSIONS.includes(lower.slice(dot + 1));
}
