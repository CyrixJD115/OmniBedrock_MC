/**
 * Theme engine — accent palettes × light/dark mode × UI density × font scale.
 *
 * Preset themes are pure data: each id maps to a CSS variable block in
 * app.css. The custom theme instead derives its accent ramp from one user
 * hex (`accentRamp`) and sets the same variables inline on <html>. Switching
 * rewrites `data-theme` / `data-mode` / `data-density` attributes on <html>;
 * nothing else in the app needs to know a theme exists. Preferences persist
 * through the same localStorage-backed store used by other panel state, and
 * app.html applies them pre-paint so there is no flash of the wrong theme
 * (its inline script carries a byte-equivalent copy of `accentRamp` — keep
 * the two in sync).
 */
import { derived } from 'svelte/store';
import { persisted } from './persisted.ts';

export type ThemeId =
  | 'cyan' | 'minecraft' | 'blue' | 'purple'
  | 'red' | 'orange' | 'pink' | 'teal'
  | 'emerald' | 'lime' | 'amber' | 'rose'
  | 'fuchsia' | 'sky' | 'indigo' | 'slate'
  | 'custom';

/** 'system' follows the OS preference; the other two are explicit. */
export type ThemeMode = 'dark' | 'light' | 'system';

export type UiDensity = 'comfortable' | 'compact';

export interface ThemeMeta {
  id: ThemeId;
  label: string;
  /** Short blurb shown under the label in the picker. */
  hint: string;
  /** Two swatch colors (CSS hex) previewing the accent in the picker. */
  swatch: [string, string];
}

export const THEMES: ThemeMeta[] = [
  { id: 'cyan', label: 'Cyan', hint: 'OmniBedrock default', swatch: ['#22d3ee', '#0891b2'] },
  { id: 'minecraft', label: 'Minecraft', hint: 'Bedrock green & stone', swatch: ['#4aa73e', '#3c8527'] },
  { id: 'blue', label: 'Blue', hint: 'Classic panel blue', swatch: ['#60a5fa', '#2563eb'] },
  { id: 'purple', label: 'Purple', hint: 'End portal vibes', swatch: ['#a78bfa', '#7c3aed'] },
  { id: 'emerald', label: 'Emerald', hint: 'Deep gemstone green', swatch: ['#34d399', '#059669'] },
  { id: 'lime', label: 'Lime', hint: 'Creeper-bright green', swatch: ['#a3e635', '#65a30d'] },
  { id: 'amber', label: 'Amber', hint: 'Gold block shine', swatch: ['#fbbf24', '#d97706'] },
  { id: 'orange', label: 'Orange', hint: 'Copper & lava', swatch: ['#fb923c', '#ea580c'] },
  { id: 'red', label: 'Red', hint: 'Nether-hot accent', swatch: ['#f87171', '#dc2626'] },
  { id: 'rose', label: 'Rose', hint: 'Crimson bloom', swatch: ['#fb7185', '#e11d48'] },
  { id: 'pink', label: 'Pink', hint: 'Cherry blossom', swatch: ['#f472b6', '#db2777'] },
  { id: 'fuchsia', label: 'Fuchsia', hint: 'Dragon-egg purple', swatch: ['#e879f9', '#c026d3'] },
  { id: 'indigo', label: 'Indigo', hint: 'Deep dusk blue', swatch: ['#818cf8', '#4f46e5'] },
  { id: 'sky', label: 'Sky', hint: 'Open daylight blue', swatch: ['#38bdf8', '#0284c7'] },
  { id: 'teal', label: 'Teal', hint: 'Deep ocean', swatch: ['#2dd4bf', '#0d9488'] },
  { id: 'slate', label: 'Slate', hint: 'Stone, no color cast', swatch: ['#94a3b8', '#475569'] },
  { id: 'custom', label: 'Custom', hint: 'Your own accent color', swatch: ['#38bdf8', '#0284c7'] },
];

export const THEME_MODES: { id: ThemeMode; label: string }[] = [
  { id: 'dark', label: 'Dark' },
  { id: 'light', label: 'Light' },
  { id: 'system', label: 'System' },
];

