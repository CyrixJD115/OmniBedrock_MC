import { writable, derived, get } from 'svelte/store';
import { api, setInstance } from '$lib/api/client';
import { persisted } from '$lib/persisted';
import { wsManager } from '$lib/websocket';
import type { Instance, HostInfo } from '$types/index';

export const instances = writable<Instance[]>([]);
export const hostInfo = writable<HostInfo | null>(null);
export const activeInstanceId = persisted<string>('omnibedrock.active-instance', 'default');

/** The instance every page currently operates on. */
export const activeInstance = derived(
  [instances, activeInstanceId],
  ([list, id]) => list.find((i) => i.id === id) ?? list.find((i) => i.id === 'default') ?? list[0] ?? null,
);

export async function loadInstances(): Promise<Instance[]> {
  const list = await api.listInstances();
  instances.set(list);
  const current = get(activeInstanceId);
  if (!list.some((i) => i.id === current)) {
    const fallback = list.find((i) => i.id === 'default')?.id ?? list[0]?.id ?? 'default';
    activeInstanceId.set(fallback);
    setInstance(fallback);
  } else {
    setInstance(current);
  }
  return list;
}

export async function loadHost(): Promise<void> {
  try {
    hostInfo.set(await api.getHost());
  } catch {
    hostInfo.set(null);
  }
}

/**
 * Switch the panel-wide active instance. WebSockets are torn down so pages
 * reconnect against the new instance as they resubscribe.
 */
export function switchInstance(id: string) {
  if (get(activeInstanceId) === id) return;
  activeInstanceId.set(id);
  setInstance(id);
  wsManager.disconnectAll();
}

/** Refresh statuses in the list without disturbing the active selection. */
export async function refreshInstances(): Promise<void> {
  try {
    instances.set(await api.listInstances());
  } catch {
    /* keep the last known list */
  }
}
