<script lang="ts">
  /** One server.properties entry: input matched to its type, plus help text. */
  import type { PropSpec } from '$lib/properties/schema';
  import { ChevronDown, ChevronRight } from '@lucide/svelte';

  let { name, value, spec, description, hint, canEdit, onchange }: {
    name: string;
    value: string;
    spec: PropSpec;
    description: string;
    hint: string | null;
    canEdit: boolean;
    onchange: (value: string) => void;
  } = $props();

  let open = $state(false);
  let invalid = $state('');

  function commitNumber(raw: string) {
    const v = raw.trim();
    const num = Number(v);
    if (v === '' || Number.isNaN(num)) { invalid = 'Enter a number'; return; }
    if (spec.min !== undefined && num < spec.min) { invalid = `Minimum ${spec.min}`; return; }
    if (spec.max !== undefined && spec.max !== null && num > spec.max) { invalid = `Maximum ${spec.max}`; return; }
    invalid = '';
    onchange(v);
  }
</script>

<div class="grid grid-cols-1 md:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] gap-x-4 gap-y-1 px-3 py-2.5 border-b border-deep-700/20 hover:bg-deep-800/20">
  <div class="min-w-0">
    <p class="font-mono text-xs text-bedrock-300 break-all">{name}</p>
    {#if spec.summary}<p class="text-[11px] text-deep-300 leading-snug">{spec.summary}</p>{/if}
    {#if description}
      <button class="mt-0.5 flex items-center gap-1 text-[10px] text-deep-400 hover:text-deep-200" onclick={() => (open = !open)} aria-expanded={open}>
        {#if open}<ChevronDown size={10} />{:else}<ChevronRight size={10} />{/if} Details
      </button>
      {#if open}<p class="mt-1 text-[11px] text-deep-400 leading-relaxed">{description}</p>{/if}
    {/if}
  </div>
  <div class="min-w-0 self-center">
    {#if !canEdit}
      <span class="font-mono text-xs text-strong">{value}</span>
    {:else if spec.kind === 'bool'}
      <button role="switch" aria-checked={value === 'true'} aria-label={name}
              onclick={() => onchange(value === 'true' ? 'false' : 'true')}
              class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors {value === 'true' ? 'bg-bedrock-500' : 'bg-deep-600'}">
        <span class="inline-block h-4 w-4 rounded-full bg-white shadow transition-transform {value === 'true' ? 'translate-x-4' : 'translate-x-0.5'}"></span>
      </button>
      <span class="ml-2 text-xs font-mono {value === 'true' ? 'text-teal-300' : 'text-deep-400'}">{value}</span>
    {:else if spec.kind === 'enum'}
      <select value={value} onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)} class="input w-full max-w-xs text-xs py-1">
        {#if !spec.options?.includes(value)}<option value={value}>{value} (current)</option>{/if}
        {#each spec.options ?? [] as o (o)}<option value={o}>{o}</option>{/each}
      </select>
    {:else if spec.kind === 'number'}
      <div class="flex items-center gap-2">
        <input type="number" value={value} min={spec.min} max={spec.max ?? undefined}
               onchange={(e) => commitNumber((e.currentTarget as HTMLInputElement).value)}
               class="input w-36 text-xs py-1 font-mono {invalid ? '!border-red-500/70' : ''}" aria-invalid={!!invalid} />
        <span class="text-[10px] text-deep-400">{spec.min ?? ''}{spec.max !== undefined ? `–${spec.max ?? '∞'}` : ''}</span>
      </div>
    {:else}
      <input type="text" value={value} onchange={(e) => onchange((e.currentTarget as HTMLInputElement).value)}
             class="input w-full text-xs py-1" />
    {/if}
    {#if invalid}<p class="text-[10px] text-red-300 mt-0.5">{invalid}</p>{/if}
    {#if hint}<p class="text-[10px] text-deep-400 mt-0.5">{hint}</p>{/if}
  </div>
</div>
