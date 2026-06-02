<script lang="ts">
  import { toasts, dismissToast, type Toast } from '$stores/toast';
  import { X } from '@lucide/svelte';

  let visibleToasts = $state<Toast[]>([]);
  toasts.subscribe(t => { visibleToasts = t; });
</script>

<div class="fixed bottom-4 right-4 z-[9999] flex flex-col gap-2 max-w-sm pointer-events-none">
  {#each visibleToasts as toast (toast.id)}
    <div class="pointer-events-auto flex items-start gap-3 px-4 py-3 text-white text-sm shadow-block shadow-black/60 animate-slide-up border-l-4"
         class:bg-teal-700={toast.type === 'success'}
         class:bg-red-700={toast.type === 'error'}
         class:bg-deep-700={toast.type === 'info'}
         class:border-teal-400={toast.type === 'success'}
         class:border-red-400={toast.type === 'error'}
         class:border-bedrock-400={toast.type === 'info'}>
      <span class="flex-1">{toast.message}</span>
      {#if toast.action}
        <button onclick={() => { toast.action!.callback(); dismissToast(toast.id); }}
                class="px-2 py-0.5 text-xs uppercase tracking-wider font-semibold
                       hover:bg-white/20 rounded border border-white/30">
          {toast.action.label}
        </button>
      {/if}
      <button onclick={() => dismissToast(toast.id)}
              class="p-0.5 hover:bg-white/20 rounded shrink-0">
        <X size={14} />
      </button>
    </div>
  {/each}
</div>
