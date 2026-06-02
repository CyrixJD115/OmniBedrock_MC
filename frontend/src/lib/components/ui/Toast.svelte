<script lang="ts">
  let { message, type = 'info', duration = 4000 }: { message: string; type?: 'info' | 'success' | 'error'; duration?: number } = $props();
  let visible = $state(true);

  $effect(() => {
    const timer = setTimeout(() => visible = false, duration);
    return () => clearTimeout(timer);
  });

  let toastClass = $derived(`fixed bottom-4 right-4 z-50 animate-in slide-in-from-right-4 fade-in duration-200 px-4 py-3 rounded-lg shadow-lg text-white text-sm max-w-sm ${
    type === 'success' ? 'bg-bedrock-600' : type === 'error' ? 'bg-red-600' : 'bg-surface-700'
  }`);
</script>

{#if visible}
  <div class={toastClass}>
    {message}
  </div>
{/if}
