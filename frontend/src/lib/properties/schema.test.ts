import { test } from 'node:test';
import assert from 'node:assert/strict';
import { descriptions, fromComment, specFor } from './schema.ts';

test('BDS comments below a key become its description', () => {
  const entries = [
    { key: 'compression-algorithm', comment: '' },
    { key: '', comment: '# Determines the compression algorithm to use for networking' },
    { key: '', comment: '# Allowed values: "zlib", "snappy"' },
    { key: '', comment: '' },
    { key: 'view-distance', comment: '' },
  ];
  const d = descriptions(entries);
  assert.match(d.get('compression-algorithm') ?? '', /compression algorithm/);
  assert.equal(d.has('view-distance'), false);
});

test('allowed-values comments pick the widget for unknown keys', () => {
  assert.deepEqual(fromComment('Allowed values: "true" or "false"'), { kind: 'bool' });
  assert.deepEqual(fromComment('Allowed values: "a", "b", "c"'), { kind: 'enum', options: ['a', 'b', 'c'] });
  assert.deepEqual(fromComment('Allowed values: 0-65535'), { kind: 'number', min: 0, max: 65535 });
  assert.equal(specFor('future-flag', 'x', 'Allowed values: "true" or "false"').kind, 'bool');
  assert.equal(specFor('allow-list', 'false').category, 'players');
});
