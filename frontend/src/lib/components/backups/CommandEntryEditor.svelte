<script lang="ts">
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import type { CommandEntry } from '$types/index';

  let { entry, onchange, ondelete }: {
    entry: CommandEntry;
    onchange: (e: CommandEntry) => void;
    ondelete: () => void;
  } = $props();

  let typeOptions: CommandEntry['type'][] = ['command', 'wait', 'comment', 'send'];

  let testing = $state(false);
  let testResult = $state<{ kind: string; output: string; exit_code: number } | null>(null);

  function handleTypeChange(t: string) {
    onchange({ type: t as CommandEntry['type'], value: t === 'wait' ? 5 : '' });
  }

  function handleValueChange(v: string | number) {
    onchange({ type: entry.type, value: v });
  }

  async function testEntry() {
    testing = true;
    testResult = null;
    try {
      testResult = await api.testCommand(entry);
    } catch (e: any) {
      addToast(`Test failed: ${e.message}`, 'error');
    }
    testing = false;
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
        placeholder={entry.type === 'send' ? 'save hold' : 'Comment text...'}
      />
    {/if}
    {#if testResult}
      <div class="mt-1.5 p-1.5 rounded bg-deep-900/80 border border-deep-600/30 font-mono text-xs max-h-24 overflow-y-auto">
        <div class="flex items-center gap-2 mb-0.5">
          <span class="text-deep-500 uppercase tracking-wider text-[10px]">exit {testResult.exit_code}</span>
          {#if testResult.exit_code === 0}
            <span class="text-green-500 text-[10px]">OK</span>
          {:else}
            <span class="text-red-400 text-[10px]">FAIL</span>
          {/if}
          <button onclick={() => testResult = null} class="ml-auto text-deep-500 hover:text-deep-300 text-[10px]">&times;</button>
        </div>
        <pre class="text-deep-200 whitespace-pre-wrap break-all">{testResult.output}</pre>
      </div>
    {/if}
  </div>
  <div class="flex flex-col gap-1 shrink-0">
    <button
      onclick={testEntry}
      disabled={testing}
      class="text-bedrock-400 hover:text-bedrock-300 transition text-xs px-1 py-0.5 disabled:opacity-30"
      title="Test this command"
    >{testing ? '...' : 'Test'}</button>
    <button onclick={ondelete} class="text-deep-400 hover:text-red-400 transition text-xs px-1 py-0.5">&times;</button>
  </div>
</div>
