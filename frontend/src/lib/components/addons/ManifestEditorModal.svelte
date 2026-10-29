<!-- Manifest editor modal: visual form and raw JSON editing of a pack's
     manifest.json. Owns the draft state; the page persists via onsave(raw). -->
<script lang="ts">
  import { Shuffle } from '@lucide/svelte';
  import { addToast } from '$stores/toast';
  import { applyDraft, parseDraft, type ManifestDraft } from '$lib/addons/manifestDraft';

  let {
    open = $bindable(false),
    onsave,
  }: {
    open?: boolean;
    onsave: (raw: string) => void;
  } = $props();

  let draft = $state<ManifestDraft>({
    name: '',
    description: '',
    v0: 0, v1: 0, v2: 0,
    m0: 0, m1: 0, m2: 0,
    uuid: '',
    authors: '',
    subpacks: [],
  });
  let raw = $state('');
  let valid = $state(true);
  let mode = $state<'visual' | 'raw'>('visual');

  /** Reset the editor from a manifest document and open the modal. */
  export function openWith(manifest: Record<string, unknown> | null) {
    raw = JSON.stringify(manifest ?? {}, null, 2);
    valid = true;
    mode = 'visual';
    draft = parseDraft(raw) ?? draft;
    open = true;
  }

  function addSubpack() {
    draft.subpacks.push({ name: '', folder: '', tier: '' });
  }
  function removeSubpack(i: number) {
    draft.subpacks.splice(i, 1);
  }

  function setManifestMode(m: 'visual' | 'raw') {
    if (m === mode) return;
    if (m === 'visual') {
      const d = parseDraft(raw);
      if (!d) {
        addToast('Invalid JSON — fix the raw editor first', 'error');
        return;
      }
      draft = d;
    } else {
      try {
        raw = applyDraft(draft, raw);
      } catch {
        return;
      }
    }
    mode = m;
  }

  function validateJson() {
    try {
      JSON.parse(raw);
      valid = true;
    } catch {
      valid = false;
    }
  }

  function save() {
    if (mode === 'visual') {
      try {
        raw = applyDraft(draft, raw);
      } catch (e: any) {
        addToast(`Cannot apply visual edits: ${e.message}`, 'error');
        return;
      }
    }
    if (!valid) return;
    onsave(raw);
  }
</script>

{#if open}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) open = false; }}
       onkeydown={(e) => e.key === 'Escape' && (open = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-2xl shadow-block-lg">
      <div class="flex items-center justify-between mb-3">
        <h2 class="text-sm font-bold text-strong uppercase tracking-widest">Edit Manifest</h2>
        <div class="flex border border-deep-600/40 rounded overflow-hidden">
          <button onclick={() => setManifestMode('visual')}
                  class="px-3 py-1 text-[10px] uppercase tracking-wider {mode === 'visual' ? 'bg-bedrock-500/20 text-bedrock-300' : 'text-deep-400 hover:text-strong'}">Visual</button>
          <button onclick={() => setManifestMode('raw')}
                  class="px-3 py-1 text-[10px] uppercase tracking-wider border-l border-deep-600/40 {mode === 'raw' ? 'bg-bedrock-500/20 text-bedrock-300' : 'text-deep-400 hover:text-strong'}">Raw</button>
        </div>
      </div>

      {#if mode === 'visual'}
        <div class="space-y-3 max-h-96 overflow-y-auto pr-1">
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Name</span>
            <input bind:value={draft.name} class="input w-full text-xs" />
          </label>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Description</span>
            <textarea bind:value={draft.description} class="input w-full h-16 text-xs" ></textarea>
          </label>
          <div class="grid grid-cols-2 gap-3">
            <div>
              <span class="text-deep-400 text-[10px] uppercase tracking-wider block mb-1">Version</span>
              <div class="grid grid-cols-3 gap-1.5">
                <input type="number" min="0" bind:value={draft.v0} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={draft.v1} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={draft.v2} class="input text-xs text-center" />
              </div>
            </div>
            <div>
              <span class="text-deep-400 text-[10px] uppercase tracking-wider block mb-1">Min engine version</span>
              <div class="grid grid-cols-3 gap-1.5">
                <input type="number" min="0" bind:value={draft.m0} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={draft.m1} class="input text-xs text-center" />
                <input type="number" min="0" bind:value={draft.m2} class="input text-xs text-center" />
              </div>
            </div>
          </div>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">UUID</span>
            <div class="flex gap-2">
              <input bind:value={draft.uuid} class="input flex-1 text-xs font-mono" />
              <button onclick={() => (draft.uuid = crypto.randomUUID())}
                      class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1 shrink-0">
                <Shuffle size={11} /> New
              </button>
            </div>
          </label>
          <label class="space-y-1 block">
            <span class="text-deep-400 text-[10px] uppercase tracking-wider">Authors <span class="text-deep-400">(comma-separated)</span></span>
            <input bind:value={draft.authors} class="input w-full text-xs" placeholder="Jane Doe, DevCo" />
          </label>
          <div class="space-y-1">
            <div class="flex items-center justify-between">
              <span class="text-deep-400 text-[10px] uppercase tracking-wider">Subpacks</span>
              <button onclick={addSubpack}
                      class="btn-ghost text-[10px] px-2 py-0.5 rounded border border-deep-600/30 flex items-center gap-1">
                + Add
              </button>
            </div>
            {#if draft.subpacks.length === 0}
              <p class="text-[10px] text-deep-400">None — resolution tiers like 128x/64x live here.</p>
            {/if}
            {#each draft.subpacks as sub, i}
              <div class="flex gap-1.5 items-center">
                <input bind:value={sub.name} class="input flex-1 text-xs" placeholder="Name (128x)"
                       aria-label="Subpack {i + 1} name" />
                <input bind:value={sub.folder} class="input w-24 text-xs font-mono" placeholder="folder"
                       aria-label="Subpack {i + 1} folder name" />
                <input bind:value={sub.tier} type="number" min="0" class="input w-16 text-xs text-center" placeholder="tier"
                       aria-label="Subpack {i + 1} memory tier" />
                <button onclick={() => removeSubpack(i)}
                        class="btn-ghost text-xs px-1.5 py-1 rounded border border-red-500/30 text-red-400 shrink-0"
                        aria-label="Remove subpack {i + 1}">✕</button>
              </div>
            {/each}
          </div>
          <p class="text-[10px] text-deep-400">Visual edits rewrite the JSON document; other fields (modules, dependencies, capabilities) are preserved as-is.</p>
        </div>
      {:else}
        <textarea bind:value={raw} oninput={validateJson}
                  class="w-full h-80 bg-deep-950 text-xs font-mono p-3 border rounded resize-y
                         {valid ? 'border-deep-600/30' : 'border-red-500/50'}"
                  spellcheck="false"></textarea>
        {#if !valid}
          <p class="text-red-400 text-xs mt-1">Invalid JSON — cannot save</p>
        {/if}
      {/if}
      <div class="flex justify-end gap-2 mt-4">
        <button onclick={() => open = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={save} disabled={!valid}
                class="btn-primary text-xs px-4 py-1.5 disabled:opacity-40">Save</button>
      </div>
    </div>
  </div>
{/if}
