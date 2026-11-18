/**
 * Known config.yml shape (MCXboxBroadcast config-version 7) and the YAML ⇄
 * form-model conversion behind the broadcaster page's Visual editor:
 * `visualDefaults` seeds the form model, `normalize` fills it from a parsed
 * YAML document, and `overlayModel` writes edited fields back onto the last
 * parsed document so unknown keys survive a visual edit round-trip.
 */

export interface BcVisualConfig {
  session: {
    'remote-address': string;
    'remote-port': string;
    'update-interval': number;
    'query-server': boolean;
    'web-query-fallback': boolean;
    'config-fallback': boolean;
    'session-info': {
      'host-name': string;
      'world-name': string;
      'game-mode': string;
      players: number;
      'max-players': number;
    };
    'hide-old-sessions': boolean;
    'ice-port-range': { min: number; max: number };
  };
  'friend-sync': {
    'update-interval': number;
    'auto-friend': boolean;
    'initial-invite': boolean;
    expiry: { enabled: boolean; days: number; check: number };
  };
  notifications: {
    enabled: boolean;
    'webhook-url': string;
    'session-expired-message': string;
    'friend-restriction-message': string;
  };
  'debug-mode': boolean;
  'suppress-session-update-message': boolean;
}

export function visualDefaults(): BcVisualConfig {
  return {
    session: {
      // The offer-relay target: loopback, because the panel runs the
      // broadcaster on the same host as the server (config-version 7;
      // pre-v7 session-info.ip/port are migrated into these keys by the jar).
      'remote-address': '127.0.0.1',
      'remote-port': 'auto',
      'update-interval': 30,
      'query-server': true,
      'web-query-fallback': false,
      'config-fallback': false,
      'session-info': {
        'host-name': '',
        'world-name': '',
        'game-mode': 'Survival',
        players: 0,
        'max-players': 20,
      },
      'hide-old-sessions': true,
      'ice-port-range': { min: 0, max: 0 },
    },
    'friend-sync': {
      'update-interval': 60,
      'auto-friend': true,
      'initial-invite': true,
      expiry: { enabled: true, days: 15, check: 1800 },
    },
    notifications: {
      enabled: false,
      'webhook-url': '',
      'session-expired-message': '',
      'friend-restriction-message': '',
    },
    'debug-mode': false,
    'suppress-session-update-message': false,
  };
}

function num(value: unknown, fallback: number): number {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}
function str(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback;
}
/** Text fields that YAML may hand back as numbers (`remote-port: 24474`
 * parses as a number; showing the fallback 'auto' there misreported the
 * file on disk and re-saved it as 'auto'). */
function text(value: unknown, fallback: string): string {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  return fallback;
}
function bool(value: unknown, fallback: boolean): boolean {
  if (typeof value === 'boolean') return value;
  if (value === 'true') return true;
  if (value === 'false') return false;
  return fallback;
}

/** Fill the form model from a parsed YAML document, defaulting unknown fields. */
export function normalize(doc: Record<string, any>): BcVisualConfig {
  const d = visualDefaults();
  const s = doc['session'] ?? {};
  const info = s['session-info'] ?? {};
  const ice = s['ice-port-range'] ?? {};
  const f = doc['friend-sync'] ?? {};
  const expiry = f['expiry'] ?? {};
  const n = doc['notifications'] ?? {};
  d.session['remote-address'] = text(s['remote-address'], '127.0.0.1');
  d.session['remote-port'] = text(s['remote-port'], 'auto');
  d.session['update-interval'] = num(s['update-interval'], 30);
  d.session['query-server'] = bool(s['query-server'], true);
  d.session['web-query-fallback'] = bool(s['web-query-fallback'], false);
  d.session['config-fallback'] = bool(s['config-fallback'], false);
  d.session['hide-old-sessions'] = bool(s['hide-old-sessions'], true);
  d.session['session-info']['host-name'] = text(info['host-name'], '');
  d.session['session-info']['world-name'] = text(info['world-name'], '');
  d.session['session-info']['game-mode'] = str(info['game-mode'], 'Survival');
  d.session['session-info']['players'] = num(info['players'], 0);
  d.session['session-info']['max-players'] = num(info['max-players'], 20);
  d.session['ice-port-range'].min = num(ice['min'], 0);
  d.session['ice-port-range'].max = num(ice['max'], 0);
  d['friend-sync']['update-interval'] = num(f['update-interval'], 60);
  d['friend-sync']['auto-friend'] = bool(f['auto-friend'], true);
  d['friend-sync']['initial-invite'] = bool(f['initial-invite'], true);
  d['friend-sync'].expiry.enabled = bool(expiry['enabled'], true);
  d['friend-sync'].expiry.days = num(expiry['days'], 15);
  d['friend-sync'].expiry.check = num(expiry['check'], 1800);
  d.notifications.enabled = bool(n['enabled'], false);
  d.notifications['webhook-url'] = str(n['webhook-url'], '');
  d.notifications['session-expired-message'] = str(n['session-expired-message'], '');
  d.notifications['friend-restriction-message'] = str(n['friend-restriction-message'], '');
  d['debug-mode'] = bool(doc['debug-mode'], false);
  d['suppress-session-update-message'] = bool(doc['suppress-session-update-message'], false);
  return d;
}

