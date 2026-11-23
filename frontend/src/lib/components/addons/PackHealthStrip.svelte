<script lang="ts">
  /**
   * One-line health summary for the selected world's packs, from the
   * compatibility checker (cached server-side, so this is cheap after the
   * first scan). Publishes per-pack status for the Active list's dots.
   */
  import { api } from '$lib/api/client';
  import type { CompatReport } from '$types/compat';
  import { compact } from '$lib/compat/format';
  import { ShieldAlert, ShieldCheck, Loader2, ChevronRight } from '@lucide/svelte';

  let { world, instanceId, health = $bindable({}), onopen }: {
    world: string;
    instanceId: string;
    health?: Record<string, 'error' | 'warning'>;
    onopen: () => void;
  } = $props();

  let report = $state<CompatReport | null>(null);
  let loading = $state(false);

  let epoch = 0;
  $effect(() => {
    instanceId;
    const target = world;
    const mine = ++epoch;
    report = null;
    health = {};
    if (!target) return;
    loading = true;
    api.getCompat(target)
      .then((r) => {
        if (mine !== epoch) return;
        report = r;
        const map: Record<string, 'error' | 'warning'> = {};
        for (const p of r.packs) {
          if (p.status === 'error' || p.status === 'warning') map[`${p.pack_type}/${p.folder}`] = p.status;
        }
        health = map;
      })
      .catch(() => undefined)
      .finally(() => { if (mine === epoch) loading = false; });
  });

  const bad = $derived(report?.packs.filter((p) => p.status === 'error') ?? []);
  const top = $derived(report?.issues.find((i) => i.pack && (i.severity === 'critical' || i.severity === 'error')) ?? null);
</script>

{#if loading && !report}
  <div class="card !py-2.5 flex items-center gap-2 text-xs text-deep-400"><Loader2 size={13} class="animate-spin" /> Checking pack health…</div>
{:else if report}
  <button onclick={onopen}
          class="card !py-2.5 w-full flex flex-wrap items-center gap-x-3 gap-y-1 text-left text-xs hover:border-bedrock-500/40 transition-colors
                 {report.summary.critical ? 'border-l-4 border-l-red-400' : bad.length ? 'border-l-4 border-l-yellow-400' : ''}">
    {#if report.summary.critical || bad.length}
      <ShieldAlert size={14} class={report.summary.critical ? 'text-red-400' : 'text-yellow-400'} />
      <span class="text-strong font-semibold">{bad.length} pack{bad.length === 1 ? '' : 's'} with errors</span>
      {#if report.summary.critical}<span class="badge-stopped !text-[9px] !py-0.5">{report.summary.critical} critical</span>{/if}
      {#if report.summary.events}<span class="text-deep-400">{compact(report.summary.events)} log events</span>{/if}
      {#if top}<span class="text-deep-300 truncate min-w-0 flex-1">Top: <b class="text-strong">{top.pack?.name}</b>: {top.title}</span>{/if}
    {:else}
      <ShieldCheck size={14} class="text-teal-400" />
      <span class="text-teal-300">No pack errors found in the server logs or manifests</span>
      <span class="flex-1"></span>
    {/if}
    <span class="flex items-center gap-1 text-bedrock-300 shrink-0">Compatibility <ChevronRight size={12} /></span>
  </button>
{/if}
