<script lang="ts">
  /** Which installed packs would break on the selected game version. */
  import { api } from '$lib/api/client';
  import type { VersionPreview } from '$types/compat';
  import { CircleCheck, CircleX, TriangleAlert, Loader2 } from '@lucide/svelte';

  let { version, installed }: { version: string; installed: string | null } = $props();

  let preview = $state<VersionPreview | null>(null);
  let loading = $state(false);
  let error = $state('');
  let showAll = $state(false);

  let epoch = 0;
  $effect(() => {
    const target = version;
    const mine = ++epoch;
    preview = null;
    error = '';
    showAll = false;
    if (!target || target === 'latest' || target === installed) return;
    loading = true;
    const t = setTimeout(() => {
      api.compatPreview(target)
        .then((p) => { if (mine === epoch) preview = p; })
        .catch((e: any) => { if (mine === epoch) error = e.message.replace(/^API Error \d+: /, ''); })
        .finally(() => { if (mine === epoch) loading = false; });
    }, 250);
    return () => clearTimeout(t);
  });

  const fresh = $derived(preview?.issues.filter((i) => i.new) ?? []);
  const shown = $derived(showAll ? preview?.issues ?? [] : fresh);
</script>

{#if version && version !== 'latest' && version !== installed}
  <div class="mt-3 border border-deep-600/40 bg-deep-900/30 p-3 text-xs space-y-2">
    <p class="text-[10px] uppercase tracking-wider text-deep-400 flex items-center gap-1.5">
      Pack compatibility on {version}
      {#if loading}<Loader2 size={11} class="animate-spin" />{/if}
    </p>
    {#if error}
      <p class="text-deep-400">Preview unavailable: {error}</p>
    {:else if preview}
      {#if preview.blocking > 0}
        <p class="flex items-center gap-1.5 text-red-300"><CircleX size={13} /> {preview.blocking} new problem(s) would stop packs from loading on {version}.</p>
      {:else if fresh.length > 0}
        <p class="flex items-center gap-1.5 text-yellow-300"><TriangleAlert size={13} /> {fresh.length} new warning(s); packs should still load.</p>
      {:else}
        <p class="flex items-center gap-1.5 text-teal-300"><CircleCheck size={13} /> No new pack problems detected for {version}.</p>
      {/if}
      {#if preview.resolved > 0}<p class="text-teal-300/80">{preview.resolved} current problem(s) would go away.</p>{/if}
      {#if preview.target_script_api}<p class="text-deep-400">Script API on {version}: @minecraft/server ≤ {preview.target_script_api}</p>{/if}
      {#if shown.length}
        <ul class="space-y-1 max-h-48 overflow-y-auto">
          {#each shown as i, k (k)}
            <li class="flex items-start gap-1.5">
              <span class="mt-0.5 w-1.5 h-1.5 shrink-0 rounded-full {i.severity === 'critical' || i.severity === 'error' ? 'bg-red-400' : 'bg-yellow-400'}"></span>
              <span><b class="text-strong">{i.pack || 'World'}</b>: {i.title}{#if !i.new}<span class="text-deep-400"> (already today)</span>{/if}</span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if preview.issues.length > fresh.length}
        <button class="btn-ghost text-[10px] px-2 py-0.5" onclick={() => (showAll = !showAll)}>{showAll ? 'Only new problems' : `Show all ${preview.issues.length}`}</button>
      {/if}
      <p class="text-[10px] text-deep-400">Static check of manifests and script modules. Run Mods → Compatibility after switching for the full log analysis.</p>
    {/if}
  </div>
{/if}
