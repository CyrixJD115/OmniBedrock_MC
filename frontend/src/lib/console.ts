import { AnsiUp } from 'ansi_up';

const ansi = new AnsiUp();

ansi.use_classes = false;

const LEVEL_RE = /\[(ERROR|WARN(?:ING)?|INFO|DEBUG|LOCAL)\]/i;
const ERROR_KEYWORDS = /(?:error|exception|traceback|fatal)/i;
const TAG_RE = /(<[^>]*>)|([^<]+)/g;

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

export function matchesQuery(text: string, query: string, useRegex: boolean): boolean {
  if (!query) return true;
  try {
    if (useRegex) {
      return new RegExp(query, 'i').test(text);
    }
    return text.toLowerCase().includes(query.toLowerCase());
  } catch {
    return text.toLowerCase().includes(query.toLowerCase());
  }
}

export function highlightMatches(html: string, query: string, useRegex: boolean): string {
  if (!query) return html;
  let re: RegExp;
  try {
    re = useRegex ? new RegExp(query, 'gi') : new RegExp(escapeRegex(query), 'gi');
  } catch {
    return html;
  }

  return html.replace(TAG_RE, (_: string, tag: string, text: string) => {
    if (tag) return tag;
    return text.replace(re, m => `<mark class="console-match">${m}</mark>`);
  });
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
