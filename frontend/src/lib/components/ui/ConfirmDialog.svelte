<script lang="ts">
  let { open = false, title = 'Confirm', message = '', confirmText = 'Confirm', cancelText = 'Cancel',
        variant = 'danger', onconfirm, oncancel }: {
    open: boolean; title?: string; message?: string; confirmText?: string; cancelText?: string;
    variant?: 'danger' | 'primary' | 'warning'; onconfirm: () => void; oncancel?: () => void;
  } = $props();

  function handleBgClick(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('modal-bg')) {
      oncancel?.();
    }
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape') oncancel?.();
  }
</script>

<svelte:window onkeydown={handleKeyDown} />

{#if open}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-bg"
       style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
       onclick={handleBgClick}
       onkeydown={(e) => { if (e.key === 'Escape') handleKeyDown(e); }}
       role="dialog" aria-modal="true" tabindex="-1">
    <div class="bg-deep-900 border-2 border-deep-600/50 p-6 w-full max-w-sm shadow-block-lg shadow-black/50"
         style="box-shadow: inset 2px 2px 0 rgba(255,255,255,0.03), inset -1px -1px 0 rgba(0,0,0,0.3), 6px 6px 0 rgba(0,0,0,0.5);">
      <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-3">{title}</h2>

      {#if message}
        <p class="text-xs text-deep-300 mb-5 leading-relaxed">{message}</p>
      {/if}

      <div class="flex gap-2 justify-end">
        <button onclick={oncancel} class="btn-secondary text-xs">{cancelText}</button>
        <button onclick={onconfirm}
                class="btn-ghost text-xs px-4 py-2 rounded uppercase tracking-wider font-semibold
                       {variant === 'danger' ? 'text-red-400 hover:bg-red-500/10 border-red-500/30' :
                        variant === 'warning' ? 'text-yellow-400 hover:bg-yellow-500/10 border-yellow-500/30' :
                        'text-bedrock-400 hover:bg-bedrock-500/10 border-bedrock-500/30'}
                       border">
          {confirmText}
        </button>
      </div>
    </div>
  </div>
{/if}
