// Pure display/formatting helpers for the Addon Organizer: metadata accessors,
// comparisons, formatters, and the structured manifest HTML renderer.
import type { Addon } from '$types/index';

export type PackKind = 'bp' | 'rp';

/** Details-tab table sort columns. */
export type DetailsSortKey = 'name' | 'type' | 'world' | 'author' | 'version' | 'uuid' | 'size' | 'modified' | 'status';

/** Available-tab sort columns. */
export type AvailSortKey = 'name' | 'date' | 'author' | 'type' | 'status' | 'version' | 'size';

export function kindOf(p: Addon): PackKind {
  return p.pack_type === 'behavior_packs' ? 'bp' : 'rp';
}

export function packColorClass(packType: 'bp' | 'rp'): string {
  return packType === 'bp' ? 'text-bedrock-400' : 'text-teal-400';
}

export function authorOf(p: Addon): string {
  const authors = (p.manifest as any)?.metadata?.authors;
  if (Array.isArray(authors) && authors.length > 0) return authors.join(', ');
  const st = p.store as any;
  if (Array.isArray(st?.authors) && st.authors.length > 0) return st.authors.join(', ');
  return '';
}

export function dateOf(p: Addon): number {
  const st = p.store as any;
  if (st?.installed_at) {
    const t = Date.parse(String(st.installed_at));
    if (!Number.isNaN(t)) return t / 1000;
  }
  return p.modified ?? 0;
}

export function cmpVersion(a: number[], b: number[]): number {
  for (let i = 0; i < 4; i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x - y;
  }
  return 0;
}

export const statusRank = (p: Addon, order: string[]): number => {
  if (order.includes(p.uuid)) return 0;
  if (!p.valid) return 2;
  return 1;
};

export function esc(s: string): string {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

export function versionStr(v: number[] | undefined): string {
  if (!v || v.length === 0) return '—';
  return v.join('.');
}

export function fmtSize(bytes?: number): string {
  if (!bytes) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1048576).toFixed(1)} MiB`;
}

export function fmtDate(secs?: number): string {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleDateString();
}

/** Structured manifest view rendered into the preview drawer via {@html}. */
export function structuredHtml(a: Addon): string {
  const m = a.manifest as any;
  if (!m || !m.header) return '<p class="text-red-400 text-xs">Invalid or missing manifest</p>';
  const h = m.header;
  const row = (label: string, value: string, valClass = 'text-deep-200') =>
    `<div class="flex justify-between gap-2 py-1"><span class="text-deep-400 shrink-0">${label}</span><span class="${valClass} text-right break-all">${esc(value)}</span></div>`;
  let html = `<div class="space-y-0.5 text-xs">`;
  // The API resolves lang keys like `pack.name` against texts/*.lang; show
  // the resolved text with the raw key in parentheses when they differ.
  const nameKey = typeof h.name === 'string' ? h.name : '';
  const displayName = a.name || nameKey;
  html += row('Name', nameKey && nameKey !== displayName ? `${displayName} (${nameKey})` : displayName, 'text-strong font-medium');
  html += row('UUID', h.uuid ?? '', 'text-green-400 font-mono');
  html += row('Version', (h.version ?? []).join('.'), 'text-blue-400 font-mono');
  html += row('Format', m.format_version ?? '');
  if (h.min_engine_version) html += row('Min Engine', h.min_engine_version.join('.'));
  const descText = a.description ?? (typeof h.description === 'string' ? h.description : '');
  if (descText) {
    html += `<div class="bg-deep-800/60 rounded p-2 mt-2 text-deep-300 italic text-[11px]">${esc(descText)}</div>`;
  }
  if (m.modules?.length) {
    html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Modules (${m.modules.length})</span>`;
    html += m.modules.map((mod: any) =>
      `<div class="flex justify-between text-deep-300 py-0.5 gap-2"><span class="min-w-0 truncate">${esc(mod.type ?? '')}${mod.description ? ' — <span class="text-deep-400">' + esc(mod.description) + '</span>' : ''}</span><span class="font-mono text-[10px] text-deep-400 shrink-0">${esc((mod.version ?? []).join('.'))}</span></div>`
    ).join('') + `</div>`;
  }
  if (m.subpacks?.length) {
    html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Subpacks (${m.subpacks.length})</span>`;
    html += m.subpacks.map((sub: any) => {
      const tier = Number.isFinite(sub.memory_tier) ? ` · tier ${sub.memory_tier}` : '';
      const folder = sub.folder_name ? `<span class="font-mono text-[10px] text-deep-400 shrink-0">${esc(sub.folder_name)}/</span>` : '';
      return `<div class="flex justify-between text-deep-300 py-0.5 gap-2"><span class="min-w-0 truncate">${esc(sub.name ?? '')}${tier ? `<span class="text-deep-400">${esc(tier)}</span>` : ''}</span>${folder}</div>`;
    }).join('') + `</div>`;
  }
  if (m.dependencies?.length) {
    html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Dependencies (${m.dependencies.length})</span>`;
    html += m.dependencies.map((dep: any) =>
      `<div class="text-deep-300 py-0.5 font-mono text-[10px]">${esc(dep.uuid ?? dep.module_name ?? '')} ${dep.version ? '<span class="text-deep-400">v' + esc(dep.version.join('.')) + '</span>' : ''}</div>`
    ).join('') + `</div>`;
  }
  if (m.metadata) {
    html += `<div class="mt-3"><span class="text-deep-400 block mb-1 uppercase tracking-wider text-[10px]">Metadata</span>`;
    if (m.metadata.authors) html += `<div class="text-deep-300 text-[11px]">Authors: ${esc(m.metadata.authors.join(', '))}</div>`;
    if (m.metadata.product_type) html += `<div class="text-deep-300 text-[11px]">Product: ${esc(m.metadata.product_type)}</div>`;
    html += `</div>`;
  }
  if (m.capabilities?.length) {
    html += `<div class="mt-2"><span class="text-deep-400 text-[10px] uppercase tracking-wider">Capabilities:</span> <span class="text-deep-300">${m.capabilities.map((c: string) => esc(c)).join(', ')}</span></div>`;
  }
  html += `</div>`;
  return html;
}
