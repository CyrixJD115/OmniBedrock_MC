import { writable, derived } from 'svelte/store';
import type { BackupEvent, BackupSettings } from '$types/index';
import { getToken } from '$lib/api/client';
import { wsManager } from '$lib/websocket';

export const backupEvents = writable<BackupEvent[]>([]);
export const backupSettings = writable<BackupSettings | null>(null);
export const backupRunning = writable<boolean>(false);
export const latestBackupEvent = writable<BackupEvent | null>(null);

function getBackupWsUrl(): string {
  const token = getToken();
  return `/api/v1/backups/ws${token ? `?token=${token}` : ''}`;
}

let disconnectWs: (() => void) | null = null;

export function connectBackupWs() {
  disconnectWs?.();
  const url = getBackupWsUrl();
  disconnectWs = wsManager.connect(url, (data: Record<string, unknown>) => {
    const event = data as unknown as BackupEvent;
    latestBackupEvent.set(event);
    backupEvents.update((events) => {
      const next = [...events, event];
      if (next.length > 500) next.splice(0, next.length - 500);
      return next;
    });
    if (event.type === 'status' || event.type === 'progress') {
      backupRunning.set(true);
    } else if (event.type === 'done' || event.type === 'error') {
      backupRunning.set(false);
    }
  });
}

export function disconnectBackupWs() {
  disconnectWs?.();
  disconnectWs = null;
}
