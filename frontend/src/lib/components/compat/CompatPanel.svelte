<script lang="ts">
  /**
   * Add-on compatibility checker. The server reads its logs (latest + rotated),
   * classifies every warning/error into issues attributed to packs, checks
   * manifests and script-API versions against this server build, and scans
   * pack JavaScript for removed APIs. Read-only: nothing here changes packs.
   */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import type { CompatReport, CompatSeverity } from '$types/compat';
  import { SEVERITIES, SEVERITY_LABEL, SOURCE_LABEL, bytes, compact, severityBadge } from '$lib/compat/format';
  import IssueCard from './IssueCard.svelte';
  import CompatTimeline from './CompatTimeline.svelte';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import { Activity, FileDown, Loader2, Package, RotateCw, Search, ShieldCheck, Trash2, X } from '@lucide/svelte';

  let { world, canManage, instanceId }: { world?: string; canManage: boolean; instanceId: string } = $props();

  let report = $state<CompatReport | null>(null);
  let loading = $state(false);
  let error = $state('');
  let sevFilter = $state<Set<CompatSeverity>>(new Set(['critical', 'error', 'warning']));
  let sourceFilter = $state<string>('all');
  let packFilter = $state<string | null>(null);
  let query = $state('');
  let openIds = $state<Set<string>>(new Set());
  let confirmPrune = $state(false);
  let limit = $state(60);

  let epoch = 0;
  async function load(refresh = false) {
    const mine = ++epoch;
    loading = true;
    error = '';
    try {
      const r = await api.getCompat(world, refresh);
      if (mine !== epoch) return;
      report = r;
      // Expand the single worst issue so the first view already explains it.
      if (!openIds.size && r.issues[0]) openIds = new Set([r.issues[0].id]);
    } catch (e: any) {
      if (mine === epoch) error = e.message;
    } finally {
      if (mine === epoch) loading = false;
    }
  }

  $effect(() => {
    instanceId;
    world;
    report = null;
    openIds = new Set();
    packFilter = null;
    void load();
  });

  const issues = $derived.by(() => {
    if (!report) return [];
    const q = query.trim().toLowerCase();
    return report.issues.filter((i) =>
      sevFilter.has(i.severity) &&
      (sourceFilter === 'all' || i.source === sourceFilter) &&
      (!packFilter || i.pack?.folder === packFilter) &&
      (!q || `${i.title} ${i.detail} ${i.pack?.name ?? ''} ${i.hint ?? ''} ${i.rule}`.toLowerCase().includes(q)));
  });
  const shown = $derived(issues.slice(0, limit));
  const troubled = $derived(report?.packs.filter((p) => p.status === 'error' || p.status === 'warning') ?? []);
  const packName = $derived(report?.packs.find((p) => p.folder === packFilter)?.name ?? packFilter);
  const flood = $derived(report?.issues.find((i) => i.source === 'log' && i.count >= 10000) ?? null);

  function toggleSev(s: CompatSeverity) {
    const next = new Set(sevFilter);
    if (next.has(s)) next.delete(s);
    else next.add(s);
    sevFilter = next;
  }
  function setOpen(id: string, value: boolean) {
    const next = new Set(openIds);
    if (value) next.add(id);
    else next.delete(id);
    openIds = next;
  }
  async function exportReport() {
    try { await api.exportCompat(world); } catch (e: any) { addToast(`Export failed: ${e.message}`, 'error'); }
  }
  async function prune() {
    confirmPrune = false;
    try {
      const r = await api.pruneContentLogs();
      addToast(r.message, 'success');
      await load();
    } catch (e: any) { addToast(`Cleanup failed: ${e.message}`, 'error'); }
  }
</script>