const isThemeId = (v: unknown): v is ThemeId =>
  typeof v === 'string' && THEMES.some((t) => t.id === v);
const isThemeMode = (v: unknown): v is ThemeMode =>
  v === 'dark' || v === 'light' || v === 'system';
const isDensity = (v: unknown): v is UiDensity =>
  v === 'comfortable' || v === 'compact';

/** Persisted accent choice. Keys mirror the pre-paint script in app.html. */
export const theme = persisted<ThemeId>('omb.theme', 'cyan');
export const themeMode = persisted<ThemeMode>('omb.theme-mode', 'dark');
/** Base color for the 'custom' theme (any #rrggbb). */
export const customAccent = persisted<string>('omb.theme-custom-hex', '#38bdf8');
export const uiDensity = persisted<UiDensity>('omb.ui-density', 'comfortable');
/** UI font-size multiplier (0.9–1.2); drives the root font-size. */
export const uiFontScale = persisted<number>('omb.ui-font-scale', 1);

export const isHexColor = (v: unknown): v is string =>
  typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v);

const CUSTOM_FALLBACK = '#38bdf8';

// ─── custom accent ramp ───────────────────────────────────────────────────────

export const ACCENT_STOPS = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'] as const;

/** Lightness per ramp stop; mirrors the hand-tuned preset ramps. */
const STOP_LIGHTNESS = [0.97, 0.94, 0.86, 0.76, 0.66, 0.56, 0.48, 0.41, 0.34, 0.28, 0.20];
/** Saturation tapers off toward the pale end so 50/100 don't look neon. */
const STOP_SATURATION = [0.35, 0.5, 0.75, 0.9, 0.95, 1, 1, 1, 1, 1, 1];

function hslToRgbTriplet(h: number, s: number, l: number): string {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const hp = (((h % 360) + 360) % 360) / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r = 0, g = 0, b = 0;
  if (hp < 1) { r = c; g = x; }
  else if (hp < 2) { r = x; g = c; }
  else if (hp < 3) { g = c; b = x; }
  else if (hp < 4) { g = x; b = c; }
  else if (hp < 5) { r = x; b = c; }
  else { r = c; b = x; }
  const m = l - c / 2;
  const to255 = (v: number) => Math.round(Math.min(255, Math.max(0, (v + m) * 255)));
  return `${to255(r)} ${to255(g)} ${to255(b)}`;
}

/**
 * Derive the full accent ramp from one base hex. `dark` is the saturated
 * ramp used in dark mode; `light` mirrors the hand-tuned light overrides in
 * app.css (300/400 deepen for contrast, the pale end stays). Values are
 * 'r g b' channel triplets ready for `--bedrock-<stop>`.
 */
export function accentRamp(hex: string): { dark: string[]; light: string[] } {
  if (!isHexColor(hex)) return accentRamp(CUSTOM_FALLBACK);
  const r0 = parseInt(hex.slice(1, 3), 16) / 255;
  const g0 = parseInt(hex.slice(3, 5), 16) / 255;
  const b0 = parseInt(hex.slice(5, 7), 16) / 255;
  const max = Math.max(r0, g0, b0);
  const min = Math.min(r0, g0, b0);
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r0) h = (g0 - b0) / d + (g0 < b0 ? 6 : 0);
    else if (max === g0) h = (b0 - r0) / d + 2;
    else h = (r0 - g0) / d + 4;
    h *= 60;
  }
  const dark = STOP_LIGHTNESS.map((stopL, i) => hslToRgbTriplet(h, s * STOP_SATURATION[i], stopL));
  // Light mode: 300/400 deepen to the 700/600 stops (text contrast), 200
  // keeps a soft tint — same recipe as the preset light blocks in app.css.
  const light = [...dark];
  light[3] = dark[7];
  light[4] = dark[6];
  light[5] = dark[5];
  return { dark, light };
}

function readStored<T>(key: string, guard: (v: unknown) => v is T, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw !== null) {
      const parsed = JSON.parse(raw);
      if (guard(parsed)) return parsed;
    }
  } catch { /* corrupt or unavailable — keep the default */ }
  return fallback;
}

