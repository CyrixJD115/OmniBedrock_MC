<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { consoleLines, errorStats, serverStatus, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { Send, Trash2, Filter, AlertTriangle, ArrowUp, ArrowDown, Binary, Download, Play, Square, RotateCw } from '@lucide/svelte';
  import { parseAnsi, sanitizeConsoleHtml, highlightMatches, matchesQuery, nearBottom, stickToBottom } from '$lib/console';
  import type { FeedLine } from '$lib/console';
  import type { ErrorStat } from '$types/index';

  function loadSetting<T>(key: string, def: T): T {
    if (typeof localStorage === 'undefined') return def;
    try { return JSON.parse(localStorage.getItem(key) || 'null') ?? def; }
    catch { return def; }
  }
  function saveSetting(key: string, value: unknown) {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, JSON.stringify(value));
  }

  let input = $state('');
  let autoScroll = $state(loadSetting('omb_console_scroll', true));
  let filterText = $state(loadSetting('omb_console_filter', ''));
  let useRegex = $state(loadSetting('omb_console_regex', false));
  let levelFilter = $state(loadSetting('omb_console_level', ''));
  let tab = $state<'console' | 'errors'>(
    loadSetting('omb_console_tab', 'console' as string) === 'errors' ? 'errors' : 'console'
  );

  $effect(() => { saveSetting('omb_console_filter', filterText); });
  $effect(() => { saveSetting('omb_console_regex', useRegex); });
  $effect(() => { saveSetting('omb_console_level', levelFilter); });
  $effect(() => { saveSetting('omb_console_tab', tab); });
  $effect(() => { saveSetting('omb_console_scroll', autoScroll); });

  let terminalEl: HTMLDivElement;
  let commandHistory: string[] = [];
  let historyIdx = $state(-1);
  let errorSortKey = $state<'count' | 'signature' | 'last_seen'>('count');
  let errorSortDir = $state<'desc' | 'asc'>('desc');
  let currentMatch = $state(-1);
  // Follow new output only while the reader is already at the bottom; once
  // they scroll up to inspect history, their position is left alone.
  let stick = $state(true);

  let lines = $state<FeedLine[]>([]);
  let errors = $state<ErrorStat[]>([]);

  consoleLines.subscribe(v => { lines = v; });

  const unsubErrors = errorStats.subscribe(v => {
    errors = v;
  });

  onDestroy(() => { unsubErrors(); });

  function onTerminalScroll() {
    if (!terminalEl) return;
    stick = nearBottom(terminalEl);
  }

  function jumpToLatest() {
    stick = true;
    if (terminalEl) stickToBottom(terminalEl);
  }

  // Runs after the DOM update, so scrollHeight is final (no content-visibility
  // estimates). Batching in the feed keeps this to at most one scroll per
  // frame, and the stick guard means a detached reader is never yanked down.
  $effect(() => {
    void filtered.length;
    void tab;
    if (tab !== 'console' || !autoScroll || !stick || !terminalEl) return;
    stickToBottom(terminalEl);
  });

  onMount(async () => {
    try {
      serverStatus.set(await api.getServerStatus());
    } catch { /* ignore */ }
  });

  function downloadLog() {
    const text = lines.map(l => l.text).join('\n');
    const blob = new Blob([text], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `console-${new Date().toISOString().slice(0, 19)}.log`;
    a.click();
    URL.revokeObjectURL(url);
  }

  let errorCount = $derived(errors.reduce((a, e) => a + e.count, 0));

  let sortedErrors = $derived(
    [...errors].sort((a, b) => {
      const dir = errorSortDir === 'desc' ? -1 : 1;
      if (errorSortKey === 'count') return (a.count - b.count) * dir;
      if (errorSortKey === 'last_seen') return (a.last_seen - b.last_seen) * dir;
      return a.signature.localeCompare(b.signature) * dir;
    })
  );

  function toggleSort(key: typeof errorSortKey) {
    if (errorSortKey === key) {
      errorSortDir = errorSortDir === 'desc' ? 'asc' : 'desc';
    } else {
      errorSortKey = key;
      errorSortDir = 'desc';
    }
  }

  function sortArrow(key: typeof errorSortKey): string {
    if (errorSortKey !== key) return '';
    return errorSortDir === 'desc' ? ' ↓' : ' ↑';
  }

  function formatTime(ts: number): string {
    const d = new Date(ts * 1000);
    return d.toLocaleTimeString();
  }

  async function send() {
    const cmd = input.trim();
    if (!cmd) return;
    commandHistory.push(cmd);
    historyIdx = -1;
    input = '';
    try {
      await api.sendCommand(cmd);
    } catch (e: any) {
      addToast(`Command failed: ${e.message}`, 'error');
    }
  }

  let acting = $state<string | null>(null);

  async function act(action: string) {
    acting = action;
    try {
      const res = await api.serverAction(action);
      addToast(res.message, res.success ? 'success' : 'error');
      if (res.success) {
        setTimeout(async () => {
          try {
            const s = await api.getServerStatus();
            serverStatus.set(s);
          } catch { /* ignore */ }
        }, 1500);
      }
    } catch (e: any) {
      addToast(`Action failed: ${e.message}`, 'error');
    }
    acting = null;
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') { send(); return; }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (commandHistory.length === 0) return;
      historyIdx = Math.min(historyIdx + 1, commandHistory.length - 1);
      input = commandHistory[commandHistory.length - 1 - historyIdx];
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      historyIdx = Math.max(historyIdx - 1, -1);
      input = historyIdx === -1 ? '' : commandHistory[commandHistory.length - 1 - historyIdx];
    }
  }

  function clearLines() {
    consoleLines.clear();
    addToast('Console cleared', 'info');
    currentMatch = -1;
  }

  function filterActive(): boolean {
    return filterText.length > 0 || levelFilter.length > 0;
  }

  let filtered = $derived.by(() => {
    const out: FeedLine[] = [];
    lines.forEach(l => {
      if (levelFilter && l.level !== levelFilter) return;
      if (!filterText) { out.push(l); return; }
      if (matchesQuery(l.text, filterText, useRegex)) out.push(l);
    });
    return out;
  });

  let matchIds = $derived(filtered.map(f => f.id));

  let matchCount = $derived(matchIds.length);

  function navigateMatch(dir: -1 | 1) {
    if (matchCount === 0) return;
    currentMatch = (currentMatch + dir + matchCount) % matchCount;
    const id = matchIds[currentMatch];
    const el = terminalEl?.querySelector(`[data-line-id="${id}"]`) as HTMLElement | null;
    if (el) {
      el.scrollIntoView({ block: 'center' });
      el.classList.add('console-match-active');
      setTimeout(() => el.classList.remove('console-match-active'), 1500);
    }
  }

  // Rendered HTML is memoized per (line text, filter, regex mode): typing in
  // the filter re-derives the list on every keystroke, and without the cache
  // that rebuilt up to 2000 ANSI parses + {@html} strings each time. The key
  // embeds the filter, so stale entries are never reused — the map is just
  // bounded memory, cleared wholesale when full.
  const renderCache = new Map<string, string>();
  function renderLine(text: string): string {
    const key = `${useRegex ? 'R' : 'S'}\u0000${filterText}\u0000${text}`;
    let html = renderCache.get(key);
    if (html === undefined) {
      // Sanitize AFTER the ANSI parse: ansi_up emits real <a href> for
      // OSC-8 sequences, and console text is attacker-influenced.
      html = sanitizeConsoleHtml(parseAnsi(text));
      if (filterText) {
        html = highlightMatches(html, filterText, useRegex);
      }
      if (renderCache.size >= 4000) renderCache.clear();
      renderCache.set(key, html);
    }
    return html;
  }
