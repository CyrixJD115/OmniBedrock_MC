/**
 * Pretty-view support for the file explorer's inline editor: parse known
 * text formats into renderable models. Pure string shaping only — no stores,
 * no API client — so the unit tests run without any app bootstrap:
 *
 *   npm test
 */
import { load as yamlLoad } from 'js-yaml';

/** One row of the .properties table (comments and blanks included). */
export interface PropertyRow {
  kind: 'pair' | 'comment' | 'blank';
  key: string;
  value: string;
  comment: string;
}

/** What the editor should render for a file, or null for raw-only files. */
export type PrettyView =
  | { kind: 'tree'; data: unknown }
  | { kind: 'properties'; rows: PropertyRow[] }
  | null;

/** Extensions whose text renders as a tree (JSON documents, YAML docs). */
const TREE_EXTENSIONS = ['json', 'yaml', 'yml'];

export function isPropertiesFile(name: string): boolean {
  return name.toLowerCase().endsWith('.properties');
}

export function isTreeFile(name: string): boolean {
  const lower = name.toLowerCase();
  return TREE_EXTENSIONS.some((ext) => lower.endsWith(`.${ext}`));
}

/**
 * Parse Minecraft-style .properties text into table rows. Everything is
 * preserved: full-line comments (# or !), blank lines, inline comments cut
 * off the value (Minecraft's own convention), and separator-less lines that
 * carry only a key.
 */
export function parseProperties(text: string): PropertyRow[] {
  return text.split(/\r?\n/).map((line) => {
    if (line.trim() === '') return { kind: 'blank' as const, key: '', value: '', comment: '' };
    const trimmed = line.trimStart();
    if (trimmed.startsWith('#') || trimmed.startsWith('!')) {
      return { kind: 'comment' as const, key: '', value: '', comment: trimmed };
    }
    const sep = findSeparator(line);
    if (sep === -1) {
      return { kind: 'pair' as const, key: line.trim(), value: '', comment: '' };
    }
    const key = line.slice(0, sep).trim();
    const rawValue = line.slice(sep + 1);
    const hash = rawValue.search(/(^|\s)#/);
    const value = (hash === -1 ? rawValue : rawValue.slice(0, hash)).trim();
    const comment = hash === -1 ? '' : rawValue.slice(hash).trim();
    return { kind: 'pair' as const, key, value, comment };
  });
}

/** First = or : that is not escaped with a backslash. */
function findSeparator(line: string): number {
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '\\') { i++; continue; }
    if (c === '=' || c === ':') return i;
  }
  return -1;
}

/**
 * Build the pretty view for one file: JSON and YAML parse into a tree,
 * .properties into a table, everything else stays raw. Unparseable content
 * (truncated JSON, bad YAML) also stays raw — the raw editor is always
 * available regardless.
 */
export function prettyView(name: string, text: string): PrettyView {
  if (isPropertiesFile(name)) {
    return { kind: 'properties', rows: parseProperties(text) };
  }
  if (isTreeFile(name)) {
    try {
      // js-yaml handles JSON as a subset of YAML 1.2, but the JSON path is
      // stricter about duplicates — prefer the native parser for .json.
      const data = name.toLowerCase().endsWith('.json')
        ? JSON.parse(text)
        : yamlLoad(text);
      if (data === undefined || data === null || typeof data !== 'object') return null;
      return { kind: 'tree', data };
    } catch {
      return null;
    }
  }
  return null;
}
