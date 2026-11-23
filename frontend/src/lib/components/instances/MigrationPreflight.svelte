<script lang="ts">
  /** Live preflight for a deployment switch: blockers, warnings, and steps. */
  import { api } from '$lib/api/client';
  import type { MigrationPlan } from '$types/migration';
  import { CircleCheck, CircleAlert, CircleX, Info, Loader2, ArrowRight } from '@lucide/svelte';

  let { instanceId, from, to, ready = $bindable(true) }: {
    instanceId: string;
    from: string;
    to: string;
    ready?: boolean;
  } = $props();

  let plan = $state<MigrationPlan | null>(null);
  let loading = $state(false);
  let error = $state('');

  let epoch = 0;
  $effect(() => {
    const target = to;
    const mine = ++epoch;
    plan = null;
    error = '';
    if (!target || target === from) {
      ready = true;
      return;
    }
    ready = false;
    loading = true;
    api.migrationPlan(instanceId, target)
      .then((p) => { if (mine === epoch) { plan = p; ready = p.ready; } })
      .catch((e: any) => { if (mine === epoch) { error = e.message; ready = false; } })
      .finally(() => { if (mine === epoch) loading = false; });
  });
</script>

{#if to && to !== from}
  <div class="mt-2 border border-deep-600/40 bg-deep-900/30 p-2 space-y-2 text-[11px]">
    <p class="flex items-center gap-1.5 uppercase tracking-wider text-[10px] text-deep-400">
      Switch {from} <ArrowRight size={10} /> {to}
      {#if loading}<Loader2 size={10} class="animate-spin" />{/if}
    </p>
    {#if error}
      <p class="text-red-300">Preflight failed: {error}</p>
    {:else if plan}
      <ul class="space-y-1">
        {#each plan.checks as c (c.id)}
          <li class="flex items-start gap-1.5">
            {#if c.level === 'ok'}<CircleCheck size={12} class="text-teal-400 shrink-0 mt-px" />
            {:else if c.level === 'error'}<CircleX size={12} class="text-red-400 shrink-0 mt-px" />
            {:else if c.level === 'warning'}<CircleAlert size={12} class="text-yellow-400 shrink-0 mt-px" />
            {:else}<Info size={12} class="text-bedrock-300 shrink-0 mt-px" />{/if}
            <span class={c.level === 'error' ? 'text-red-200' : 'text-deep-200'}>{c.message}</span>
          </li>
        {/each}
      </ul>
      {#if plan.steps.length}
        <ol class="list-decimal pl-5 space-y-0.5 text-deep-300 border-t border-deep-600/30 pt-2">
          {#each plan.steps as step, i (i)}<li>{step}</li>{/each}
        </ol>
      {/if}
    {/if}
  </div>
{/if}