</script>

<style>
  .terminal-container {
    -webkit-font-smoothing: none;
    -moz-osx-font-smoothing: unset;
    font-smooth: never;
  }
  :global(.console-match) {
    background: rgb(var(--term-info) / 0.28);
    border-radius: 2px;
    outline: 1px solid rgb(var(--term-info) / 0.55);
  }
  :global(.console-match-active) {
    background: rgb(var(--term-info) / 0.15);
    outline: 2px solid rgb(var(--term-info) / 0.7);
    outline-offset: -2px;
  }
  .filter-btn {
    @apply flex items-center justify-center w-7 h-7 rounded text-deep-400 hover:text-strong hover:bg-deep-700/50 transition-colors text-[11px] font-mono;
  }
  .filter-btn.active {
    @apply text-bedrock-400 bg-deep-700/50;
  }
  .level-dot {
    @apply w-2 h-2 rounded-full inline-block flex-shrink-0;
  }
</style>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Console{#if $activeInstance && $activeInstance.id !== 'default'} <span class="text-deep-400 text-sm">· {$activeInstance.name}</span>{/if}</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex items-center gap-2 flex-wrap justify-end">
      {#if $userPermissions.includes('SERVER_MANAGE')}
        <div class="flex gap-2 mr-2">
          <button onclick={() => act('start')} disabled={$isServerRunning || acting !== null}
                  class="btn-success flex items-center gap-2 text-xs">
            <Play size={14} /> {acting === 'start' ? 'Starting...' : 'Start'}
          </button>
          <button onclick={() => act('restart')} disabled={!$isServerRunning || acting !== null}
                  class="btn-secondary flex items-center gap-2 text-xs">
            <RotateCw size={14} /> {acting === 'restart' ? 'Restarting...' : 'Restart'}
          </button>
          <button onclick={() => act('stop')} disabled={!$isServerRunning || acting !== null}
                  class="btn-danger flex items-center gap-2 text-xs">
            <Square size={14} /> {acting === 'stop' ? 'Stopping...' : 'Stop'}
          </button>
        </div>
      {/if}
      <div class="relative">
        <Filter size={13} class="absolute left-3 top-1/2 -translate-y-1/2 text-deep-400" />
        <input type="text" bind:value={filterText} placeholder="Filter"
               class="input pl-8 py-1.5 text-xs w-36 font-mono" />
      </div>

      <button onclick={() => useRegex = !useRegex}
              title={useRegex ? 'Regex mode on' : 'Regex mode off'}
              class="filter-btn {useRegex ? 'active' : ''}">
        <Binary size={12} />
      </button>

      <div class="relative">
        <select bind:value={levelFilter}
                class="input py-1.5 text-xs w-24 font-mono appearance-none bg-deep-900/80 cursor-pointer">
          <option value="">All</option>
          <option value="error">Error</option>
          <option value="warn">Warn</option>
          <option value="info">Info</option>
          <option value="debug">Debug</option>
          <option value="local">Local</option>
        </select>
      </div>

      {#if matchCount > 0}
        <div class="flex items-center gap-1 text-xs font-mono text-deep-400">
          <button onclick={() => navigateMatch(-1)} class="filter-btn" title="Previous match">
            <ArrowUp size={12} />
          </button>
          <span class="tabular-nums w-14 text-center">{currentMatch + 1}/{matchCount}</span>
          <button onclick={() => navigateMatch(1)} class="filter-btn" title="Next match">
            <ArrowDown size={12} />
          </button>
        </div>
      {/if}

      <button onclick={downloadLog} class="btn-ghost p-2 text-xs" title="Download Log">
        <Download size={14} />
      </button>
      <button onclick={clearLines} class="btn-ghost p-2 text-xs" title="Clear">
        <Trash2 size={14} />
      </button>
      <label class="flex items-center gap-1.5 text-xs text-deep-400 uppercase tracking-wider cursor-pointer select-none">
        <input type="checkbox" bind:checked={autoScroll} class="accent-bedrock-500" />
        Scroll
      </label>
    </div>
  </div>

  <!-- Sub-tabs: keep the live console uncluttered; error tracking gets its own tab. -->
  <div class="flex gap-1" role="tablist" aria-label="Console views">
    <button role="tab" aria-selected={tab === 'console'} onclick={() => tab = 'console'}
            class={"px-4 py-2 text-xs font-bold uppercase tracking-wider border-2 border-b-0 transition-colors " + (tab === 'console'
              ? 'bg-deep-800 text-strong border-deep-600/50'
              : 'bg-deep-900/60 text-deep-400 border-transparent hover:text-strong')}>
      Console
    </button>
    <button role="tab" aria-selected={tab === 'errors'} onclick={() => tab = 'errors'}
            class={"px-4 py-2 text-xs font-bold uppercase tracking-wider border-2 border-b-0 transition-colors inline-flex items-center gap-1.5 " + (tab === 'errors'
              ? 'bg-deep-800 text-strong border-deep-600/50'
              : 'bg-deep-900/60 text-deep-400 border-transparent hover:text-strong')}>
      <AlertTriangle size={12} class={errorCount > 0 ? 'text-red-400' : ''} />
      Errors
      {#if errorCount > 0}<span class="text-red-400 font-mono">({errorCount})</span>{/if}
    </button>
  </div>

  {#if tab === 'console'}
  <!-- The console viewport keeps a dark surface in every theme/mode: ANSI
       colors and the in-game log aesthetic both assume a dark backdrop. -->
  <div class="terminal-surface relative border-2 border-deep-600/50 overflow-hidden">
    <div class="flex items-center gap-2 px-4 py-2 bg-white/5 border-b border-white/10">
      <span class="w-2.5 h-2.5 bg-red-500/80"></span>
      <span class="w-2.5 h-2.5 bg-yellow-500/80"></span>
      <span class="w-2.5 h-2.5 bg-teal-500/80"></span>
      <span class="text-deep-300 text-[10px] uppercase tracking-wider ml-2 font-mono">server-console</span>
      <span class="text-deep-400 text-[10px] font-mono">{lines.length} lines</span>
      {#if filterActive()}
        <span class="ml-auto text-[10px] font-mono text-deep-300">{filtered.length} / {lines.length} lines</span>
      {/if}
    </div>
    <div bind:this={terminalEl} onscroll={onTerminalScroll}
         class="terminal-container h-[55vh] overflow-y-auto p-4 font-terminal text-base leading-relaxed">
      {#each filtered as f (f.id)}
        <div data-line-id={f.id}
             class={"leading-snug " + (
               f.level === 'error' ? 'terminal-line-error' :
               f.level === 'warn' ? 'terminal-line-warn' :
               f.level === 'debug' ? 'terminal-line-debug' :
               f.level === 'local' ? 'terminal-line-local' :
               'terminal-line-info'
             )}>{@html renderLine(f.text)}</div>
      {/each}
      {#if filtered.length === 0}
        <div class="terminal-line-plain opacity-60 font-mono">
          {filterActive() ? '> No matches' : '> Console output appears here...'}
        </div>
      {/if}
    </div>
    {#if !stick && lines.length > 0}
      <button onclick={jumpToLatest}
              class="absolute bottom-4 right-4 z-10 flex items-center gap-1.5 px-3 py-1.5 text-[11px] uppercase tracking-wider
                     bg-deep-900/95 border-2 border-bedrock-500/60 text-bedrock-300 hover:text-strong hover:border-bedrock-400 shadow-block transition-colors">
        <ArrowDown size={12} /> Jump to latest
      </button>
    {/if}
  </div>
  {:else}
  <div class="card">
    <div class="flex items-center justify-between mb-3">
      <h2 class="card-header !mb-0 flex items-center gap-2"><AlertTriangle size={13} class="text-red-400" /> Error Tracking</h2>
      <span class="text-xs text-deep-400 font-mono">{errorCount} total</span>
    </div>
    {#if sortedErrors.length === 0}
      <p class="text-xs text-deep-400 py-6 text-center uppercase tracking-wider">No errors tracked yet</p>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-[11px] font-mono">
          <thead>
            <tr class="border-b border-deep-600/20 text-deep-400">
              <th onclick={() => toggleSort('signature')} class="text-left px-3 py-1.5 cursor-pointer hover:text-strong transition-colors">
                Signature{sortArrow('signature')}
              </th>
              <th onclick={() => toggleSort('count')} class="text-right px-3 py-1.5 cursor-pointer hover:text-strong transition-colors w-16">
                Count{sortArrow('count')}
              </th>
              <th onclick={() => toggleSort('last_seen')} class="text-right px-3 py-1.5 cursor-pointer hover:text-strong transition-colors w-20">
                Last{sortArrow('last_seen')}
              </th>
            </tr>
          </thead>
          <tbody>
            {#each sortedErrors as err (err.signature)}
              <tr class="border-b border-deep-600/10 hover:bg-red-950/20 transition-colors">
                <td class="px-3 py-1 text-red-300 truncate max-w-[400px]" title={err.signature}>{err.signature}</td>
                <td class="px-3 py-1 text-right text-red-400 font-bold">{err.count}</td>
                <td class="px-3 py-1 text-right text-deep-400">{formatTime(err.last_seen)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
  {/if}

  {#if $userPermissions.includes('CONSOLE_SEND')}
    <div class="flex gap-2">
      <input type="text" bind:value={input} onkeydown={onKeyDown}
             placeholder="Enter command..."
             class="input flex-1 font-terminal text-base tracking-wider" />
      <button onclick={send} class="btn-primary flex items-center gap-2 text-xs">
        <Send size={14} /> Send
      </button>
    </div>
  {/if}
</div>
