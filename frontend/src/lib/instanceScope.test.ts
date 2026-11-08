/**
 * Unit tests for the boot-time fallback instance selection
 * (`fallbackInstanceId`), run with Node's built-in test runner:
 *
 *   npm test
 *
 * The helper is pure on purpose — no svelte stores or API client — so these
 * run without any app bootstrap (see frontend/src/lib/instanceScope.ts).
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fallbackInstanceId } from './instanceScope.ts';

test('keeps the persisted selection while it still exists', () => {
  const list = [{ id: 'default' }, { id: 'game-night' }];
  assert.equal(fallbackInstanceId('game-night', list), 'game-night');
  assert.equal(fallbackInstanceId('default', list), 'default');
});

test('falls back to default when the persisted instance is gone', () => {
  // The renamed/deleted-instance case: the browser kept 'old-server'.
  const list = [{ id: 'default' }, { id: 'other' }];
  assert.equal(fallbackInstanceId('old-server', list), 'default');
});

test('falls back to the first entry when no default exists', () => {
  const list = [{ id: 'alpha' }, { id: 'beta' }];
  assert.equal(fallbackInstanceId('missing', list), 'alpha');
});

test('falls back to default when the list cannot be loaded', () => {
  assert.equal(fallbackInstanceId('stale-scope', []), 'default');
});

test('treats an empty persisted selection like a missing one', () => {
  assert.equal(fallbackInstanceId('', [{ id: 'default' }]), 'default');
  assert.equal(fallbackInstanceId('', [{ id: 'alpha' }]), 'alpha');
});

test('prefers default over the first entry', () => {
  const list = [{ id: 'first' }, { id: 'default' }];
  assert.equal(fallbackInstanceId('gone', list), 'default');
});
