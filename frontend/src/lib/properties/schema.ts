// Known server.properties keys: category, input type and a short summary.
// Anything not listed stays editable as free text under "Other"; the file's
// own comment above each key is shown as the long description.

export type Category = 'general' | 'gameplay' | 'world' | 'players' | 'network' | 'performance' | 'content' | 'scripting' | 'other';

export interface PropSpec {
  category: Category;
  kind: 'bool' | 'enum' | 'number' | 'text';
  options?: string[];
  min?: number;
  max?: number | null;
  summary: string;
  /** Read only at server start (true for nearly everything in BDS). */
  restart?: boolean;
}

export const CATEGORIES: { id: Category; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'gameplay', label: 'Gameplay' },
  { id: 'world', label: 'World' },
  { id: 'players', label: 'Players & permissions' },
  { id: 'network', label: 'Network' },
  { id: 'performance', label: 'Performance' },
  { id: 'content', label: 'Content & logging' },
  { id: 'scripting', label: 'Scripting & debugging' },
  { id: 'other', label: 'Other' },
];

const b = (category: Category, summary: string): PropSpec => ({ category, kind: 'bool', summary, restart: true });
const n = (category: Category, summary: string, min: number, max: number | null): PropSpec => ({ category, kind: 'number', min, max, summary, restart: true });
const e = (category: Category, summary: string, options: string[]): PropSpec => ({ category, kind: 'enum', options, summary, restart: true });
const t = (category: Category, summary: string): PropSpec => ({ category, kind: 'text', summary, restart: true });

export const SCHEMA: Record<string, PropSpec> = {
  'server-name': t('general', 'Name shown in the server list'),
  motd: t('general', 'Message of the day'),
  'max-players': n('general', 'Player slots', 1, 1000),
  gamemode: e('gameplay', 'Default game mode for new players', ['survival', 'creative', 'adventure']),
  'force-gamemode': b('gameplay', 'Apply the default game mode to returning players too'),
  difficulty: e('gameplay', 'World difficulty', ['peaceful', 'easy', 'normal', 'hard']),
  'allow-cheats': b('gameplay', 'Allow commands that count as cheats'),
  'player-idle-timeout': n('gameplay', 'Kick idle players after this many minutes (0 = never)', 0, null),
  'disable-player-interaction': b('gameplay', 'Players cannot interact with each other'),
  'chat-restriction': e('gameplay', 'Limit chat messages', ['None', 'Dropped', 'Disabled']),
  'level-name': t('world', 'World folder to load'),
  'level-seed': t('world', 'Seed used when the world is created'),
  'level-type': e('world', 'Generator for new worlds', ['DEFAULT', 'FLAT', 'LEGACY']),
  'view-distance': n('world', 'Maximum view distance in chunks', 5, 96),
  'tick-distance': n('world', 'Simulation distance in chunks', 4, 12),
  'default-player-permission-level': e('players', 'Permission level for new players', ['visitor', 'member', 'operator']),
  'allow-list': b('players', 'Only players in the allowlist can join'),
  'white-list': b('players', 'Legacy name for allow-list'),
  'online-mode': b('players', 'Require Xbox Live authentication'),
  'disable-persona': b('players', 'Hide persona skins'),
  'disable-custom-skins': b('players', 'Block custom skins'),
  'server-port': n('network', 'IPv4 port (NetherNet signaling / RakNet)', 1, 65535),
  'server-portv6': n('network', 'IPv6 port', 1, 65535),
  transport: e('network', 'Network transport', ['raknet', 'nethernet']),
  'server-udp-ports': t('network', 'UDP ports for NetherNet game traffic'),
  'enable-lan-visibility': b('network', 'Announce the server on the LAN'),
  'compression-threshold': n('network', 'Smallest packet that gets compressed (bytes)', 0, 65535),
  'compression-algorithm': e('network', 'Packet compression', ['zlib', 'snappy']),
  'server-authoritative-movement-strict': b('performance', 'Strict server-side movement checks'),
  'server-authoritative-dismount-strict': b('performance', 'Strict server-side dismount checks'),
  'server-authoritative-entity-interactions-strict': b('performance', 'Strict server-side entity interaction checks'),
  'player-position-acceptance-threshold': n('performance', 'Position difference accepted before correction', 0, null),
  'player-movement-action-direction-threshold': n('performance', 'Allowed look/attack direction mismatch', -1, 1),
  'server-authoritative-block-breaking-pick-range-scalar': n('performance', 'Block-breaking reach tolerance multiplier', 0, null),
  'server-authoritative-entity-attack-request-validation': b('performance', 'Validate entity attack requests'),
  'server-authoritative-entity-attack-range-validation': b('performance', 'Validate entity attack range'),
  'server-authoritative-entity-attack-range-tolerance': n('performance', 'Extra attack range tolerated (blocks)', 0, null),
  'persistent-ticking-area-max-count': n('world', 'Maximum persistent ticking areas', 0, null),
  'persistent-ticking-area-max-chunks': n('world', 'Chunks per persistent ticking area', 0, null),
  'transient-ticking-area-max-chunks': n('world', 'Chunks per transient ticking area', 0, null),
  'max-threads': n('performance', 'Worker threads (0 = as many as possible)', 0, null),
  'client-side-chunk-generation-enabled': b('performance', 'Let clients generate visual chunks'),
  'block-network-ids-are-hashes': b('performance', 'Hashed block IDs (keep on with many custom blocks)'),
  'server-build-radius-ratio': t('performance', 'Share of view distance the server builds (Disabled or 0.0–1.0)'),
  'texturepack-required': b('content', 'Players must accept the server resource packs'),
  'content-log-file-enabled': b('content', 'Write ContentLog files to disk (they can grow to hundreds of MB)'),
  'content-log-console-output-enabled': b('content', 'Print content errors to the console'),
  'content-log-level': e('content', 'Lowest content-log level reported', ['verbose', 'info', 'warning', 'error']),
  'emit-server-telemetry': b('content', 'Send server telemetry'),
  'allow-outbound-script-debugging': b('scripting', 'Allow /script debugger connect'),
  'allow-inbound-script-debugging': b('scripting', 'Allow debuggers to attach to the server'),
  'force-inbound-debug-port': n('scripting', 'Fixed port for inbound script debugging', 1, 65535),
  'script-debugger-auto-attach': e('scripting', 'Attach a debugger at startup', ['disabled', 'connect', 'listen']),
  'script-watchdog-enable': b('scripting', 'Watchdog for long-running scripts'),
  'script-watchdog-enable-exception-handling': b('scripting', 'Watchdog handles script exceptions'),
  'script-watchdog-hang-threshold': n('scripting', 'Script hang threshold (ms)', 0, null),
  'script-watchdog-spike-threshold': n('scripting', 'Script spike warning threshold (ms)', 0, null),
  'script-watchdog-slow-threshold': n('scripting', 'Slow script warning threshold (ms)', 0, null),
  'script-watchdog-memory-warning': n('scripting', 'Script memory warning (MB)', 0, null),
  'script-watchdog-memory-limit': n('scripting', 'Script memory limit (MB)', 0, null),
};

