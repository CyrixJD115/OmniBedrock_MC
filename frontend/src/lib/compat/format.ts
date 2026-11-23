// Display helpers for the compatibility checker.
import type { CompatIssue, CompatSeverity } from '$types/compat';

export const SEVERITIES: CompatSeverity[] = ['critical', 'error', 'warning', 'info'];

export const SEVERITY_LABEL: Record<CompatSeverity, string> = {
  critical: 'Critical',
  error: 'Error',
  warning: 'Warning',
  info: 'Info',
};

/** Badge class per severity (badge-* from app.css). */
export function severityBadge(s: CompatSeverity): string {
  return s === 'critical' || s === 'error' ? 'badge-stopped' : s === 'warning' ? 'badge-warn' : 'badge-info';
}

/** Left accent border per severity. */
export function severityBorder(s: CompatSeverity): string {
  return s === 'critical'
    ? 'border-l-red-400'
    : s === 'error'
      ? 'border-l-red-500/60'
      : s === 'warning'
        ? 'border-l-yellow-400/70'
        : 'border-l-bedrock-400/50';
}

export const SOURCE_LABEL: Record<CompatIssue['source'], string> = {
  log: 'Server log',
  manifest: 'Manifest',
  'script-scan': 'Script scan',
  disk: 'Disk',
};

/** 1234567 → "1.23M". */
export function compact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 1 : 2)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

export function bytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(2)} GB`;
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

/** Plain-text summary of one issue, for the clipboard. */
export function issueText(i: CompatIssue): string {
  const lines = [`[${SEVERITY_LABEL[i.severity]}] ${i.title}`];
  if (i.pack) lines.push(`Pack: ${i.pack.name} (${i.pack.folder})`);
  else if (i.hint) lines.push(`Attributed to: ${i.hint}`);
  if (i.source === 'log') lines.push(`Occurrences: ${i.count}${i.first_seen ? ` · first ${i.first_seen}` : ''}`);
  if (i.location) lines.push(`Location: ${i.location.function ?? '<anonymous>'} ${i.location.file ?? ''}${i.location.line ? `:${i.location.line}` : ''}`);
  lines.push('', i.detail, '', i.explanation, '', 'How to fix:', ...i.fixes.map((f) => `- ${f}`));
  if (i.docs.length) lines.push('', ...i.docs.map((d) => `${d.title}: ${d.url}`));
  return lines.join('\n');
}
