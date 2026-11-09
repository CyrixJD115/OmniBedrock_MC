/**
 * Unit tests for the explorer pretty-view parsers, run with Node's built-in
 * test runner:
 *
 *   npm test
 *
 * Pure helpers only (see frontend/src/lib/files/pretty.ts).
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { isPropertiesFile, isTreeFile, parseProperties, prettyView } from './pretty.ts';

test('parseProperties keeps pairs, comments, blanks, and inline comments', () => {
  const rows = parseProperties(
    '# top comment\n\nlevel-type=flat # world flavor\n!bang comment\nkey:colon-value\nonlykey'
  );
  assert.deepEqual(rows.map((r) => r.kind), ['comment', 'blank', 'pair', 'comment', 'pair', 'pair']);
  assert.equal(rows[2].key, 'level-type');
  assert.equal(rows[2].value, 'flat');
  assert.equal(rows[2].comment, '# world flavor');
  assert.equal(rows[4].key, 'key');
  assert.equal(rows[4].value, 'colon-value');
  // A separator-less line carries only a key.
  assert.equal(rows[5].key, 'onlykey');
  assert.equal(rows[5].value, '');
});

test('parseProperties handles escaped separators', () => {
  const rows = parseProperties('weird\\=key=real-value\n');
  assert.equal(rows[0].key, 'weird\\=key');
  assert.equal(rows[0].value, 'real-value');
});

test('prettyView routes by extension', () => {
  assert.equal(isTreeFile('config.json'), true);
  assert.equal(isTreeFile('config.yml'), true);
  assert.equal(isTreeFile('config.yaml'), true);
  assert.equal(isTreeFile('endstone_config.jsonc'), false);
  assert.equal(isPropertiesFile('server.properties'), true);
  assert.equal(isPropertiesFile('notes.txt'), false);
});

test('prettyView renders JSON as a tree and refuses broken JSON to raw', () => {
  const good = prettyView('a.json', '{"nested":{"list":[1,2]},"flag":true}');
  assert.equal(good?.kind, 'tree');
  assert.deepEqual((good as any).data, { nested: { list: [1, 2] }, flag: true });
  assert.equal(prettyView('a.json', '{broken'), null);
  // Scalars have nothing to expand — stay raw.
  assert.equal(prettyView('a.json', '"just a string"'), null);
});

test('prettyView renders YAML as a tree', () => {
  const view = prettyView('config.yml', 'session:\n  remote-address: 127.0.0.1\nfriends-sync:\n  enabled: true\n');
  assert.equal(view?.kind, 'tree');
  assert.deepEqual((view as any).data, {
    session: { 'remote-address': '127.0.0.1' },
    'friends-sync': { enabled: true },
  });
  assert.equal(prettyView('config.yml', 'a: [unclosed'), null);
});

test('prettyView renders .properties as a table and leaves others raw', () => {
  const view = prettyView('server.properties', 'gamemode=survival\n# note');
  assert.equal(view?.kind, 'properties');
  assert.equal((view as any).rows.length, 2);
  assert.equal(prettyView('plugin.py', 'print(1)'), null);
  assert.equal(prettyView('world.dat', 'binary-ish'), null);
});
