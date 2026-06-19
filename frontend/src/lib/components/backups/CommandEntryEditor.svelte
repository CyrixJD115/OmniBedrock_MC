<script lang="ts">
  let { entry, onchange, ondelete }: {
    entry: { type: string; value: string | number };
    onchange: (e: { type: string; value: string | number }) => void;
    ondelete: () => void;
  } = $props();

  let typeOptions = ['command', 'wait', 'comment', 'send'];

  function handleTypeChange(t: string) {
    onchange({ type: t, value: t === 'wait' ? 5 : '' });
  }

  function handleValueChange(v: string | number) {
    onchange({ ...entry, value: v });
  }
</script>

<div class="flex items-start gap-2 p-2 bg-deep-800/50 rounded border border-deep-600/30">
  <div class="flex flex-col gap-1 shrink-0">
    <select
      value={entry.type}
      onchange={(e) => handleTypeChange((e.target as HTMLSelectElement).value)}
      class="bg-deep-900 text-deep-200 text-xs border border-deep-600/40 rounded px-1 py-1 w-20"
    >
      {#each typeOptions as opt}
        <option value={opt} selected={entry.type === opt}>{opt}</option>
      {/each}
    </select>
  </div>
  <div class="flex-1 min-w-0">
    {#if entry.type === 'command'}
      <textarea
        value={String(entry.value)}
        oninput={(e) => handleValueChange((e.target as HTMLTextAreaElement).value)}
        class="w-full bg-deep-900 text-deep-100 text-xs border border-deep-600/40 rounded px-2 py-1 font-mono"
        rows="2"
        placeholder="Shell command..."
      ></textarea>
    {:else if entry.type === 'wait'}
      <div class="flex items-center gap-2">
        <input
          type="number"
          value={Number(entry.value) || 5}
          oninput={(e) => handleValueChange(parseInt((e.target as HTMLInputElement).value) || 5)}
          min="1" max="600"
          class="w-20 bg-deep-900 text-deep-100 text-xs border border-deep-600/40 rounded px-2 py-1"
        />
        <span class="text-deep-400 text-xs">seconds</span>
      </div>
    {:else}
      <input
        type="text"
        value={String(entry.value)}
        oninput={(e) => handleValueChange((e.target as HTMLInputElement).value)}
        class="w-full bg-deep-900 text-deep-100 text-xs border border-deep-600/40 rounded px-2 py-1"
        placeholder={entry.type === 'send' ? '/say Backup starting...' : 'Comment text...'}
      />
    {/if}
  </div>
  <button onclick={ondelete} class="text-deep-400 hover:text-red-400 transition shrink-0 text-xs px-1 py-1">&times;</button>
</div>
