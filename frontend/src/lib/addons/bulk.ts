// Bulk addon operations shared by the Addon Organizer: manifest version bumps
// across many packs and hidden-UUID set toggling.
import { api } from '$lib/api/client';
import type { Addon } from '$types/index';

/**
 * Bump one version component on every given pack's manifest via the API.
 * Packs without a manifest are skipped; failed updates are skipped too.
 * Returns the number of packs actually updated.
 */
export async function bulkBumpPacks(packs: Addon[], part: 'major' | 'minor' | 'patch'): Promise<number> {
  let count = 0;
  for (const a of packs) {
    if (!a.manifest) continue;
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
  return count;
}

/** Toggle every given UUID in the hidden set; returns the next set plus counts. */
export function toggleHiddenBulk(uuids: string[], hidden: Set<string>): {
  next: Set<string>;
  hiddenCount: number;
  unhiddenCount: number;
} {
  const next = new Set(hidden);
  let hiddenCount = 0;
  let unhiddenCount = 0;
  for (const uuid of uuids) {
    if (next.has(uuid)) { next.delete(uuid); unhiddenCount++; }
    else { next.add(uuid); hiddenCount++; }
  }
  return { next, hiddenCount, unhiddenCount };
}
