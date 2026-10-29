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
function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
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
  d.session['remote-address'] = str(s['remote-address'], '127.0.0.1');
  d.session['remote-port'] = str(s['remote-port'], 'auto');
  d.session['update-interval'] = num(s['update-interval'], 30);
  d.session['query-server'] = bool(s['query-server'], true);
  d.session['web-query-fallback'] = bool(s['web-query-fallback'], false);
  d.session['config-fallback'] = bool(s['config-fallback'], false);
  d.session['hide-old-sessions'] = bool(s['hide-old-sessions'], true);
  d.session['session-info']['host-name'] = str(info['host-name'], '');
  d.session['session-info']['world-name'] = str(info['world-name'], '');
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
