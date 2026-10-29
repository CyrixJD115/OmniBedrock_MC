<!-- UUID editor modal: change or randomize a pack's UUID. The page persists
     via onsave. -->
<script lang="ts">
  let {
    open = $bindable(false),
    mode,
    value = $bindable(''),
    onsave,
  }: {
    open?: boolean;
    mode: 'change' | 'randomize';
    value?: string;
    onsave: () => void;
  } = $props();
</script>

{#if open}
  <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       onclick={(e) => { if ((e.target as HTMLElement).classList.contains('modal-bg')) open = false; }}
       onkeydown={(e) => e.key === 'Escape' && (open = false)}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-lg shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-3">
        {mode === 'randomize' ? 'Randomize UUID' : 'Change UUID'}
      </h2>
      <label for="uuid-input" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">UUID</label>
      <input id="uuid-input" bind:value={value} class="input w-full text-xs py-1.5 font-mono mb-3" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
      {#if mode === 'randomize'}
        <button onclick={() => value = crypto.randomUUID()}
                class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 mb-3">Generate New</button>
      {/if}
      <div class="flex justify-end gap-2">
        <button onclick={() => open = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={onsave} class="btn-primary text-xs px-4 py-1.5">Save</button>
      </div>
    </div>
  </div>
{/if}
