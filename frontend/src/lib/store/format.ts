// Formatting helpers shared by the Store grid and project detail modal.
import { api } from '$lib/api/client';
import type { CfProject } from '$types/index';

export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}
export function fmtDate(iso?: string): string {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString();
}
export function releaseClass(type: number): string {
  if (type === 1) return 'text-green-400 border-green-500/30 bg-green-500/10';
  if (type === 2) return 'text-yellow-400 border-yellow-500/30 bg-yellow-500/10';
  return 'text-red-400 border-red-500/30 bg-red-500/10';
}
export function releaseLabel(type: number): string {
  return type === 1 ? 'Release' : type === 2 ? 'Beta' : type === 3 ? 'Alpha' : 'Unknown';
}
export function thumbUrl(p: CfProject): string | null {
  const url = p.logo?.thumbnailUrl || p.logo?.url;
  return url ? api.storeImageUrl(url) : null;
}
