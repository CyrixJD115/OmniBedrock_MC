/**
 * Presentation helpers for the instances page: memory/uptime formatting,
 * deployment badge styling, limit-enforcement notes, and the memory usage
 * bar percentage.
 */
import type { Instance } from '$types/index';

export function deploymentBadge(d: string): string {
  if (d === 'docker') return 'text-sky-400 border-sky-500/40 bg-sky-500/10';
  if (d === 'systemd') return 'text-violet-400 border-violet-500/40 bg-violet-500/10';
  return 'text-deep-300 border-deep-500/40 bg-deep-600/20';
}

export function enforcementNote(i: Instance): string {
  if (i.enforcement === 'container') return 'Limits enforced by the container runtime (hard cgroup limits; OOM kills the container).';
  if (i.enforcement === 'cgroup') return 'Runs as a systemd user service: MemoryMax/CPUQuota enforced, and the server keeps running through panel restarts.';
  return 'Advisory only: usage is reported, but the OS does not enforce these limits on native processes.';
}

export function memBar(i: Instance): number {
  const limit = i.resources.memory_mb;
  if (!limit || !i.usage) return 0;
  return Math.min(100, (i.usage.memory_mb / limit) * 100);
}

export function fmtMb(n: number): string {
  return n >= 1024 ? `${(n / 1024).toFixed(1)} GB` : `${Math.round(n)} MB`;
}

export function fmtUptime(s: number): string {
  if (!s) return '—';
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}
