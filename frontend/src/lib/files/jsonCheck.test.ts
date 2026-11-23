import { test } from 'node:test';
import assert from 'node:assert/strict';
import { jsonError, stripJsonComments } from './jsonCheck.ts';

test('comments are allowed, strings untouched', () => {
  const text = '{\n  // pack name\n  "url": "http://x.y/z", /* block\n comment */ "n": 1\n}';
  assert.equal(jsonError(text), null);
  assert.match(stripJsonComments(text), /http:\/\/x\.y\/z/);
});

test('syntax errors report a line', () => {
  const err = jsonError('{\n  "a": 1,\n  "b": \n}');
  assert.ok(err && /Line \d+/.test(err), String(err));
});
