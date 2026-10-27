import { writable, derived } from 'svelte/store';
import type { ServerStatus, Metrics, ErrorStat } from '$types/index';
import { createConsoleFeed } from '$lib/console';

export const serverStatus = writable<ServerStatus>({
  status: 'stopped',
  pid: null,
  uptime: 0,
  version: null,
});

// Capped at 2000 lines with stable per-line keys and rAF batching — see
// createConsoleFeed for why the cap no longer stalls the page.
export const consoleLines = createConsoleFeed(2000);
export const metrics = writable<Metrics | null>(null);
export const errorStats = writable<ErrorStat[]>([]);

export const isServerRunning = derived(serverStatus, ($s) => $s.status === 'running');

export const theme = writable<'dark' | 'light'>('dark');
