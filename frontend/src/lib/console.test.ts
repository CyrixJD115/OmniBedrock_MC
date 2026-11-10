/**
 * Unit tests for the console feed helpers: run collapsing (the ×N terminal
 * compaction) and the broadcaster console singleton (clear-survives-replay
 * markers and resume cursors), run with Node's built-in test runner:
 *
 *   npm test
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { collapseKey, collapseRuns, type FeedLine } from './console.ts';
import {
  broadcasterFeed,
  handleBroadcastEvent,
  clearView,
  resumeSuffix,
  isReconnect,
  markLoaded,
  resetFor,
  resetAll,
} from './broadcaster/console.ts';

function line(id: number, text: string, seq?: number): FeedLine {
  return { id, text, level: 'info', timestamp: 1000 + id, sequence: seq };
}

/** Feed pushes batch per animation frame (microtask under node): let them flush. */
async function settle(): Promise<void> {
  await new Promise((r) => setTimeout(r, 0));
}

test('collapseRuns merges consecutive duplicates and keeps distinct runs', () => {
  const runs = collapseRuns([
    line(1, '[Primary Session] Creating Xbox LIVE session...'),
    line(2, '[Primary Session] Updated session!'),
    line(3, '[Primary Session] Updated session!'),
    line(4, '[Primary Session] Updated session!'),
    line(5, 'some other line'),
    line(6, '[Primary Session] Updated session!'),
    line(7, '[Primary Session] Updated session!'),
  ]);
  assert.deepEqual(
    runs.map((r) => `${r.text} x${r.count}`),
    [
      '[Primary Session] Creating Xbox LIVE session... x1',
      '[Primary Session] Updated session! x3',
      'some other line x1',
      '[Primary Session] Updated session! x2',
    ],
    'the same text must form SEPARATE runs when a different line interrupts',
  );
});

test('collapseRuns carries the newest timestamp and sequence of a run', () => {
  const runs = collapseRuns([line(1, 'a', 10), line(2, 'a', 11), line(3, 'a', 12)]);
  assert.equal(runs[0].count, 3);
  assert.equal(runs[0].sequence, 12);
  assert.equal(runs[0].timestamp, 1003);
  // Key stays pinned to the run's first line so the DOM node is reused.
  assert.equal(runs[0].id, 1);
});

test('collapseRuns ignores the leading timestamp stamp when merging', () => {
  // The real jar stamps every line with the wall clock — byte-equality
  // would never merge these, which is the collapser's whole purpose.
  const runs = collapseRuns([
    line(1, '[16:37:28 INFO] [Primary Session] Updated session!', 1),
    line(2, '[16:37:59 INFO] [Primary Session] Updated session!', 2),
    line(3, '[16:38:29 INFO] [Primary Session] Updated session!', 3),
    line(4, '[16:38:59 INFO] [Primary Session] session failed!', 4),
    line(5, '[16:39:29 INFO] [Primary Session] Updated session!', 5),
  ]);
  assert.deepEqual(
    runs.map((r) => `${collapseKey(r.text)} x${r.count}`),
    [
      '[Primary Session] Updated session! x3',
      '[Primary Session] session failed! x1',
      '[Primary Session] Updated session! x1',
    ],
  );
  // The rendered text stays the FIRST occurrence, stamp included.
  assert.match(runs[0].text, /^\[16:37:28 INFO\]/);
});

test('collapseKey only strips genuine timestamp stamps', () => {
  assert.equal(collapseKey('[16:36:57 INFO] hello'), 'hello');
  assert.equal(collapseKey('[2026-11-09 13:24:42:608 INFO] hello'), 'hello');
  assert.equal(collapseKey('[16:36:57 WARNING] hello'), 'hello');
  // Bracketed message text without a timestamp+level stamp is untouched.
  assert.equal(collapseKey('[Primary Session] Updated session!'), '[Primary Session] Updated session!');
  assert.equal(collapseKey('[not-a-time INFO] x'), '[not-a-time INFO] x');
  assert.equal(collapseKey('16:36:57 INFO hello'), '16:36:57 INFO hello');
});

test('collapseRuns merges ANSI-colored jar lines across stamps', () => {
  // Log4j %highlight keeps emitting ANSI on Linux pipes even when JAnsi
  // reports itself unavailable — broadcaster lines arrive color-wrapped.
  const jar = (stamp: string, seq: number) =>
    line(seq, `\x1b[36m[${stamp}\x1b[m \x1b[36mINFO\x1b[m [Primary Session] Updated session!`, seq);
  const runs = collapseRuns([
    jar('16:36:57', 1),
    jar('16:37:28', 2),
    line(3, '\x1b[36m[16:37:59\x1b[m \x1b[36mINFO\x1b[m [Primary Session] session failed!', 3),
    line(4, '\r\n', 4), // CR-only fragment
    jar('16:38:29', 5),
  ]);
  assert.deepEqual(
    runs.map((r) => `${r.count}x`),
    ['2x', '1x', '1x', '1x'],
    'colored stamp variants merge; failures and blank lines stay separate',
  );
  // Whitespace jitter and missing close-bracket variants also merge.
  const j1 = line(10, '\x1b[36m[16:40:00\x1b[m  INFO  [Primary Session] Updated session!', 10);
  const j2 = line(11, '[16:40:30 \x1b[36mINFO\x1b[m [Primary Session] Updated session!', 11);
  const j3 = line(12, '[16:41:00 INFO [Primary Session] Updated session!', 12);
  const j4 = line(13, '[16:41:30 INFO] [Primary Session] Updated session!', 13);
  assert.equal(collapseRuns([j1, j2, j3, j4]).length, 1, 'all four forms are the same message');
});

