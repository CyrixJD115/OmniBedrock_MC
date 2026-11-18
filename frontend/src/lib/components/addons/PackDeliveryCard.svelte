<script lang="ts">
  /**
   * "What joining players download" for one world: the active resource
   * packs, their sizes, and what makes every join heavier than it needs to
   * be. Clients cache server packs by uuid+version and the panel never bumps
   * those on start, so repeat downloads mean either a changed version or a
   * client that cannot keep the stack — console clients have a small,
   * platform-managed pack cache.
   */
  import { api } from '$lib/api/client';
  import type { PackDelivery } from '$types/index';
  import { Download, ChevronDown, ChevronRight, TriangleAlert } from '@lucide/svelte';

  let { world }: { world: string } = $props();

  let report = $state<PackDelivery | null>(null);
  let open = $state(false);
  let error = $state('');

  function mb(bytes: number): string {
    return bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  $effect(() => {
    const target = world;
    report = null;
    error = '';
    if (!target) return;
    api.getPackDelivery(target)
      .then((r) => { if (target === world) report = r; })
      .catch((e: any) => { if (target === world) error = e.message; });
  });

  const unpaired = $derived(report?.packs.filter((p) => p.unpaired) ?? []);
  const mismatched = $derived(report?.packs.filter((p) => p.version_mismatch) ?? []);
  const heavy = $derived(report?.console_heavy ?? false);
</script>

{#if report && report.packs.length > 0}
  <div class="card !py-3">
    <button class="w-full flex items-center justify-between gap-3 text-left" onclick={() => (open = !open)}>
      <span class="flex items-center gap-2 text-xs">
        <Download size={14} class="text-bedrock-400" />
        <span class="uppercase tracking-wider text-strong font-semibold">Client download</span>
        <span class="font-mono text-deep-300">{mb(report.total_bytes)} · {report.packs.length} resource pack{report.packs.length === 1 ? '' : 's'}</span>
        {#if heavy}<span class="badge-warn !text-[9px] !py-0.5">heavy for consoles</span>{/if}
        {#if unpaired.length}<span class="badge-info !text-[9px] !py-0.5">{unpaired.length} unpaired</span>{/if}
        {#if mismatched.length}<span class="badge-stopped !text-[9px] !py-0.5">{mismatched.length} version pin</span>{/if}
      </span>
      {#if open}<ChevronDown size={14} class="text-deep-400" />{:else}<ChevronRight size={14} class="text-deep-400" />{/if}
    </button>
    {#if open}
      <div class="mt-3 space-y-3 text-xs">
        {#if heavy}
          <p class="flex items-start gap-2 text-yellow-300/90 leading-snug">
            <TriangleAlert size={13} class="shrink-0 mt-0.5" />
            <span>
              Every new client downloads about {mb(report.total_bytes)}. PC and mobile keep server packs cached by
              version, but Xbox, PlayStation, and Switch hold them in a small platform cache, so a stack this size is
              usually evicted and downloaded again on the next join. The panel never changes pack versions on start.
              To make console joins lighter, remove unused packs (especially <b>unpaired</b> ones below) or ship
              smaller texture variants.
            </span>
          </p>
        {/if}
        <ul class="mc-slot divide-y divide-deep-600/20">
          {#each report.packs as pack (pack.uuid)}
            <li class="flex items-center justify-between gap-3 px-3 py-1.5">
              <span class="min-w-0 truncate">
                <span class="text-strong">{pack.name}</span>
                <span class="font-mono text-[10px] text-deep-400"> {pack.folder} · v{pack.version.join('.')}</span>
                {#if pack.unpaired}<span class="text-[10px] text-bedrock-300" title="No active behavior pack depends on this resource pack; every join still downloads it"> · unpaired</span>{/if}
                {#if pack.subpacks}<span class="text-[10px] text-deep-400"> · subpacks</span>{/if}
                {#if pack.version_mismatch}<span class="text-[10px] text-red-300" title="world_resource_packs.json pins a different version than the manifest"> · pinned version differs</span>{/if}
              </span>
              <span class="font-mono text-deep-300 shrink-0">{mb(pack.bytes)}</span>
            </li>
          {/each}
        </ul>
        {#if report.missing.length}
          <p class="text-[10px] text-red-300">Referenced but missing on disk: <span class="font-mono">{report.missing.join(', ')}</span></p>
        {/if}
        {#if report.texturepack_required === false}
          <p class="text-[10px] text-deep-400">texturepack-required=false — players may decline the download and join without the packs.</p>
        {/if}
      </div>
    {/if}
  </div>
{:else if error}
  <!-- Diagnostics are optional; stay silent on failure. -->
{/if}
