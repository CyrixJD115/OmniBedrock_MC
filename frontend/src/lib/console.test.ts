/**
 * Unit tests for the console feed helpers: run collapsing (the ×N terminal
 * compaction) and the broadcaster console singleton (clear-survives-replay
 * markers and resume cursors), run with Node's built-in test runner:
 *
 *   npm test
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { collapseRuns, type FeedLine } from './console.ts';
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
