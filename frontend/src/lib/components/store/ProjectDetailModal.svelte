<script lang="ts">
  /**
   * CurseForge project detail: sandboxed description, file list with
   * version filter and paging, and install into the selected world (or as a
   * new world for maps).
   */
  import { api, authsrc } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import { theme, activeMode } from '$lib/theme';
  import type { CfProject, CfFile } from '$types/index';
  import { formatCount, fmtDate, releaseClass, releaseLabel, thumbUrl } from '$lib/store/format';
  import { Package, Loader2, X, ChevronLeft, ChevronRight, ExternalLink, Download, Globe } from '@lucide/svelte';

  let { project, selWorld, pageSize, canInstall, versions, onclose }: {
    project: CfProject;
    selWorld: string;
    pageSize: number;
    canInstall: boolean;
    versions: string[];
    onclose: () => void;
  } = $props();

  const detail = $derived(project);
  let brokenThumb = $state(false);
  let detailDescription = $state('');
  let detailLoading = $state(false);
  let detailFiles = $state<CfFile[]>([]);
  let detailFilesTotal = $state(0);
  let detailFilesPage = $state(1);
  let detailVersion = $state('');
  let filesLoading = $state(false);
  let installTarget = $state<'auto' | 'behavior_packs' | 'resource_packs' | 'world'>('auto');
  let installingFileId = $state<number | null>(null);

  const descriptionDoc = $derived.by(() => {
    // Rebuild when the palette changes so the sandboxed iframe matches.
    void $theme; void $activeMode;
    // The iframe cannot inherit CSS vars, so bake the resolved values in.
    // NOTE: no literal <style> tag may appear in this source — the Svelte
    // CSS pipeline would try to parse the (interpolated) rules as CSS.
    const cssVar = (name: string) =>
      `rgb(${getComputedStyle(document.documentElement).getPropertyValue(name).trim()})`;
    if (!detailDescription) return '';
    const body = detailDescription
      // Belt and braces on top of the script-less sandboxed iframe.
      .replace(/<script[\s\S]*?<\/script>/gi, '')
      .replace(/<style[\s\S]*?<\/style>/gi, '')
      .replace(/<iframe[\s\S]*?<\/iframe>/gi, '');
    const rules = [
      `body { background:${cssVar('--deep-900')}; color:${cssVar('--deep-100')}; font-family:'Montserrat',sans-serif; font-size:13px; line-height:1.55; padding:14px; margin:0; }`,
      'img { max-width:100%; height:auto; border-radius:4px; }',
      `a { color:${cssVar('--bedrock-400')}; }`,
      `h1,h2,h3,h4 { color:${cssVar('--color-strong')}; margin:0.8em 0 0.3em; }`,
      `code, pre { background:${cssVar('--deep-800')}; border:1px solid ${cssVar('--deep-600')}; border-radius:4px; padding:1px 4px; }`,
      'pre { padding:8px; overflow-x:auto; }',
      `table { border-collapse:collapse; } td,th { border:1px solid ${cssVar('--deep-600')}; padding:4px 8px; }`,
      `::-webkit-scrollbar { width:8px; height:8px; } ::-webkit-scrollbar-thumb { background:${cssVar('--deep-600')}; border-radius:4px; }`,
    ].join('\n');
    const styleTag = `<${'style'}>${rules}</${'style'}>`;
    return `<!DOCTYPE html><html><head><base target="_blank">${styleTag}</head><body>${body}</body></html>`;
  });

  // ─── helpers ──────────────────────────────────────────────────────────────

  function needsWorld(): boolean {
    return installTarget !== 'world';
  }

  // Load description + first file page whenever the project changes.
  $effect(() => {
    const p = project;
    void open(p);
  });

  async function open(p: CfProject) {
    detailDescription = '';
    detailFiles = [];
    detailFilesTotal = 0;
    detailFilesPage = 1;
    detailVersion = '';
    brokenThumb = false;
    installTarget = p.classId === 6913 ? 'world' : 'auto';
    detailLoading = true;
    const projectId = p.id;
    const [description, files] = await Promise.allSettled([
      api.storeDescription(projectId),
      api.storeFiles(projectId, 1, pageSize, ''),
    ]);
    if (project.id !== projectId) return; // switched mid-flight
    if (description.status === 'fulfilled') detailDescription = description.value.description;
    if (files.status === 'fulfilled') {
      detailFiles = files.value.files;
      detailFilesTotal = files.value.total;
    } else {
      addToast(`Failed to load files: ${files.reason?.message ?? 'unknown error'}`, 'error');
    }
    detailLoading = false;
  }

  async function loadDetailFiles() {
    if (!detail) return;
    filesLoading = true;
    try {
      const response = await api.storeFiles(detail.id, detailFilesPage, pageSize, detailVersion);
      detailFiles = response.files;
      detailFilesTotal = response.total;
    } catch (e: any) {
      addToast(`Failed to load files: ${e.message}`, 'error');
    } finally {
      filesLoading = false;
    }
  }

  async function installFile(file: CfFile) {
    if (!detail || installingFileId !== null) return;
    if (needsWorld() && (!selWorld || selWorld === '__all__')) {
      addToast('Select a world first (top bar) to install packs into', 'error');
      return;
    }
    installingFileId = file.id;
    try {
      const result = await api.storeInstall({
        project_id: detail.id,
        project_slug: detail.slug,
        file_id: file.id,
        world: selWorld || '',
        target: installTarget,
      });
      addToast(result.message, 'success');
    } catch (e: any) {
      const raw = e.message ?? 'Install failed';
      // request() wraps errors in "API Error <status>: <json>"; surface the detail.
      let message = raw;
      try {
        const jsonStart = raw.indexOf('{');
        if (jsonStart >= 0) message = JSON.parse(raw.slice(jsonStart)).detail ?? raw;
      } catch { /* keep raw */ }
      addToast(message, 'error', 9000);
    } finally {
      installingFileId = null;
    }
  }
