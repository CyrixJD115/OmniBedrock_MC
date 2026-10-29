// Visual manifest editor model: converts a raw manifest.json document into a
// form-friendly draft and applies draft edits back onto the raw JSON text.
// Both directions are pure — the caller owns the editor state and error toasts.

export interface ManifestDraftSubpack {
  name: string;
  folder: string;
  tier: string;
}

export interface ManifestDraft {
  name: string;
  description: string;
  v0: number; v1: number; v2: number;
  m0: number; m1: number; m2: number;
  uuid: string;
  authors: string;
  subpacks: ManifestDraftSubpack[];
}

/** Parse a raw manifest document into a draft; null when the JSON is invalid. */
export function parseDraft(text: string): ManifestDraft | null {
  try {
    const m = JSON.parse(text);
    const h = m?.header ?? {};
    const v = Array.isArray(h.version) ? h.version : [0, 0, 0];
    const mv = Array.isArray(h.min_engine_version) ? h.min_engine_version : [1, 0, 0];
    return {
      name: typeof h.name === 'string' ? h.name : '',
      description: typeof h.description === 'string' ? h.description : '',
      v0: v[0] ?? 0, v1: v[1] ?? 0, v2: v[2] ?? 0,
      m0: mv[0] ?? 1, m1: mv[1] ?? 0, m2: mv[2] ?? 0,
      uuid: typeof h.uuid === 'string' ? h.uuid : '',
      authors: Array.isArray(m?.metadata?.authors) ? m.metadata.authors.join(', ') : '',
      subpacks: Array.isArray(m?.subpacks)
        ? m.subpacks.map((s: any) => ({
            name: typeof s?.name === 'string' ? s.name : '',
            folder: typeof s?.folder_name === 'string' ? s.folder_name : '',
            tier: Number.isFinite(s?.memory_tier) ? String(s.memory_tier) : '',
          }))
        : [],
    };
  } catch {
    return null;
  }
}

/**
 * Merge draft edits into the raw manifest text and return the serialized
 * document. Throws when the raw text is not valid JSON (caller toasts).
 */
export function applyDraft(draft: ManifestDraft, rawText: string): string {
  {
    const m = JSON.parse(rawText);
    if (!m.header) m.header = {};
    m.header.name = draft.name;
    if (draft.description) m.header.description = draft.description;
    else delete m.header.description;
    m.header.version = [Number(draft.v0) || 0, Number(draft.v1) || 0, Number(draft.v2) || 0];
    m.header.min_engine_version = [Number(draft.m0) || 0, Number(draft.m1) || 0, Number(draft.m2) || 0];
    if (draft.uuid.trim()) m.header.uuid = draft.uuid.trim();
    const authors = draft.authors.split(',').map((a: string) => a.trim()).filter(Boolean);
    if (authors.length > 0) {
      if (!m.metadata) m.metadata = {};
      m.metadata.authors = authors;
    } else if (m.metadata) {
      delete m.metadata.authors;
    }
    // Rows without a folder_name are dropped; memory_tier is only written
    // when it parses as an integer (the game treats it as optional). Number
    // inputs hand Svelte real numbers, so coerce defensively.
    const subpacks = draft.subpacks
      .map((s) => {
        const sub: any = {};
        if (String(s.name ?? '').trim()) sub.name = String(s.name).trim();
        if (String(s.folder ?? '').trim()) sub.folder_name = String(s.folder).trim();
        const tierText = String(s.tier ?? '').trim();
        const tier = Number(tierText);
        if (tierText !== '' && Number.isInteger(tier)) sub.memory_tier = tier;
        return sub;
      })
      .filter((s) => s.folder_name);
    if (subpacks.length > 0) m.subpacks = subpacks;
    else delete m.subpacks;
    return JSON.stringify(m, null, 2);
  }
}
