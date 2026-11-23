<script lang="ts">
  /** One compatibility issue: collapsed summary row, expandable diagnosis. */
  import type { CompatIssue } from '$types/compat';
  import { SEVERITY_LABEL, SOURCE_LABEL, compact, issueText, severityBadge, severityBorder } from '$lib/compat/format';
  import { addToast } from '$stores/toast';
  import { ChevronDown, ChevronRight, Copy, ExternalLink, MapPin, Package, Wrench } from '@lucide/svelte';

  let { issue, open = $bindable(false), onpack }: {
    issue: CompatIssue;
    open?: boolean;
    onpack?: (folder: string) => void;
  } = $props();

  const peak = $derived(Math.max(1, ...issue.by_file.map((f) => f.count)));

  async function copy() {
    try {
      await navigator.clipboard.writeText(issueText(issue));
      addToast('Issue copied', 'success');
    } catch {
      addToast('Clipboard unavailable', 'error');
    }
  }
</script>

<div class="mc-slot border-l-4 {severityBorder(issue.severity)}">
  <button class="w-full flex items-start gap-3 px-3 py-2.5 text-left hover:bg-deep-800/40 transition-colors"
          onclick={() => (open = !open)} aria-expanded={open}>
    <span class="mt-0.5 text-deep-400 shrink-0">
      {#if open}<ChevronDown size={14} />{:else}<ChevronRight size={14} />{/if}
    </span>
    <span class="min-w-0 flex-1">
      <span class="flex flex-wrap items-center gap-2">
        <span class="{severityBadge(issue.severity)} !text-[9px] !py-0.5 !px-1.5">{SEVERITY_LABEL[issue.severity]}</span>
        <span class="text-xs text-strong font-semibold break-words">{issue.title}</span>
      </span>
      <span class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10px] text-deep-400">
        {#if issue.pack}
          <span class="flex items-center gap-1 text-bedrock-300"><Package size={10} />{issue.pack.name}</span>
        {:else if issue.hint}
          <span class="font-mono">{issue.hint}</span>
        {/if}
        <span>{SOURCE_LABEL[issue.source]}</span>
        {#if issue.first_seen}<span title="First occurrence in the scanned logs">since {issue.first_seen}</span>{/if}
      </span>
    </span>
    {#if issue.source === 'log'}
      <span class="shrink-0 text-right">
        <span class="block font-mono text-sm text-strong" title="{issue.count.toLocaleString()} occurrences">{compact(issue.count)}</span>
        <span class="block text-[9px] uppercase tracking-wider text-deep-400">events</span>
      </span>
    {/if}
  </button>

  {#if open}
    <div class="border-t border-deep-600/20 px-4 py-3 space-y-3 text-xs">
      <pre class="terminal-surface whitespace-pre-wrap break-words text-[11px] p-2 rounded-sm max-h-40 overflow-auto">{issue.detail}</pre>

      <p class="text-deep-200 leading-relaxed">{issue.explanation}</p>

      <div>
        <p class="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-deep-400 mb-1.5"><Wrench size={11} /> How to fix</p>
        <ol class="list-decimal pl-5 space-y-1 text-deep-100">
          {#each issue.fixes as fix, i (i)}<li class="leading-snug">{fix}</li>{/each}
        </ol>
      </div>

      {#if issue.location}
        <p class="flex items-center gap-1.5 text-deep-300">
          <MapPin size={11} class="text-deep-400" />
          <span class="font-mono">{issue.location.function ?? '<anonymous>'}</span>
          <span class="text-deep-400">in</span>
          <span class="font-mono">{issue.location.file ?? '?'}{issue.location.line ? `:${issue.location.line}` : ''}</span>
        </p>
      {/if}
      {#if issue.context}
        <pre class="terminal-surface whitespace-pre-wrap break-all text-[10px] p-2 rounded-sm max-h-48 overflow-auto">{issue.context}</pre>
      {/if}

      {#if issue.samples.length}
        <div>
          <p class="text-[10px] uppercase tracking-wider text-deep-400 mb-1">Examples</p>
          <div class="flex flex-wrap gap-1">
            {#each issue.samples as s (s)}<code class="text-[10px] bg-deep-800/60 px-1.5 py-0.5 rounded-sm break-all">{s}</code>{/each}
          </div>
        </div>
      {/if}

      {#if issue.by_file.length > 1}
        <div>
          <p class="text-[10px] uppercase tracking-wider text-deep-400 mb-1">Per log file</p>
          <div class="flex items-end gap-1 h-12">
            {#each issue.by_file as f (f.label)}
              <div class="flex-1 min-w-[6px] bg-red-400/60 rounded-t-sm" style="height: {Math.max(4, (f.count / peak) * 100)}%"
                   title="{f.label}: {f.count.toLocaleString()}"></div>
            {/each}
          </div>
        </div>
      {/if}

      <div class="flex flex-wrap items-center gap-x-4 gap-y-1 pt-1">
        {#each issue.docs as d (d.url)}
          <a href={d.url} target="_blank" rel="noopener noreferrer" class="flex items-center gap-1 text-bedrock-300 hover:text-bedrock-200 hover:underline">
            <ExternalLink size={10} />{d.title}
          </a>
        {/each}
        <span class="flex-1"></span>
        {#if issue.pack && onpack}
          <button class="btn-ghost text-[10px] px-2 py-1" onclick={() => onpack?.(issue.pack!.folder)}>Only this pack</button>
        {/if}
        <button class="btn-ghost text-[10px] px-2 py-1 flex items-center gap-1" onclick={copy}><Copy size={10} /> Copy</button>
      </div>
    </div>
  {/if}
</div>
