<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { consoleLines } from '$stores/index';
  import { api } from '$lib/api/client';
  import { Terminal, Send, Trash2, Filter } from '@lucide/svelte';

  let input = $state('');
  let autoScroll = $state(true);
  let filterText = $state('');
  let terminalEl: HTMLDivElement;
  let lines: { text: string; level: string; timestamp: number }[] = $state([]);

  $effect(() => {
    $consoleLines;
    lines = [...$consoleLines];
    if (autoScroll && terminalEl) {
      requestAnimationFrame(() => {
        terminalEl.scrollTop = terminalEl.scrollHeight;
      });
    }
  });

  function getLevelClass(level: string): string {
    return `terminal-line-${level}`;
  }

  async function sendCommand() {
    if (!input.trim()) return;
    lines.push({ text: `> ${input}`, level: 'local', timestamp: Date.now() });
    await api.sendCommand(input.trim());
    input = '';
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') sendCommand();
  }

  function clearConsole() {
    consoleLines.set([]);
    lines = [];
  }

  let filteredLines = $derived.by(() => {
    if (!filterText) return lines;
    return lines.filter(l => l.text.toLowerCase().includes(filterText.toLowerCase()));
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Live Console</h1>
    <div class="flex items-center gap-2">
      <div class="relative">
        <Filter size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-surface-500" />
        <input type="text" bind:value={filterText} placeholder="Filter..."
               class="input pl-8 py-1.5 text-sm w-48" />
      </div>
      <button onclick={clearConsole} class="btn-ghost p-2" title="Clear console">
        <Trash2 size={16} />
      </button>
      <label class="flex items-center gap-2 text-sm text-surface-400 cursor-pointer">
        <input type="checkbox" bind:checked={autoScroll} class="accent-bedrock-500" />
        Auto-scroll
      </label>
    </div>
  </div>

  <div class="card p-0 overflow-hidden">
    <div bind:this={terminalEl}
         class="h-[60vh] overflow-y-auto p-4 font-mono text-sm leading-relaxed bg-black/50">
      {#each filteredLines as line, i (i)}
        <div class={getLevelClass(line.level)}>
          {line.text}
        </div>
      {/each}
      {#if filteredLines.length === 0}
        <div class="text-surface-600 italic">
          {#if filterText}
            No matching output
          {:else}
            Console output will appear here when the server is running
          {/if}
        </div>
      {/if}
    </div>
  </div>

  <div class="flex gap-2">
    <input type="text" bind:value={input} onkeydown={onKeyDown}
           placeholder="Enter a command..."
           class="input flex-1 font-mono" />
    <button onclick={sendCommand} class="btn-primary flex items-center gap-2">
      <Send size={16} /> Send
    </button>
  </div>
</div>