/** Overlay the form model onto the last parsed document so unknown keys survive. */
export function overlayModel(doc: Record<string, any>, model: BcVisualConfig): Record<string, any> {
  doc['session'] = {
    ...(doc['session'] ?? {}),
    'remote-address': model.session['remote-address'],
    'remote-port': model.session['remote-port'],
    'update-interval': model.session['update-interval'],
    'query-server': model.session['query-server'],
    'web-query-fallback': model.session['web-query-fallback'],
    'config-fallback': model.session['config-fallback'],
    'hide-old-sessions': model.session['hide-old-sessions'],
    'session-info': {
      ...(doc['session']?.['session-info'] ?? {}),
      ...model.session['session-info'],
    },
    'ice-port-range': {
      ...(doc['session']?.['ice-port-range'] ?? {}),
      ...model.session['ice-port-range'],
    },
  };
  doc['friend-sync'] = {
    ...(doc['friend-sync'] ?? {}),
    'update-interval': model['friend-sync']['update-interval'],
    'auto-friend': model['friend-sync']['auto-friend'],
    'initial-invite': model['friend-sync']['initial-invite'],
    expiry: { ...(doc['friend-sync']?.['expiry'] ?? {}), ...model['friend-sync'].expiry },
  };
  doc['notifications'] = {
    ...(doc['notifications'] ?? {}),
    ...model.notifications,
  };
  doc['debug-mode'] = model['debug-mode'];
  doc['suppress-session-update-message'] = model['suppress-session-update-message'];
  return doc;
}

/** server.properties facts (see the status endpoint's `server`). */
export interface ServerFactsLike {
  port: number;
  server_name: string | null;
  level_name: string | null;
  max_players: number | null;
  gamemode: string | null;
}

/** One field where the broadcaster config disagrees with server.properties. */
export interface DriftItem {
  key: string;
  label: string;
  /** 'operational' changes what the broadcaster talks to; 'fallback' values
   * only show on the Xbox session when the live query cannot answer. */
  kind: 'operational' | 'fallback';
  config: string;
  server: string;
  /** Write the server.properties value into the form model. */
  apply: (cfg: BcVisualConfig) => void;
}

function gameModeLabel(mode: string | null): string | null {
  if (!mode) return null;
  const m = mode.trim().toLowerCase();
  if (m === '0' || m === 'survival') return 'Survival';
  if (m === '1' || m === 'creative') return 'Creative';
  if (m === '2' || m === 'adventure') return 'Adventure';
  return null;
}

/**
 * Fields where the broadcaster's config.yml disagrees with the instance's
 * server.properties. The relay port matters operationally (a wrong port
 * queries and advertises another server); the session-info values are the
 * fallback the Xbox session shows when the query ping cannot answer.
 */
export function serverDrift(cfg: BcVisualConfig, facts: ServerFactsLike | null | undefined): DriftItem[] {
  if (!facts) return [];
  const out: DriftItem[] = [];
  const port = cfg.session['remote-port'].trim();
  if (port !== 'auto' && port !== String(facts.port)) {
    out.push({
      key: 'remote-port', label: 'Relay port (session.remote-port)', kind: 'operational',
      config: port || '(empty)', server: String(facts.port),
      apply: (c) => { c.session['remote-port'] = String(facts.port); },
    });
  }
  const info = cfg.session['session-info'];
  if (facts.server_name && info['host-name'] !== facts.server_name) {
    out.push({
      key: 'host-name', kind: 'fallback', label: 'Host name ← server-name',
      config: info['host-name'] || '(empty)', server: facts.server_name,
      apply: (c) => { c.session['session-info']['host-name'] = facts.server_name ?? ''; },
    });
  }
  if (facts.level_name && info['world-name'] !== facts.level_name) {
    out.push({
      key: 'world-name', kind: 'fallback', label: 'World name ← level-name',
      config: info['world-name'] || '(empty)', server: facts.level_name,
      apply: (c) => { c.session['session-info']['world-name'] = facts.level_name ?? ''; },
    });
  }
  if (facts.max_players != null && info['max-players'] !== facts.max_players) {
    out.push({
      key: 'max-players', kind: 'fallback', label: 'Max players',
      config: String(info['max-players']), server: String(facts.max_players),
      apply: (c) => { c.session['session-info']['max-players'] = facts.max_players ?? c.session['session-info']['max-players']; },
    });
  }
  const mode = gameModeLabel(facts.gamemode);
  if (mode && info['game-mode'] !== mode) {
    out.push({
      key: 'game-mode', kind: 'fallback', label: 'Game mode',
      config: info['game-mode'], server: mode,
      apply: (c) => { c.session['session-info']['game-mode'] = mode; },
    });
  }
  return out;
}
