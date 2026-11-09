import { AnsiUp } from 'ansi_up';
import { writable } from 'svelte/store';
import type { ConsoleLine } from '$types/index';

const ansi = new AnsiUp();

ansi.use_classes = false;

const LEVEL_RE = /\[(ERROR|WARN(?:ING)?|INFO|DEBUG|LOCAL)\]/i;
const ERROR_KEYWORDS = /(?:error|exception|traceback|fatal)/i;
const TAG_RE = /(<[^>]*>)|([^<]+)/g;
const ANCHOR_RE = /<a href="([^"]*)"[^>]*>([\s\S]*?)<\/a>/g;

export function parseAnsi(text: string): string {
  return ansi.ansi_to_html(text);
}

/**
 * ansi_up converts ANSI OSC-8 hyperlink sequences into live anchors. Console
 * text is attacker-influenced (player chat, addon install output), so only
 * http(s) targets survive; anything else is unwrapped to inert text.
 */
export function sanitizeConsoleHtml(html: string): string {
  return html.replace(ANCHOR_RE, (whole, href: string, text: string) => {
    const target = href.trim().toLowerCase();
    if (target.startsWith('http://') || target.startsWith('https://')) return whole;
    return `<span>${text}</span>`;
  });
}

export function detectLevel(text: string): 'error' | 'warn' | 'info' | 'debug' | 'local' {
  const m = LEVEL_RE.exec(text);
  if (m) {
    const token = m[1].toUpperCase();
    if (token === 'ERROR') return 'error';
    if (token === 'WARN' || token === 'WARNING') return 'warn';
    if (token === 'DEBUG') return 'debug';
    if (token === 'LOCAL') return 'local';
    return 'info';
  }
  if (ERROR_KEYWORDS.test(text)) return 'error';
  return 'info';
}

export function matchesQuery(text: string, query: string, useRegex: boolean): boolean {
  if (!query) return true;
  try {
    if (useRegex) {
      return new RegExp(query, 'i').test(text);
    }
    return text.toLowerCase().includes(query.toLowerCase());
  } catch {
    return text.toLowerCase().includes(query.toLowerCase());
  }
}

export function highlightMatches(html: string, query: string, useRegex: boolean): string {
  if (!query) return html;
  let re: RegExp;
  try {
    re = useRegex ? new RegExp(query, 'gi') : new RegExp(escapeRegex(query), 'gi');
  } catch {
    return html;
  }

  return html.replace(TAG_RE, (_: string, tag: string, text: string) => {
    if (tag) return tag;
    return text.replace(re, m => `<mark class="console-match">${m}</mark>`);
  });
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// ─── Console feeds ──────────────────────────────────────────────────────────

/** A console line with a client-assigned id that never changes. */
export type FeedLine = ConsoleLine & { id: number };

/** A run of consecutive identical lines (the terminal collapse view). */
export interface CollapsedLine extends FeedLine {
  /** How many consecutive raw lines this run covers (1 for a single line). */
  count: number;
}

/**
 * The collapse identity of a log line: the line text with a leading
 * timestamp+level stamp stripped. Broadcaster and BDS stamp every line
 * (`[16:36:57 INFO] …`, `[2026-11-09 13:24:42:608 INFO] …`), so raw
 * byte-equality would never merge the periodic "Updated session!" spam the
 * collapser exists for — the message body is what makes two lines "the
 * same". Only a genuine timestamp-stamp prefix is stripped; bracketed
 * message text is left alone.
 */
const TIMESTAMP_STAMP_PREFIX =
  /^\[(?:\d{4}-\d{2}-\d{2}[ ])?\d{1,2}:\d{2}:\d{2}(?::\d{1,3})?\s+(?:INFO|WARN(?:ING)?|ERROR|ERR|DEBUG|TRACE|FATAL|LOCAL)\]\s?/;

export function collapseKey(text: string): string {
  return text.replace(TIMESTAMP_STAMP_PREFIX, '');
}

/**
 * Collapse consecutive identical lines into runs, exactly the terminal
 * compaction users expect: repeated output ([Primary Session] Updated
 * session! every 30s, heartbeat spam) renders once with a ×N badge, and any
 * different line starts a new run — so the same text can reappear later as
 * its own collapsed run. Identity ignores the leading timestamp stamp, so
 * per-line clock stamps do not defeat the merge. Operates on whatever list
 * it is given (live, frozen, filtered), so runs are always recomputed from
 * the actually rendered slice and never split across freeze boundaries.
 */
export function collapseRuns(lines: FeedLine[]): CollapsedLine[] {
  const out: CollapsedLine[] = [];
  for (const line of lines) {
    const last = out[out.length - 1];
    if (
      last &&
      last.level === line.level &&
      collapseKey(last.text) === collapseKey(line.text)
    ) {
      last.count++;
      // The run carries the most recent occurrence: timestamps-on shows the
      // latest instant, and the sequence tracks the newest line for cursors.
      if (line.timestamp) last.timestamp = line.timestamp;
      if (line.sequence !== undefined) last.sequence = line.sequence;
    } else {
      out.push({ ...line, count: 1 });
    }
  }
  return out;
}

/**
 * A capped console line store with stable per-line keys.
 *
 * Keys matter: trimming an index-keyed array re-keys every surviving line, so
 * a capped console rebuilt its whole DOM on every line once it hit the cap —
 * which is exactly when it is busiest. Stable ids keep `{#each}` updates to
 * one append (plus one drop at the cap) regardless of volume. Lines are also
 * batched per animation frame, so a burst of output costs one store update,
 * not one per line.
 */
export interface ConsoleFeed {
  subscribe: (run: (lines: FeedLine[]) => void) => () => void;
  push: (text: string, level?: ConsoleLine['level'], extra?: Partial<ConsoleLine>) => void;
  clear: () => void;
}

export function createConsoleFeed(max: number): ConsoleFeed {
  let uid = 0;
  const pending: FeedLine[] = [];
  let scheduled = false;
  const store = writable<FeedLine[]>([]);

  function flush() {
    scheduled = false;
    if (pending.length === 0) return;
    const batch = pending.splice(0);
    store.update(lines => {
      lines.push(...batch);
      if (lines.length > max) lines.splice(0, lines.length - max);
      return lines;
    });
  }

  function push(text: string, level?: ConsoleLine['level'], extra?: Partial<ConsoleLine>) {
    pending.push({
      id: ++uid,
      text,
      level: level ?? detectLevel(text),
      timestamp: Date.now() / 1000,
      ...extra,
    });
    if (!scheduled) {
      scheduled = true;
      if (typeof requestAnimationFrame === 'function') requestAnimationFrame(flush);
      else queueMicrotask(flush);
    }
  }

  function clear() {
    pending.length = 0;
    store.set([]);
  }

  return { subscribe: store.subscribe, push, clear };
}

// ─── Terminal scrolling ─────────────────────────────────────────────────────

/** True when the element's viewport is within `threshold` px of its bottom. */
export function nearBottom(el: HTMLElement, threshold = 48): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
}

/** Snap the element to its last line. Call after the DOM has updated. */
export function stickToBottom(el: HTMLElement): void {
  el.scrollTop = el.scrollHeight;
}
