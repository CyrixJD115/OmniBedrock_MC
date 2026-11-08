/**
 * Unit tests for the explorer path/breadcrumb helpers, run with Node's
 * built-in test runner:
 *
 *   npm test
 *
 * The helpers are pure on purpose — no svelte stores or API client — so
 * these run without any app bootstrap (see frontend/src/lib/files/paths.ts).
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { baseName, breadcrumbs, isTextFile, joinPath, normalizePath, parentPath } from './paths.ts';

test('normalizePath strips cosmetic separators and dot segments', () => {
  assert.equal(normalizePath(''), '');
  assert.equal(normalizePath('docs'), 'docs');
  assert.equal(normalizePath('/docs/'), 'docs');
  assert.equal(normalizePath('docs//notes.txt'), 'docs/notes.txt');
  assert.equal(normalizePath('./docs/./notes.txt'), 'docs/notes.txt');
  assert.equal(normalizePath('server\\worlds\\level.dat'), 'server/worlds/level.dat');
});

test('normalizePath rejects traversal instead of resolving it', () => {
  assert.equal(normalizePath('..'), null);
  assert.equal(normalizePath('../etc'), null);
  assert.equal(normalizePath('docs/../secret'), null);
  assert.equal(normalizePath('a/../../b'), null);
});

test('breadcrumbs always start at the root and accumulate paths', () => {
  assert.deepEqual(breadcrumbs(''), [{ name: 'root', path: '' }]);
  assert.deepEqual(breadcrumbs('docs'), [
    { name: 'root', path: '' },
    { name: 'docs', path: 'docs' },
  ]);
  assert.deepEqual(breadcrumbs('server/worlds'), [
    { name: 'root', path: '' },
    { name: 'server', path: 'server' },
    { name: 'worlds', path: 'server/worlds' },
  ]);
  // A malformed path still yields just the root rather than throwing.
  assert.deepEqual(breadcrumbs('../x'), [{ name: 'root', path: '' }]);
});

test('joinPath tolerates the root and preserves listing names', () => {
  assert.equal(joinPath('', 'notes.txt'), 'notes.txt');
  assert.equal(joinPath('docs', 'notes.txt'), 'docs/notes.txt');
  assert.equal(joinPath('server/worlds', 'level.dat'), 'server/worlds/level.dat');
});

test('parentPath walks back to the root', () => {
  assert.equal(parentPath(''), '');
  assert.equal(parentPath('notes.txt'), '');
  assert.equal(parentPath('docs/notes.txt'), 'docs');
  assert.equal(parentPath('server/worlds'), 'server');
});

test('baseName isolates the final segment', () => {
  assert.equal(baseName(''), '');
  assert.equal(baseName('notes.txt'), 'notes.txt');
  assert.equal(baseName('docs/notes.txt'), 'notes.txt');
  assert.equal(baseName('server/worlds'), 'worlds');
});

test('isTextFile gates the inline editor by extension', () => {
  for (const name of ['server.properties', 'config.yml', 'plugin.py', 'a.b.json', 'README.md']) {
    assert.ok(isTextFile(name), `${name} should be editable`);
  }
  for (const name of ['level.dat', 'pack.zip', 'mod.whl', 'world.ldb', 'noext', '.gitignore', 'image.png']) {
    assert.ok(!isTextFile(name), `${name} should be download-only`);
  }
});
