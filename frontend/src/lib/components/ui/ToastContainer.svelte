<script lang="ts">
  import { toasts, dismissToast, type Toast } from '$stores/toast';
  import { X } from '@lucide/svelte';

  let visibleToasts = $state<Toast[]>([]);
  toasts.subscribe(t => { visibleToasts = t; });
</script>

<div class="fixed bottom-4 right-4 z-[9999] flex flex-col gap-2 max-w-sm pointer-events-none">
  {#each visibleToasts as toast (toast.id)}
    <!-- Success/error toasts sit on solid dark status colors (white text in
         both modes); info toasts follow the themed surface ramp instead. -->
    {#if toast.type === 'info'}
      <div class="pointer-events-auto flex items-start gap-3 px-4 py-3 bg-deep-700 text-strong text-sm rounded-sm border-l-4 border-bedrock-400 shadow-block animate-slide-up">
        <span class="flex-1">{toast.message}</span>
        {#if toast.action}
          <button onclick={() => { toast.action!.callback(); dismissToast(toast.id); }}
                  class="px-2 py-0.5 text-xs uppercase tracking-wider font-semibold
                         hover:bg-black/10 rounded-sm border border-deep-500/60">
            {toast.action.label}
          </button>
        {/if}
        <button onclick={() => dismissToast(toast.id)}
                class="p-0.5 hover:bg-black/10 rounded-sm shrink-0">
          <X size={14} />
        </button>
      </div>
    {:else}
      <div class={"pointer-events-auto flex items-start gap-3 px-4 py-3 text-white text-sm rounded-sm border-l-4 shadow-block animate-slide-up " +
                  (toast.type === 'success' ? 'bg-teal-700 border-teal-400' : 'bg-red-700 border-red-400')}>
        <span class="flex-1">{toast.message}</span>
        {#if toast.action}
          <button onclick={() => { toast.action!.callback(); dismissToast(toast.id); }}
                  class="px-2 py-0.5 text-xs uppercase tracking-wider font-semibold
                         hover:bg-white/20 rounded-sm border border-white/40">
            {toast.action.label}
          </button>
        {/if}
        <button onclick={() => dismissToast(toast.id)}
                class="p-0.5 hover:bg-white/20 rounded-sm shrink-0">
          <X size={14} />
        </button>
      </div>
    {/if}
  {/each}
</div>
