<!-- Rename modal: set a new folder/display name for an addon. The page
     persists via onsave. -->
<script lang="ts">
  let {
    open = $bindable(false),
    value = $bindable(''),
    onsave,
  }: {
    open?: boolean;
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
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-md shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-3">Rename Addon</h2>
      <label for="rename-input" class="block text-deep-400 text-xs uppercase tracking-wider mb-1">New Name</label>
      <input id="rename-input" bind:value={value} class="input w-full text-xs py-1.5 mb-4" placeholder="Addon folder name" />
      <div class="flex justify-end gap-2">
        <button onclick={() => open = false} class="btn-secondary text-xs">Cancel</button>
        <button onclick={onsave} class="btn-primary text-xs px-4 py-1.5">Rename</button>
      </div>
    </div>
  </div>
{/if}
