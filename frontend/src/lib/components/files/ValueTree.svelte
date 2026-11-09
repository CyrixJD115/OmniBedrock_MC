<!-- Recursive collapsible value tree for the explorer's pretty view of
     JSON/YAML documents. Objects/arrays start expanded to `defaultDepth`
     levels, deeper levels start collapsed; primitives render inline. -->
<script lang="ts">
  import { ChevronDown, ChevronRight } from '@lucide/svelte';
  // Svelte 5 self-import: the tree recurses for nested values.
  import ValueTree from './ValueTree.svelte';

  let {
    label = '',
    value,
    depth = 0,
    defaultDepth = 1,
  }: {
    label?: string;
    value: unknown;
    depth?: number;
    defaultDepth?: number;
  } = $props();

  let open = $state(false);
  // Shallow levels start expanded; this runs once — the tree never re-keys.
  $effect(() => {
    if (depth < defaultDepth) open = true;
  });

  const isObject = $derived(
    value !== null && typeof value === 'object' && !(value instanceof Date)
  );
  const entries = $derived(
    isObject
      ? Array.isArray(value)
        ? value.map((v, i) => [String(i), v] as const)
        : Object.entries(value as Record<string, unknown>)
      : []
  );
  const scalarClass = $derived.by(() => {
    const v = value;
    if (typeof v === 'number') return 'text-teal-300';
    if (typeof v === 'boolean') return 'text-bedrock-300';
    if (v === null || v === undefined) return 'text-deep-500';
    return 'text-deep-100';
  });
  const scalarText = $derived(
    value === null || value === undefined
      ? 'null'
      : typeof value === 'string'
        ? JSON.stringify(value)
        : String(value)
  );
</script>

{#if isObject}
  <div class="leading-6">
    <button
      class="flex items-center gap-1 text-left hover:text-strong transition-colors w-full"
      onclick={() => (open = !open)}
      aria-expanded={open}
    >
      {#if open}<ChevronDown size={11} class="text-deep-400 shrink-0" />
      {:else}<ChevronRight size={11} class="text-deep-400 shrink-0" />{/if}
      {#if label}<span class="text-bedrock-300">{label}</span><span class="text-deep-500">:</span>{/if}
      <span class="text-deep-400 text-[10px] font-mono">
        {Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`}
      </span>
    </button>
    {#if open}
      <div class="pl-4 border-l border-deep-700/40 ml-2">
        {#each entries as [key, child] (key)}
          <ValueTree label={key} value={child} depth={depth + 1} {defaultDepth} />
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <div class="leading-6 flex items-baseline gap-1.5">
    <span class="w-[11px] shrink-0"></span>
    {#if label}<span class="text-bedrock-300">{label}</span><span class="text-deep-500">:</span>{/if}
    <span class={scalarClass + ' break-all'}>{scalarText}</span>
  </div>
{/if}
