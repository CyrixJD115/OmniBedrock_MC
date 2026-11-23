// Time and label helpers for the Players page.

/** 3725 → "1h 2m"; 42 → "42s". */
export function duration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Unix seconds → "5m ago" / "3d ago". */
export function ago(unix: number, now = Date.now() / 1000): string {
  const d = now - unix;
  if (d < 45) return 'just now';
  return `${duration(d)} ago`;
}

export function dateTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

export const PERMISSION_CLASS: Record<string, string> = {
  operator: 'badge-warn',
  member: 'badge-info',
  visitor: 'badge-stopped',
};
