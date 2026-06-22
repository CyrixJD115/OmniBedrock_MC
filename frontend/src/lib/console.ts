import { AnsiUp } from 'ansi_up';

const ansi = new AnsiUp();

ansi.use_classes = false;

const LEVEL_RE = /\[(ERROR|WARN(?:ING)?|INFO|DEBUG|LOCAL)\]/i;
const ERROR_KEYWORDS = /(?:error|exception|traceback|fatal)/i;

export function parseAnsi(text: string): string {
  return ansi.ansi_to_html(text);
}

export function detectLevel(text: string): 'error' | 'warn' | 'info' | 'debug' | 'local' {
  const m = LEVEL_RE.exec(text);
  if (m) {
    const token = m[1].toUpperCase();
    if (token === 'ERROR') return 'error';
    if (token === 'WARN' || token === 'WARNING') return 'warn';
    if (token === 'DEBUG') return 'debug';
    if (token === 'LOCAL') return 'local';
    return 'info';
  }
  if (ERROR_KEYWORDS.test(text)) return 'error';
  return 'info';
}