/**
 * Widget hints parsed from BDS's own "Allowed values: …" comment, so keys
 * added by newer server builds get the right input without a panel update.
 */
export function fromComment(description: string): Partial<PropSpec> {
  const m = /Allowed values:\s*(.+?)(?:$|\.\s)/i.exec(description);
  if (!m) return {};
  const allowed = m[1];
  const quoted = [...allowed.matchAll(/"([^"]+)"/g)].map((q) => q[1]);
  if (quoted.length === 2 && quoted.includes('true') && quoted.includes('false')) return { kind: 'bool' };
  if (quoted.length >= 2) return { kind: 'enum', options: quoted };
  const range = /(-?\d+(?:\.\d+)?)\s*-\s*(-?\d+(?:\.\d+)?)/.exec(allowed);
  if (range) return { kind: 'number', min: Number(range[1]), max: Number(range[2]) };
  if (/positive integer/i.test(allowed)) return { kind: 'number', min: 0, max: null };
  return {};
}

/** Spec for a key: the schema first, then the file's comment, then the value. */
export function specFor(key: string, value: string, description = ''): PropSpec {
  const known = SCHEMA[key];
  if (known) return known;
  const hinted = fromComment(description);
  if (hinted.kind) return { category: 'other', summary: '', restart: true, ...hinted } as PropSpec;
  if (value === 'true' || value === 'false') return { category: 'other', kind: 'bool', summary: '', restart: true };
  return { category: 'other', kind: 'text', summary: '', restart: true };
}

/**
 * Description per key from the comment lines that follow it (BDS writes the
 * explanation under each key) or, for hand-written files, precede it.
 */
export function descriptions(entries: { key: string; comment: string }[]): Map<string, string> {
  const out = new Map<string, string>();
  for (let i = 0; i < entries.length; i++) {
    const key = entries[i].key;
    if (!key) continue;
    const after: string[] = [];
    for (let j = i + 1; j < entries.length && !entries[j].key && entries[j].comment.startsWith('#'); j++) after.push(entries[j].comment);
    let text = describe(after.join('\n'));
    if (!text) {
      const before: string[] = [];
      for (let j = i - 1; j >= 0 && !entries[j].key && entries[j].comment.startsWith('#'); j--) before.unshift(entries[j].comment);
      text = describe(before.join('\n'));
    }
    if (text) out.set(key, text);
  }
  return out;
}

/** Turn BDS's "# Allowed values: ..." comment block into readable text. */
export function describe(comment: string): string {
  return comment
    .split('\n')
    .map((l) => l.replace(/^\s*#\s?/, '').trim())
    .filter(Boolean)
    .join(' ');
}