test('collapseRuns recomputes over any slice (freeze boundaries)', () => {
  const all = collapseRuns([line(1, 'dup'), line(2, 'dup'), line(3, 'dup'), line(4, 'x')]);
  assert.equal(all[0].count, 3);
  // A frozen slice mid-run re-collapses from the slice, no stale total.
  const frozen = collapseRuns([line(1, 'dup'), line(2, 'dup')]);
  assert.equal(frozen[0].count, 2);
});

test('broadcaster: cleared view stays clear across a full replay', async () => {
  resetAll();
  broadcasterFeed.clear();
  const seen: string[] = [];
  const unsub = broadcasterFeed.subscribe((l) => {
    seen.length = 0;
    seen.push(...l.map((x) => x.text));
  });
  const inst = 'test-instance';
  markLoaded(inst);

  // Initial stream: hello + three lines.
  handleBroadcastEvent(inst, { type: 'hello', stream_id: 's1', resumed: false });
  handleBroadcastEvent(inst, { type: 'console', line: 'boot', sequence: 1 });
  handleBroadcastEvent(inst, { type: 'console', line: 'session', sequence: 2 });
  handleBroadcastEvent(inst, { type: 'console', line: 'session', sequence: 3 });
  await settle();
  assert.equal(seen.length, 3, JSON.stringify(seen));

  // User clears: marker lands on the highest sequence seen.
  clearView(inst);
  assert.equal(seen.length, 0);

  // Simulate a page remount with a FULL journal replay (no resume cursor).
  handleBroadcastEvent(inst, { type: 'hello', stream_id: 's1', resumed: false });
  handleBroadcastEvent(inst, { type: 'console', line: 'boot', sequence: 1 });
  handleBroadcastEvent(inst, { type: 'console', line: 'session', sequence: 2 });
  handleBroadcastEvent(inst, { type: 'console', line: 'session', sequence: 3 });
  await settle();
  assert.equal(seen.length, 0, 'replayed lines at/below the clear marker must be dropped');

  // New live lines still arrive.
  handleBroadcastEvent(inst, { type: 'console', line: 'live', sequence: 4 });
  await settle();
  assert.deepEqual(seen, ['live']);
  unsub();
});

test('broadcaster: reconnects build an after=<lastSeq> resume cursor', () => {
  resetAll();
  broadcasterFeed.clear();
  const inst = 'cursor-instance';
  markLoaded(inst);
  assert.equal(resumeSuffix(inst), '', 'no cursor before any line');
  handleBroadcastEvent(inst, { type: 'hello', stream_id: 'stream-7', resumed: false });
  handleBroadcastEvent(inst, { type: 'console', line: 'a', sequence: 41 });
  assert.equal(resumeSuffix(inst), '&stream_id=stream-7&after=41');
  assert.ok(isReconnect(inst));
});

test('broadcaster: a new stream id resets the view and markers', async () => {
  resetAll();
  broadcasterFeed.clear();
  const seen: string[] = [];
  const unsub = broadcasterFeed.subscribe((l) => {
    seen.length = 0;
    seen.push(...l.map((x) => x.text));
  });
  const inst = 'restart-instance';
  markLoaded(inst);
  handleBroadcastEvent(inst, { type: 'hello', stream_id: 's1', resumed: false });
  handleBroadcastEvent(inst, { type: 'console', line: 'old-session', sequence: 900 });
  clearView(inst);

  // Backend restarted: journal sequences start over under a new stream id.
  handleBroadcastEvent(inst, { type: 'hello', stream_id: 's2', resumed: false });
  handleBroadcastEvent(inst, { type: 'console', line: 'fresh-boot', sequence: 1 });
  await settle();
  assert.deepEqual(seen, ['fresh-boot'], 'sequence reset must not be swallowed by the old marker');
  assert.equal(resumeSuffix(inst), '&stream_id=s2&after=1');
  unsub();
});

test('broadcaster: instance switch resets bookkeeping for both sides', () => {
  resetAll();
  broadcasterFeed.clear();
  const instA = 'switch-a';
  const instB = 'switch-b';
  markLoaded(instA);
  handleBroadcastEvent(instA, { type: 'hello', stream_id: 'a1', resumed: false });
  handleBroadcastEvent(instA, { type: 'console', line: 'a-line', sequence: 5 });
  clearView(instA);

  // Switch to B: A's marker is dropped, so returning replays A fresh.
  resetFor(instA);
  markLoaded(instB);
  assert.equal(isReconnect(instA), false, 'returning to A after a switch is a fresh load');
  assert.equal(isReconnect(instB), true);
  assert.equal(resumeSuffix(instA), '', 'returning to A must replay from scratch');
});
