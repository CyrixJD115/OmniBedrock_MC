<!-- Version editor modal: quick bump buttons plus explicit major/minor/patch
     fields. The page persists via onsave. -->
<script lang="ts">
  let {
    open = $bindable(false),
    name,
    major = $bindable(0),
    minor = $bindable(0),
    patch = $bindable(0),
    onsave,
  }: {
    open?: boolean;
    name: string;
    major?: number;
    minor?: number;
    patch?: number;
    onsave: () => void;
  } = $props();

  function bumpVersion(part: 'major' | 'minor' | 'patch') {
    if (part === 'major') { major++; minor = 0; patch = 0; }
    else if (part === 'minor') { minor++; patch = 0; }
    else if (part === 'patch') { patch++; }
  }
</script>

{#if open}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) open = false; }}
       onkeydown={(e) => e.key === 'Escape' && (open = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-1">Version Editor</h2>
      <p class="text-xs text-deep-400 mb-4 truncate">{name}</p>

      <div class="flex items-center gap-2 mb-4">
        <span class="text-deep-400 text-xs">Quick:</span>
        <button onclick={() => bumpVersion('major')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Major</button>
        <button onclick={() => bumpVersion('minor')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Minor</button>
        <button onclick={() => bumpVersion('patch')} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30">Bump Patch</button>
      </div>

      <div class="grid grid-cols-3 gap-3 mb-4">
        <div>
          <label for="version-major" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Major</label>
          <input id="version-major" type="number" min="0" max="9999" bind:value={major} class="input w-full text-xs py-1.5 text-center" />
        </div>
        <div>
          <label for="version-minor" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Minor</label>
          <input id="version-minor" type="number" min="0" max="9999" bind:value={minor} class="input w-full text-xs py-1.5 text-center" />
        </div>
        <div>
          <label for="version-patch" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">Patch</label>
          <input id="version-patch" type="number" min="0" max="9999" bind:value={patch} class="input w-full text-xs py-1.5 text-center" />
        </div>
      </div>

      <p class="text-center text-base font-mono text-bedrock-400 mb-4">{major}.{minor}.{patch}</p>

      <div class="flex justify-end gap-2">
        <button onclick={() => open = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={onsave} class="btn-primary text-xs px-4 py-1.5">Save</button>
      </div>
    </div>
  </div>
{/if}
