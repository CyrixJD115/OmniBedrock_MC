<script lang="ts">
  /** Errors/warnings per scanned log file, oldest → newest, with the
   * server/Endstone version each ran — answers "when did this start". */
  import type { CompatTimelineEntry } from '$types/compat';
  import { compact } from '$lib/compat/format';

  let { entries }: { entries: CompatTimelineEntry[] } = $props();

  const peak = $derived(Math.max(1, ...entries.map((e) => e.errors + e.warnings)));
  // First file whose error count jumps ≥10× the previous one.
  const jump = $derived.by(() => {
    for (let i = 1; i < entries.length; i++) {
      const prev = Math.max(entries[i - 1].errors, 1);
      if (entries[i].errors >= 1000 && entries[i].errors / prev >= 10) return entries[i].label;
    }
    return null;
  });
</script>

<div class="space-y-2">
  <div class="flex items-end gap-1.5 h-28" role="img" aria-label="Errors per log file">
    {#each entries as e (e.label)}
      <div class="flex-1 min-w-[10px] h-full flex flex-col justify-end group relative">
        <div class="bg-yellow-400/60" style="height: {(e.warnings / peak) * 100}%"></div>
        <div class="bg-red-400/70 rounded-t-sm {e.label === jump ? 'ring-2 ring-red-300' : ''}" style="height: {Math.max(e.errors ? 2 : 0, (e.errors / peak) * 100)}%"></div>
        <div class="pointer-events-none absolute bottom-full left-1/2 -translate-x-1/2 mb-1 hidden group-hover:block z-10 whitespace-nowrap bg-deep-900 border border-deep-600/40 px-2 py-1 text-[10px] rounded-sm shadow">
          <div class="text-strong">{e.label}</div>
          <div class="text-red-300">{e.errors.toLocaleString()} errors</div>
          <div class="text-yellow-300">{e.warnings.toLocaleString()} warnings</div>
          <div class="text-deep-400">{e.server_version ?? '?'}{e.endstone_version ? ` · Endstone ${e.endstone_version}` : ''}</div>
        </div>
      </div>
    {/each}
  </div>
  <div class="flex gap-1.5 text-[9px] text-deep-400 font-mono">
    {#each entries as e (e.label)}
      <div class="flex-1 min-w-[10px] truncate text-center" title={e.label}>
        {e.label.startsWith('latest') ? 'now' : e.label.slice(5)}
        {#if e.version_changed}<span class="block text-bedrock-300">▲ ver</span>{/if}
      </div>
    {/each}
  </div>
  {#if jump}
    {@const j = entries.find((e) => e.label === jump)}
    <p class="text-[11px] text-red-300">
      Errors jumped at <b>{jump}</b> ({compact(j?.errors ?? 0)}){j && !j.version_changed
        ? ' with no server or Endstone version change. Look at what changed in packs or the world around then.'
        : ', the same file where the server version changed.'}
    </p>
  {/if}
</div>
