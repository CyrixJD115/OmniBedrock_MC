import { writable } from 'svelte/store';

export function persisted<T>(key: string, defaultValue: T) {
  const store = writable<T>(defaultValue, () => {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(key) : null;
    if (raw !== null) {
      try {
        store.set(JSON.parse(raw));
      } catch {
        store.set(defaultValue);
      }
    }

    const unsub = store.subscribe(v => {
      if (typeof localStorage !== 'undefined') {
        localStorage.setItem(key, JSON.stringify(v));
      }
    });

    return unsub;
  });

  return store;
}
