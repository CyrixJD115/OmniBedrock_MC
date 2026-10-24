// The "currently selected world" shared across pages (Addon Organizer and the
// CurseForge Store) so Store installs always target the world being managed.
const KEY = 'omb_selected_world';

export function savedWorld(): string {
  try {
    return localStorage.getItem(KEY) ?? '';
  } catch {
    return '';
  }
}

export function saveWorld(world: string): void {
  try {
    if (world) localStorage.setItem(KEY, world);
    else localStorage.removeItem(KEY);
  } catch {
    /* storage unavailable */
  }
}
