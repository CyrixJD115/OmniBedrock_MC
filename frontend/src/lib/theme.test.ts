/**
 * Unit tests for the theme catalog invariants, run with Node's built-in
 * test runner (npm test). The settings grid renders `{#each THEMES as t
 * (t.id)}` — a duplicate id throws `each_key_duplicate` at render and
 * blanks the whole page, so catalog uniqueness is a load-bearing invariant.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { THEMES, THEME_MODES, accentRamp } from './theme.ts';

test('theme ids are unique — the settings grid keys on them', () => {
  const ids = THEMES.map((t) => t.id);
  assert.equal(new Set(ids).size, ids.length, `duplicate theme ids: ${ids.join(', ')}`);
});

test('mode ids are unique and every theme carries picker metadata', () => {
  const modeIds = THEME_MODES.map((m) => m.id);
  assert.equal(new Set(modeIds).size, modeIds.length);
  for (const t of THEMES) {
    assert.ok(t.label, `${t.id} lacks a label`);
    assert.ok(t.hint, `${t.id} lacks a hint`);
    assert.equal(t.swatch.length, 2, `${t.id} needs two swatch colors`);
    for (const swatch of t.swatch) {
      assert.match(swatch, /^#[0-9a-fA-F]{6}$/, `${t.id} swatch ${swatch} is not #rrggbb`);
    }
  }
});

test('accentRamp derives 11 valid RGB triplets for dark and light', () => {
  const { dark, light } = accentRamp('#38bdf8');
  const triplet = (v: string) => /^(\d{1,3} ){2}\d{1,3}$/.test(v) && v.split(' ').every((c) => +c >= 0 && +c <= 255);
  assert.equal(dark.length, 11);
  assert.equal(light.length, 11);
  for (const v of [...dark, ...light]) assert.ok(triplet(v), `bad triplet: ${v}`);
  // Light mode deepens the 300/400 text stops (indices 3/4) for contrast.
  assert.equal(light[3], dark[7]);
  assert.equal(light[4], dark[6]);
  // Invalid hex falls back instead of throwing.
  assert.doesNotThrow(() => accentRamp('not-a-color'));
});
