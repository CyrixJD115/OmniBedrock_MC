// Bedrock reads JSON with // and /* */ comments, so pack files often carry
// them. Strip comments (outside strings) before parsing, and report the
// first syntax error as a 1-based line so the editor can point at it.

/** Remove // and /* *\/ comments that are not inside strings. */
export function stripJsonComments(text: string): string {
  let out = '';
  let inString = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      out += c;
      if (c === '\\') {
        out += text[i + 1] ?? '';
        i++;
      } else if (c === '"') {
        inString = false;
      }
      continue;
    }
    if (c === '"') {
      inString = true;
      out += c;
    } else if (c === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i++;
      out += '\n';
    } else if (c === '/' && text[i + 1] === '*') {
      i += 2;
      while (i < text.length && !(text[i] === '*' && text[i + 1] === '/')) {
        if (text[i] === '\n') out += '\n';
        i++;
      }
      i++;
    } else {
      out += c;
    }
  }
  return out;
}

/** Offset and reason of the first JSON syntax error, or null when valid. */
function locate(t: string): { at: number; why: string } | null {
  let i = 0;
  const ws = () => { while (i < t.length && ' \t\r\n'.includes(t[i])) i++; };
  const fail = (why: string): never => { throw { at: i, why }; };
  const str = () => {
    i++;
    while (i < t.length && t[i] !== '"') {
      if (t[i] === '\\') i++;
      else if (t[i] === '\n') fail('unterminated string');
      i++;
    }
    if (i >= t.length) fail('unterminated string');
    i++;
  };
  const value = (): void => {
    ws();
    const c = t[i];
    if (c === '{') {
      i++; ws();
      if (t[i] === '}') { i++; return; }
      for (;;) {
        ws();
        if (t[i] !== '"') fail(t[i] === '}' ? 'trailing comma before }' : 'expected a quoted key');
        str(); ws();
        if (t[i] !== ':') fail('expected ":" after key');
        i++; value(); ws();
        if (t[i] === ',') { i++; continue; }
        if (t[i] === '}') { i++; return; }
        fail('expected "," or "}"');
      }
    } else if (c === '[') {
      i++; ws();
      if (t[i] === ']') { i++; return; }
      for (;;) {
        value(); ws();
        if (t[i] === ',') { i++; ws(); if (t[i] === ']') fail('trailing comma before ]'); continue; }
        if (t[i] === ']') { i++; return; }
        fail('expected "," or "]"');
      }
    } else if (c === '"') {
      str();
    } else if (c !== undefined && /[-0-9]/.test(c)) {
      const m = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?/.exec(t.slice(i));
      if (!m) fail('invalid number');
      i += m![0].length;
    } else {
      for (const lit of ['true', 'false', 'null']) if (t.startsWith(lit, i)) { i += lit.length; return; }
      fail(c === undefined ? 'unexpected end of file' : `unexpected "${c}"`);
    }
  };
  try {
    value(); ws();
    if (i < t.length) fail('extra content after the JSON value');
    return null;
  } catch (e) {
    return e as { at: number; why: string };
  }
}

/** null when valid; otherwise "Line N: reason". */
export function jsonError(text: string): string | null {
  // A UTF-8 byte-order mark is common in pack files and harmless.
  const clean = stripJsonComments(text.replace(/^\uFEFF/, ''));
  const err = locate(clean);
  if (!err) return null;
  const line = clean.slice(0, err.at).split('\n').length;
  return `Line ${line}: ${err.why}`;
}
