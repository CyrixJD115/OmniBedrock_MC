import { writable, derived } from 'svelte/store';
import type { ServerStatus, ConsoleLine, Metrics, ErrorStat } from '$types/index';

export const serverStatus = writable<ServerStatus>({
  status: 'stopped',
  pid: null,
  uptime: 0,
  version: null,
});

export const consoleLines = writable<ConsoleLine[]>([]);
export const metrics = writable<Metrics | null>(null);
export const errorStats = writable<ErrorStat[]>([]);

export const isServerRunning = derived(serverStatus, ($s) => $s.status === 'running');

export const theme = writable<'dark' | 'light'>('dark');
