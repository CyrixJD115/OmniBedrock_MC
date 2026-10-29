<script lang="ts">
  /**
   * Delete-instance confirmation modal with the keep-files-on-disk option,
   * two-way bound so the page's remove action sees the same flag.
   */
  import { Trash2 } from '@lucide/svelte';
  import type { Instance } from '$types/index';

  let { instance, keepData = $bindable(false), onremove, onclose }: {
    instance: Instance;
    keepData: boolean;
    onremove: () => void;
    onclose: () => void;
  } = $props();
</script>

<div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
     onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
     onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}>
  <div class="card max-w-sm w-full mx-4">
    <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-2">Delete {instance.name}?</h2>
    <p class="text-xs text-deep-300 mb-3">
      The instance is stopped and removed from the panel. This cannot be undone.
    </p>
    <label class="flex items-center gap-2 text-xs text-deep-300 mb-4">
      <input type="checkbox" bind:checked={keepData} />
      Keep files on disk (worlds, backups, logs stay under the data directory)
    </label>
    <div class="flex justify-end gap-2">
      <button onclick={onclose} class="btn-secondary text-xs">Cancel</button>
      <button onclick={onremove} class="btn-danger text-xs flex items-center gap-1.5"><Trash2 size={12} /> Delete</button>
    </div>
  </div>
</div>
