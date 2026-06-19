<script lang="ts">
  import CommandEntryEditor from './CommandEntryEditor.svelte';
  import type { CommandEntry } from '$types/index';

  let { label = 'Pre/Post Commands', before = [], after = [], onchange }: {
    label?: string;
    before: CommandEntry[];
    after: CommandEntry[];
    onchange: (b: CommandEntry[], a: CommandEntry[]) => void;
  } = $props();

  let activeTab = $state<'before' | 'after'>('before');
  let addingTo = $state<'before' | 'after'>('before');

  function addEntry() {
    const entry: CommandEntry = { type: 'send', value: '' };
    if (activeTab === 'before') {
      onchange([...before, entry], after);
    } else {
      onchange(before, [...after, entry]);
    }
  }

  function updateEntry(index: number, entry: CommandEntry) {
    if (activeTab === 'before') {
      const copy = [...before];
      copy[index] = entry;
      onchange(copy, after);
    } else {
      const copy = [...after];
      copy[index] = entry;
      onchange(before, copy);
    }
  }

  function deleteEntry(index: number) {
    if (activeTab === 'before') {
      const copy = before.filter((_, i) => i !== index);
      onchange(copy, after);
    } else {
      const copy = after.filter((_, i) => i !== index);
      onchange(before, copy);
    }
  }

  function moveEntry(index: number, dir: -1 | 1) {
    const arr = activeTab === 'before' ? [...before] : [...after];
    const target = index + dir;
    if (target < 0 || target >= arr.length) return;
    [arr[index], arr[target]] = [arr[target], arr[index]];
    if (activeTab === 'before') {
      onchange(arr, after);
    } else {
      onchange(before, arr);
    }
  }
</script>

<div class="bg-deep-800/40 border border-deep-600/30 rounded p-3">
  <div class="flex items-center justify-between mb-3">
    <span class="text-xs font-semibold text-deep-200 uppercase tracking-wider">{label}</span>
  </div>

  <div class="flex gap-1 mb-3">
    <button
      onclick={() => { activeTab = 'before'; addingTo = 'before'; }}
      class="text-xs px-3 py-1 rounded uppercase tracking-wider font-semibold transition
             {activeTab === 'before' ? 'bg-bedrock-600 text-white' : 'text-deep-300 hover:text-deep-100 bg-deep-800/60'}"
    >Before</button>
    <button
      onclick={() => { activeTab = 'after'; addingTo = 'after'; }}
      class="text-xs px-3 py-1 rounded uppercase tracking-wider font-semibold transition
             {activeTab === 'after' ? 'bg-bedrock-600 text-white' : 'text-deep-300 hover:text-deep-100 bg-deep-800/60'}"
    >After</button>
  </div>

  <div class="space-y-2 mb-3">
    {#each (activeTab === 'before' ? before : after) as entry, i}
      <div class="flex items-center gap-1">
        <div class="flex flex-col gap-0.5 shrink-0">
          <button onclick={() => moveEntry(i, -1)} disabled={i === 0}
                  class="text-deep-400 hover:text-deep-200 disabled:opacity-30 text-xs leading-none px-1">&uarr;</button>
          <button onclick={() => moveEntry(i, 1)} disabled={i === (activeTab === 'before' ? before : after).length - 1}
                  class="text-deep-400 hover:text-deep-200 disabled:opacity-30 text-xs leading-none px-1">&darr;</button>
        </div>
        <div class="flex-1">
          <CommandEntryEditor
            entry={entry}
            onchange={(e) => updateEntry(i, e)}
            ondelete={() => deleteEntry(i)}
          />
        </div>
      </div>
    {:else}
      <p class="text-deep-500 text-xs italic">No entries in this phase.</p>
    {/each}
  </div>

  <button onclick={addEntry} class="btn-ghost text-xs px-3 py-1 rounded uppercase tracking-wider border border-deep-600/30 text-bedrock-400 hover:bg-bedrock-500/10">
    + Add Entry
  </button>
</div>
