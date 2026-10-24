/**
 * Theme engine — accent palettes × light/dark mode.
 *
 * Themes are pure data: each id maps to a CSS variable block in app.css.
 * Switching rewrites `data-theme` / `data-mode` attributes on <html>; nothing
 * else in the app needs to know a theme exists. Preferences persist through
 * the same localStorage-backed store used by other panel state, and
 * app.html applies them pre-paint so there is no flash of the wrong theme.
 */
import { derived } from 'svelte/store';
import { persisted } from '$lib/persisted';

export type ThemeId =
  | 'cyan' | 'minecraft' | 'blue' | 'purple'
  | 'red' | 'orange' | 'pink' | 'teal';

/** 'system' follows the OS preference; the other two are explicit. */
export type ThemeMode = 'dark' | 'light' | 'system';

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
  { id: 'red', label: 'Red', hint: 'Nether-hot accent', swatch: ['#f87171', '#dc2626'] },
  { id: 'orange', label: 'Orange', hint: 'Copper & lava', swatch: ['#fb923c', '#ea580c'] },
  { id: 'pink', label: 'Pink', hint: 'Cherry blossom', swatch: ['#f472b6', '#db2777'] },
  { id: 'teal', label: 'Teal', hint: 'Deep ocean', swatch: ['#2dd4bf', '#0d9488'] },
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

/** Persisted accent choice. Keys mirror the pre-paint script in app.html. */
export const theme = persisted<ThemeId>('omb.theme', 'cyan');
export const themeMode = persisted<ThemeMode>('omb.theme-mode', 'dark');

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
}

/**
 * Wire up live switching. Called once from the root layout; the pre-paint
 * script in app.html already set the attributes before hydration.
 */
export function initTheme(): () => void {
  applyToDocument(readStored('omb.theme', isThemeId, 'cyan'), readStored('omb.theme-mode', isThemeMode, 'dark'));

  const unsubTheme = theme.subscribe((id) => {
    if (!isThemeId(id)) return;
    applyToDocument(id, readStored('omb.theme-mode', isThemeMode, 'dark'));
  });
  const unsubMode = themeMode.subscribe((mode) => {
    if (!isThemeMode(mode)) return;
    applyToDocument(readStored('omb.theme', isThemeId, 'cyan'), mode);
  });

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