export function resolvedMode(mode: ThemeMode = themeModeCurrent()): 'dark' | 'light' {
  if (mode !== 'system') return mode;
  if (typeof matchMedia === 'undefined') return 'dark';
  return matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

function themeModeCurrent(): ThemeMode {
  // Direct read so the derived store below stays honest without a subscribe.
  return readStored('omb.theme-mode', isThemeMode, 'dark');
}

/** The mode actually painted right now (resolves 'system'). */
export const activeMode = derived(themeMode, ($mode) => resolvedMode($mode));

let mediaListener: (() => void) | null = null;

function applyToDocument(id: ThemeId, mode: ThemeMode) {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  root.dataset.theme = id;
  const resolved = resolvedMode(mode);
  root.dataset.mode = resolved;
  // Keep the legacy class in sync for the handful of places (and browser
  // extensions) that expect a dark-mode marker.
  root.classList.toggle('dark', resolved === 'dark');

  // The custom theme has no static CSS block: its ramp is set inline. Any
  // leftover inline values from a previous custom session are removed so
  // presets are not shadowed.
  const hex = readStored('omb.theme-custom-hex', isHexColor, CUSTOM_FALLBACK);
  if (id === 'custom') {
    const { dark, light } = accentRamp(hex);
    const ramp = resolved === 'light' ? light : dark;
    ACCENT_STOPS.forEach((stop, i) => root.style.setProperty(`--bedrock-${stop}`, ramp[i]));
  } else {
    ACCENT_STOPS.forEach((stop) => root.style.removeProperty(`--bedrock-${stop}`));
  }
}

function applyUi(root: HTMLElement = document.documentElement) {
  root.dataset.density = readStored('omb.ui-density', isDensity, 'comfortable');
  const scale = readStored('omb.ui-font-scale', (v): v is number => typeof v === 'number' && v >= 0.9 && v <= 1.2, 1);
  root.style.fontSize = `${Math.round(scale * 100)}%`;
}

/**
 * Wire up live switching. Called once from the root layout; the pre-paint
 * script in app.html already set the attributes before hydration.
 */
export function initTheme(): () => void {
  const root = document.documentElement;
  applyToDocument(readStored('omb.theme', isThemeId, 'cyan'), readStored('omb.theme-mode', isThemeMode, 'dark'));
  applyUi(root);

  const unsubTheme = theme.subscribe((id) => {
    if (!isThemeId(id)) return;
    applyToDocument(id, readStored('omb.theme-mode', isThemeMode, 'dark'));
  });
  const unsubMode = themeMode.subscribe((mode) => {
    if (!isThemeMode(mode)) return;
    applyToDocument(readStored('omb.theme', isThemeId, 'cyan'), mode);
  });
  const unsubAccent = customAccent.subscribe(() => {
    if (readStored('omb.theme', isThemeId, 'cyan') !== 'custom') return;
    applyToDocument('custom', readStored('omb.theme-mode', isThemeMode, 'dark'));
  });
  const unsubDensity = uiDensity.subscribe(() => applyUi(root));
  const unsubScale = uiFontScale.subscribe(() => applyUi(root));

  if (typeof matchMedia !== 'undefined') {
    const mq = matchMedia('(prefers-color-scheme: light)');
    const onChange = () => {
      if (themeModeCurrent() === 'system') {
        applyToDocument(readStored('omb.theme', isThemeId, 'cyan'), 'system');
      }
    };
    mq.addEventListener('change', onChange);
    mediaListener = () => mq.removeEventListener('change', onChange);
  }

  return () => {
    unsubTheme();
    unsubMode();
    unsubAccent();
    unsubDensity();
    unsubScale();
    mediaListener?.();
    mediaListener = null;
  };
}

/** Cycle dark → light → system, used by the sidebar quick toggle. */
export function cycleMode(): ThemeMode {
  const order: ThemeMode[] = ['dark', 'light', 'system'];
  const current = readStored('omb.theme-mode', isThemeMode, 'dark');
  const next = order[(order.indexOf(current) + 1) % order.length];
  themeMode.set(next);
  return next;
}
