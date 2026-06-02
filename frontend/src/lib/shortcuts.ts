let active = true;
let listeners: Array<() => void> = [];

interface Shortcut {
  key: string;
  ctrl?: boolean;
  shift?: boolean;
  description: string;
  handler: () => void;
}

const shortcuts: Shortcut[] = [];

export function registerShortcut(s: Shortcut): () => void {
  shortcuts.push(s);
  return () => {
    const idx = shortcuts.indexOf(s);
    if (idx >= 0) shortcuts.splice(idx, 1);
  };
}

function handleKey(e: KeyboardEvent) {
  if (!active) return;
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
    if (e.key === 'Escape') {
      (e.target as HTMLElement).blur();
    }
    if (!e.ctrlKey && !e.metaKey) return;
  }

  for (const s of shortcuts) {
    if (
      e.key.toLowerCase() === s.key.toLowerCase() &&
      !!e.ctrlKey === !!s.ctrl &&
      !!e.shiftKey === !!s.shift
    ) {
      e.preventDefault();
      s.handler();
      return;
    }
  }
}

export function initShortcuts() {
  if (typeof window === 'undefined') return;
  window.addEventListener('keydown', handleKey);
  return () => window.removeEventListener('keydown', handleKey);
}

export function showHelp(): string {
  return shortcuts
    .map(s => {
      const mod = s.ctrl ? 'Ctrl+' : '';
      const shift = s.shift ? 'Shift+' : '';
      return `${mod}${shift}${s.key.toUpperCase()} - ${s.description}`;
    })
    .join('\n');
}
