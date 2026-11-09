/**
 * Broadcaster console state that OUTLIVES the page: a module-level feed
 * singleton plus per-instance replay bookkeeping.
 *
 * Why this exists: the broadcaster page used to create its feed per mount
 * and reconnect on every visit, so the backend replayed the whole console
 * journal — a "cleared" view resurrected itself the moment the user went to
 * another tab and back. Now the feed survives route changes, and two layers
 * keep a cleared view clear:
 *
 * 1. Resume cursor — reconnects carry `stream_id` + `after=<lastSeq>`, so
 *    the backend itself sends nothing before the last line the client saw
 *    (`hello.resumed === true`).
 * 2. `clearedThrough` — on Clear, the highest sequence seen is recorded and
 *    every replayed line at or below it is dropped client-side. This also
 *    covers the full-replay path (backend restart or expired cursor), and a
 *    `stream_id` change (new backend session) resets the view cleanly.
 *
 * Explicit relative imports with extensions keep this module runnable under
 * `node --test` (see src/lib/theme.test.ts note).
 */
import { createConsoleFeed, detectLevel, type FeedLine } from '../console.ts';

export const broadcasterFeed = createConsoleFeed(1000);

interface BcConsoleState {
  /** Highest sequence observed for this instance. */
  lastSeq: number | null;
  /** Journal stream id from `hello`; a change means a fresh backend session. */
  streamId: string | null;
  /** Cleared view marker: replayed lines at/below this sequence are dropped. */
  clearedThrough: number | null;
}

const states = new Map<string, BcConsoleState>();
/** The instance the page last loaded; same-id mounts are reconnects. */
let loadedInstanceId: string | null = null;

function stateFor(instanceId: string): BcConsoleState {
  let st = states.get(instanceId);
  if (!st) {
    st = { lastSeq: null, streamId: null, clearedThrough: null };
    states.set(instanceId, st);
  }
  return st;
}

/** True when this mount returns to the instance already being viewed. */
export function isReconnect(instanceId: string): boolean {
  return loadedInstanceId === instanceId;
}

export function markLoaded(instanceId: string): void {
  loadedInstanceId = instanceId;
}

/** Instance switch: drop bookkeeping so the next load replays fresh. */
export function resetFor(instanceId: string): void {
  states.delete(instanceId);
}

/**
 * Clear the visible console for one instance. Lines the backend replays at
 * or below the highest sequence seen so far stay suppressed afterwards.
 */
export function clearView(instanceId: string): void {
  const st = stateFor(instanceId);
  broadcasterFeed.clear();
  st.clearedThrough = st.lastSeq;
}

/**
 * Query suffix that turns a reconnect into a resume: the backend replays
 * nothing before the last sequence this client already holds.
 */
export function resumeSuffix(instanceId: string): string {
  const st = states.get(instanceId);
  if (!st || st.lastSeq === null || !st.streamId) return '';
  return `&stream_id=${encodeURIComponent(st.streamId)}&after=${st.lastSeq}`;
}

/**
 * Feed one socket event (`hello` or `console`) for an instance into the
 * singleton. Suppresses replayed lines below the clear marker.
 */
export function handleBroadcastEvent(
  instanceId: string,
  data: Record<string, unknown>,
): void {
  if (data.type === 'hello') {
    const st = stateFor(instanceId);
    const incoming = typeof data.stream_id === 'string' ? data.stream_id : null;
    if (st.streamId && incoming && incoming !== st.streamId) {
      // New backend session: the journal restarted, sequences reset, so the
      // old view and markers no longer mean anything.
      broadcasterFeed.clear();
      st.clearedThrough = null;
      st.lastSeq = null;
    }
    st.streamId = incoming;
    return;
  }
  if (data.type !== 'console') return;

  const st = stateFor(instanceId);
  const text = String(data.line ?? '');
  const seq = typeof data.sequence === 'number' ? data.sequence : null;
  if (seq !== null) {
    if (st.clearedThrough !== null && seq <= st.clearedThrough) return;
    st.lastSeq = st.lastSeq === null ? seq : Math.max(st.lastSeq, seq);
  }
  const level = (data.level as FeedLine['level'] | undefined) || detectLevel(text);
  broadcasterFeed.push(text, level, {
    sequence: seq ?? undefined,
    timestamp: typeof data.timestamp === 'number' ? data.timestamp : undefined,
  });
}

/** Test/diagnostic hook: wipe all per-instance bookkeeping. */
export function resetAll(): void {
  states.clear();
  loadedInstanceId = null;
}
