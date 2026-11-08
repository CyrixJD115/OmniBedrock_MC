/**
 * Scope reconciliation for the persisted active-instance selection.
 *
 * Kept pure (no svelte stores, no API client) so the boot path and the unit
 * test exercise the exact same rule: a browser that kept pointing at an
 * instance that was renamed or deleted elsewhere must land on a scope that
 * still exists, not have every request 404 under it.
 */

/** The id the client should operate on given the persisted selection and the
 * instances the server actually has: keep the selection when it still exists,
 * otherwise prefer 'default', then the first listed instance, then 'default'
 * when nothing is known at all (empty or unloadable list). */
export function fallbackInstanceId(persisted: string, list: Array<{ id: string }>): string {
  if (list.some((i) => i.id === persisted)) return persisted;
  return list.find((i) => i.id === 'default')?.id ?? list[0]?.id ?? 'default';
}