</script>

    <div class="fixed inset-0 z-[60] flex items-center justify-center p-4 modal-bg"
       style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
       role="dialog" aria-modal="true" tabindex="-1" aria-label={`Details for ${detail.name}`}
       onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
       onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}>
    <div class="bg-deep-900 border-2 border-deep-600/50 p-4 w-full max-w-4xl shadow-block-lg max-h-[90vh] flex flex-col">
      <!-- header -->
      <div class="flex items-start gap-3 border-b border-deep-700/50 pb-3 shrink-0">
        {#if thumbUrl(detail) && !brokenThumb}
          <img use:authsrc={{ url: thumbUrl(detail), onerror: () => (brokenThumb = true) }}
               alt="" decoding="async"
               class="w-16 h-16 rounded object-cover bg-deep-800 border border-deep-700/50 shrink-0" />
        {:else}
          <div class="w-16 h-16 rounded bg-deep-800 border border-deep-700/50 flex items-center justify-center shrink-0">
            <Package size={24} class="text-deep-400" />
          </div>
        {/if}
        <div class="min-w-0 flex-1">
          <h2 class="text-base font-bold text-strong truncate">{detail.name}</h2>
          <p class="text-xs text-deep-400 mt-0.5">
            {detail.authors.map((a) => a.name).join(', ') || 'Unknown author'}
            · {formatCount(detail.downloadCount)} downloads
            · Updated {fmtDate(detail.dateModified)}
          </p>
          <div class="flex flex-wrap gap-1 mt-1.5">
            {#each detail.categories.slice(0, 6) as c (c.id)}
              <span class="text-[9px] px-1.5 py-0.5 bg-deep-700/50 text-deep-300 rounded-sm uppercase tracking-wider">{c.name}</span>
            {/each}
          </div>
        </div>
        {#if detail.links?.websiteUrl}
          <a href={detail.links.websiteUrl} target="_blank" rel="noopener noreferrer"
             class="btn-ghost p-1.5 shrink-0" title="View on CurseForge">
            <ExternalLink size={16} />
          </a>
        {/if}
        <button onclick={onclose} class="btn-ghost p-1.5 shrink-0" title="Close"><X size={16} /></button>
      </div>

      <!-- install controls -->
      <div class="flex flex-wrap items-center gap-2 py-3 border-b border-deep-700/30 shrink-0">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Install target</span>
        <select bind:value={installTarget} class="input w-52 text-xs py-1.5" disabled={!canInstall}>
          <option value="auto">Auto-detect (packs / world)</option>
          <option value="behavior_packs">Into world · behavior_packs</option>
          <option value="resource_packs">Into world · resource_packs</option>
          <option value="world">As new world (maps)</option>
        </select>
        {#if needsWorld()}
          <span class="text-[10px] text-deep-400 flex items-center gap-1">
            <Globe size={11} /> {selWorld ? `→ ${selWorld}` : 'no world selected'}
          </span>
        {/if}
      </div>

      <!-- scrollable body -->
      <div class="flex-1 overflow-y-auto min-h-0 space-y-4 pt-3">
        <!-- description -->
        <section>
          <h3 class="text-xs font-bold text-deep-300 uppercase tracking-wider mb-2">Description</h3>
          {#if detailLoading}
            <div class="space-y-2 animate-pulse">
              {#each Array(4) as _, i (i)}<div class="h-3 bg-deep-700/40 rounded w-full"></div>{/each}
            </div>
          {:else if descriptionDoc}
            <iframe title="Project description" sandbox="" class="w-full h-72 rounded border border-deep-700/50 bg-deep-950"
                    srcdoc={descriptionDoc}></iframe>
          {:else}
            <p class="text-xs text-deep-400">{detail.summary || 'No description available.'}</p>
          {/if}
        </section>

        <!-- files -->
        <section>
          <div class="flex flex-wrap items-center gap-2 mb-2">
            <h3 class="text-xs font-bold text-deep-300 uppercase tracking-wider">Files ({detailFilesTotal})</h3>
            <div class="flex-1"></div>
            <select bind:value={detailVersion} onchange={() => { detailFilesPage = 1; loadDetailFiles(); }}
                    class="input w-36 text-xs py-1" disabled={!canInstall} aria-label="Filter files by game version">
              <option value="">All versions</option>
              {#each versions as v (v)}<option value={v}>{v}</option>{/each}
            </select>
          </div>
          {#if filesLoading}
            <div class="space-y-2 animate-pulse">
              {#each Array(3) as _, i (i)}<div class="h-10 bg-deep-700/40 rounded"></div>{/each}
            </div>
          {:else if detailFiles.length === 0}
            <p class="text-xs text-deep-400 py-4 text-center">No files match the current filters</p>
          {:else}
            <div class="space-y-1.5">
              {#each detailFiles as file (file.id)}
                <div class="flex items-center gap-2 p-2 border border-deep-700/40 hover:border-deep-500/50 rounded bg-deep-800/30">
                  <div class="min-w-0 flex-1">
                    <p class="text-xs font-medium text-deep-100 truncate" title={file.displayName}>{file.displayName || file.fileName}</p>
                    <div class="flex flex-wrap items-center gap-1.5 mt-0.5">
                      <span class="text-[9px] px-1 py-px rounded-sm border uppercase tracking-wider {releaseClass(file.releaseType)}">{releaseLabel(file.releaseType)}</span>
                      {#each file.gameVersions.slice(0, 2) as gv}<span class="text-[9px] px-1 py-px bg-deep-700/50 text-deep-300 rounded-sm font-mono">{gv}</span>{/each}
                      <span class="text-[10px] text-deep-400">{formatBytes(file.fileLength)}</span>
                      <span class="text-[10px] text-deep-400">·</span>
                      <span class="text-[10px] text-deep-400">{formatCount(file.downloadCount)} downloads</span>
                      <span class="text-[10px] text-deep-400">·</span>
                      <span class="text-[10px] text-deep-400">{fmtDate(file.fileDate)}</span>
                    </div>
                  </div>
                  {#if canInstall}
                    <button onclick={() => installFile(file)}
                            disabled={installingFileId !== null}
                            class="btn-primary text-xs px-3 py-1.5 rounded flex items-center gap-1.5 shrink-0 disabled:opacity-40">
                      {#if installingFileId === file.id}
                        <Loader2 size={12} class="animate-spin" /> Installing…
                      {:else}
                        <Download size={12} /> Install
                      {/if}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
            {#if detailFilesTotal > pageSize}
              <div class="flex items-center justify-center gap-3 mt-3">
                <button onclick={() => { detailFilesPage -= 1; loadDetailFiles(); }} disabled={detailFilesPage <= 1}
                        class="btn-secondary text-xs px-3 py-1 rounded flex items-center gap-1 disabled:opacity-30">
                  <ChevronLeft size={12} /> Prev
                </button>
                <span class="text-xs text-deep-400">Files page {detailFilesPage} of {Math.max(1, Math.ceil(detailFilesTotal / pageSize))}</span>
                <button onclick={() => { detailFilesPage += 1; loadDetailFiles(); }} disabled={detailFilesPage >= Math.ceil(detailFilesTotal / pageSize)}
                        class="btn-secondary text-xs px-3 py-1 rounded flex items-center gap-1 disabled:opacity-30">
                  Next <ChevronRight size={12} />
                </button>
              </div>
            {/if}
          {/if}
        </section>
      </div>
    </div>
  </div>