<div class="space-y-4">
  <!-- toolbar -->
  <div class="card !py-3 flex flex-wrap items-center gap-3 text-xs">
    <ShieldCheck size={16} class="text-bedrock-400" />
    <div class="min-w-0">
      <p class="text-strong font-semibold uppercase tracking-wider">Compatibility check</p>
      <p class="text-[10px] text-deep-400">
        {#if report}
          {report.summary.files_scanned} log file{report.summary.files_scanned === 1 ? '' : 's'} · {bytes(report.summary.bytes_scanned)} read
          · {report.summary.elapsed_ms} ms{report.summary.truncated ? ' · some files truncated' : ''}{report.world ? ` · world ${report.world}` : ''}
        {:else}
          Reads server logs, manifests and pack scripts. Never modifies packs.
        {/if}
      </p>
    </div>
    <span class="flex-1"></span>
    <button class="btn-secondary text-xs py-1.5 px-3 flex items-center gap-1.5" onclick={() => load(true)} disabled={loading}>
      {#if loading}<Loader2 size={13} class="animate-spin" />{:else}<RotateCw size={13} />{/if} Rescan
    </button>
    <button class="btn-ghost text-xs py-1.5 px-3 flex items-center gap-1.5" onclick={exportReport} disabled={!report}>
      <FileDown size={13} /> Export report
    </button>
  </div>

  {#if error}
    <div class="card text-xs text-red-300">Could not build the report: {error}</div>
  {:else if !report}
    <div class="card space-y-2" aria-busy="true">
      <p class="text-xs text-deep-400 flex items-center gap-2"><Loader2 size={13} class="animate-spin" /> Analysing logs and packs. The first scan of large logs takes a few seconds.</p>
      {#each [0, 1, 2] as j (j)}<div class="h-10 bg-deep-800/40 rounded animate-pulse"></div>{/each}
    </div>
  {:else}
    <!-- server + script API -->
    <div class="card !py-3 flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
      <span class="text-deep-400">Server <span class="font-mono text-strong">{report.server.version ?? 'unknown'}</span></span>
      {#if report.server.endstone_version}<span class="text-deep-400">Endstone <span class="font-mono text-strong">{report.server.endstone_version}</span></span>{/if}
      {#if report.server.beta_apis !== null}
        <span class="text-deep-400">Beta APIs <span class="font-mono {report.server.beta_apis ? 'text-teal-300' : 'text-strong'}">{report.server.beta_apis ? 'on' : 'off'}</span></span>
      {/if}
      {#each report.script_api as m (m.module)}
        <span class="text-deep-400" title="Supported by this server build (source: {m.source})">
          {m.module.replace('@minecraft/', '')} <span class="font-mono text-strong">≤ {m.max_stable ?? '?'}</span>{#if m.beta.length}<span class="font-mono text-deep-300"> · beta {m.beta[0].split('-')[0]}</span>{/if}
        </span>
      {/each}
    </div>

    <!-- summary tiles -->
    <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
      {#each SEVERITIES as s (s)}
        {@const n = s === 'critical' ? report.summary.critical : s === 'error' ? report.summary.errors : s === 'warning' ? report.summary.warnings : report.summary.info}
        <button class="card !p-3 text-left transition {sevFilter.has(s) ? '' : 'opacity-50'}" onclick={() => toggleSev(s)} aria-pressed={sevFilter.has(s)}>
          <span class="{severityBadge(s)} !text-[9px] !py-0.5 !px-1.5">{SEVERITY_LABEL[s]}</span>
          <p class="mt-2 text-xl font-bold font-mono text-strong">{n}</p>
          <p class="text-[10px] text-deep-400 uppercase tracking-wider">issue{n === 1 ? '' : 's'}</p>
        </button>
      {/each}
      <div class="card !p-3">
        <Activity size={14} class="text-bedrock-400" />
        <p class="mt-2 text-xl font-bold font-mono text-strong" title={report.summary.events.toLocaleString()}>{compact(report.summary.events)}</p>
        <p class="text-[10px] text-deep-400 uppercase tracking-wider">log events</p>
      </div>
      <div class="card !p-3">
        <Package size={14} class="text-bedrock-400" />
        <p class="mt-2 text-xl font-bold font-mono text-strong">{report.summary.packs_affected}</p>
        <p class="text-[10px] text-deep-400 uppercase tracking-wider">packs affected</p>
      </div>
    </div>

    {#if flood}
      <div class="card !py-3 border-l-4 border-l-red-400 text-xs leading-relaxed">
        <b class="text-red-300">{compact(flood.count)} events</b> come from one problem:
        <b class="text-strong">{flood.title}</b>{#if flood.pack} in <b class="text-strong">{flood.pack.name}</b>{/if}.
        Fixing or disabling that pack removes most of the log noise.
      </div>
    {/if}

    {#if report.timeline.length > 1}
      <div class="card">
        <h2 class="card-header">Timeline</h2>
        <CompatTimeline entries={report.timeline} />
      </div>
    {/if}

    <div class="grid grid-cols-1 xl:grid-cols-[18rem_1fr] gap-4 items-start">
      <!-- pack health -->
      <div class="card !p-3 xl:sticky xl:top-4">
        <h2 class="card-header">Packs with problems</h2>
        {#if troubled.length === 0}
          <p class="text-xs text-teal-300">No pack-specific problems found.</p>
        {:else}
          <ul class="space-y-1 max-h-[28rem] overflow-y-auto">
            {#each troubled as p (p.pack_type + p.folder)}
              <li>
                <button class="w-full text-left px-2 py-1.5 rounded-sm text-xs flex items-center gap-2 transition
                               {packFilter === p.folder ? 'bg-bedrock-500/15 text-strong' : 'hover:bg-deep-800/50 text-deep-200'}"
                        onclick={() => (packFilter = packFilter === p.folder ? null : p.folder)}>
                  <span class="w-2 h-2 shrink-0 rounded-full {p.status === 'error' ? 'bg-red-400' : 'bg-yellow-400'}"></span>
                  <span class="truncate flex-1" title="{p.name} · {p.folder}">{p.name}</span>
                  <span class="font-mono text-[10px] text-deep-400">{p.events ? compact(p.events) : p.issues}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if report.housekeeping.content_log_bytes > 0}
          <div class="mt-3 pt-3 border-t border-deep-600/30 text-[11px] text-deep-300 space-y-2">
            <p>{report.housekeeping.content_log_files} ContentLog file{report.housekeeping.content_log_files === 1 ? '' : 's'} · <span class="font-mono text-strong">{bytes(report.housekeeping.content_log_bytes)}</span></p>
            {#if canManage && report.housekeeping.content_log_files > 1}
              <button class="btn-ghost text-[10px] px-2 py-1 flex items-center gap-1 text-red-300" onclick={() => (confirmPrune = true)}>
                <Trash2 size={11} /> Delete old content logs
              </button>
            {/if}
          </div>
        {/if}
      </div>

      <!-- issues -->
      <div class="space-y-3 min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <div class="relative flex-1 min-w-[12rem]">
            <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400" />
            <input class="input w-full text-xs py-1.5 pl-7" placeholder="Search issues, packs, messages…" bind:value={query} />
          </div>
          <select class="input text-xs py-1.5 w-40" bind:value={sourceFilter} aria-label="Source">
            <option value="all">All sources</option>
            {#each Object.entries(SOURCE_LABEL) as [k, v] (k)}<option value={k}>{v}</option>{/each}
          </select>
          {#if packFilter}
            <button class="badge-info !text-[10px] !py-1 flex items-center gap-1" onclick={() => (packFilter = null)}>
              {packName} <X size={10} />
            </button>
          {/if}
        </div>
        <p class="text-[10px] text-deep-400 uppercase tracking-wider">{issues.length} of {report.issues.length} issues</p>
        {#each shown as issue (issue.id)}
          <IssueCard {issue} onpack={(f) => (packFilter = f)}
                     bind:open={() => openIds.has(issue.id), (v) => setOpen(issue.id, v)} />
        {:else}
          <div class="card text-xs text-teal-300 text-center py-8">Nothing matches these filters.</div>
        {/each}
        {#if issues.length > shown.length}
          <button class="btn-ghost w-full text-xs py-2" onclick={() => (limit += 60)}>Show more ({issues.length - shown.length} left)</button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirmPrune}
  title="Delete old ContentLog files?"
  message="Deletes every ContentLog*.txt in the server folder except the newest one. These are BDS debug logs; nothing else uses them."
  confirmText="Delete"
  variant="danger"
  onconfirm={prune}
  oncancel={() => (confirmPrune = false)}
/>
