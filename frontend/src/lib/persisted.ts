import { writable, type Writable, type Unsubscriber } from 'svelte/store';

export function persisted<T>(key: string, defaultValue: T): Writable<T> {
  const store: Writable<T> = writable<T>(defaultValue, () => {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(key) : null;
    if (raw !== null) {
      try {
        store.set(JSON.parse(raw));
      } catch {
        store.set(defaultValue);
      }
    }

    const unsub: Unsubscriber = store.subscribe((v: T) => {
      if (typeof localStorage !== 'undefined') {
        localStorage.setItem(key, JSON.stringify(v));
      }
    });

    return unsub;
  });

  return store;
}
