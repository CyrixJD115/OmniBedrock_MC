<!-- Shared terminal widget used by the Console and Broadcaster pages:
     ANSI rendering with sanitization, filter/regex/level search with match
     navigation, download, stick-scroll with jump-to-latest, freeze-while-
     scrolled-up, wrap + timestamp toggles, and a drag-resizable, viewport-
     adaptive height persisted per instance of the widget. The page owns the
     feed store and the send handler; this component owns the rendering. -->
<script lang="ts">
  import { Filter, ArrowUp, ArrowDown, Binary, Download, Trash2, Eraser, WrapText, Clock, Send } from '@lucide/svelte';
  import { parseAnsi, sanitizeConsoleHtml, highlightMatches, matchesQuery, nearBottom, stickToBottom } from '$lib/console';
  import type { FeedLine } from '$lib/console';

  let {
    title,
    lines,
    heightKey = '',
    emptyHint = '> Console output appears here…',
    onclear,
    onsend,
    canSend = false,
    sendPlaceholder = 'Enter command…',
  }: {
    title: string;
    lines: FeedLine[];
    /** Persisted-height key; two widgets on one page can differ. */
    heightKey?: string;
    emptyHint?: string;
    onclear?: () => void;
    onsend?: (cmd: string) => Promise<void> | void;
    canSend?: boolean;
    sendPlaceholder?: string;
  } = $props();

  function loadSetting<T>(key: string, def: T): T {
    if (typeof localStorage === 'undefined') return def;
    try { return JSON.parse(localStorage.getItem(key) || 'null') ?? def; }
    catch { return def; }
  }
  function saveSetting(key: string, value: unknown) {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, JSON.stringify(value));
  }

  // Intentionally captured once: a terminal instance never changes identity.
  // svelte-ignore state_referenced_locally
  const storeKey = `omb_term_${heightKey || title}`;

  // ─── toolbar state (all persisted) ─────────────────────────────────────────
  let filterText = $state(loadSetting(`${storeKey}_filter`, ''));
  let useRegex = $state(loadSetting(`${storeKey}_regex`, false));
  let levelFilter = $state(loadSetting(`${storeKey}_level`, ''));
  let autoScroll = $state(loadSetting(`${storeKey}_scroll`, true));
  let wrap = $state(loadSetting(`${storeKey}_wrap`, true));
  let timestamps = $state(loadSetting(`${storeKey}_ts`, false));
  $effect(() => { saveSetting(`${storeKey}_filter`, filterText); });
  $effect(() => { saveSetting(`${storeKey}_regex`, useRegex); });
  $effect(() => { saveSetting(`${storeKey}_level`, levelFilter); });
  $effect(() => { saveSetting(`${storeKey}_scroll`, autoScroll); });
  $effect(() => { saveSetting(`${storeKey}_wrap`, wrap); });
  $effect(() => { saveSetting(`${storeKey}_ts`, timestamps); });

  // ─── stick / freeze ────────────────────────────────────────────────────────
  // Follow new output only while the reader is at the bottom. Scrolling up
  // FREEZES the rendered view: incoming bursts accumulate invisibly (counted
  // in the jump pill) instead of shifting the text under the reader's eyes.
  let stick = $state(true);
  let frozenAt = $state(0);
  let terminalEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    if (stick) frozenAt = lines.length;
  });

  let renderList = $derived(stick ? lines : lines.slice(0, frozenAt));
  let newCount = $derived(Math.max(0, lines.length - frozenAt));

  function onTerminalScroll() {
    if (!terminalEl) return;
    stick = nearBottom(terminalEl);
  }

  function jumpToLatest() {
    stick = true;
    if (terminalEl) stickToBottom(terminalEl);
  }

  // Runs after the DOM update, so scrollHeight is final. While frozen the
  // rendered length never changes, so this fires but no-ops.
  $effect(() => {
    void renderList.length;
    if (!autoScroll || !stick || !terminalEl) return;
    stickToBottom(terminalEl);
  });

  // ─── filtering + match navigation ──────────────────────────────────────────
  let filtered = $derived.by(() => {
    const out: FeedLine[] = [];
    for (const l of lines) {
      if (levelFilter && l.level !== levelFilter) continue;
      if (filterText && !matchesQuery(l.text, filterText, useRegex)) continue;
      out.push(l);
    }
    return out;
  });
  // The view the user scrolls: frozen slices the UNFILTERED feed so nothing
  // shifts while reading; live rendering applies the filter.
  let visible = $derived(stick ? filtered : renderList);

  let currentMatch = $state(-1);
  let matchIds = $derived(stick ? filtered.map((f) => f.id) : []);
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

  function filterActive(): boolean {
    return filterText.length > 0 || levelFilter.length > 0;
  }

  // Rendered HTML is memoized per (line text, filter, regex mode): typing in
  // the filter re-derives the list on every keystroke, and without the cache
  // that rebuilt up to 2000 ANSI parses + {@html} strings each time.
  const renderCache = new Map<string, string>();
  function renderLine(text: string): string {
    const key = `${useRegex ? 'R' : 'S'}\u0000${filterText}\u0000${text}`;
    let html = renderCache.get(key);
    if (html === undefined) {
      // Sanitize AFTER the ANSI parse: ansi_up emits real <a href> for
      // OSC-8 sequences, and console text is attacker-influenced.
      html = sanitizeConsoleHtml(parseAnsi(text));
      if (filterText) html = highlightMatches(html, filterText, useRegex);
      if (renderCache.size >= 4000) renderCache.clear();
      renderCache.set(key, html);
    }
    return html;
  }

  function timeOf(line: FeedLine): string {
    if (!line.timestamp) return '';
    return new Date(line.timestamp * 1000).toLocaleTimeString('en-GB', { hour12: false });
  }

  function downloadLog() {
    const text = lines.map((l) => l.text).join('\n');
    const blob = new Blob([text], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${title}-${new Date().toISOString().slice(0, 19)}.log`;
    a.click();
    URL.revokeObjectURL(url);
  }

  // ─── adaptive + drag-resizable height ──────────────────────────────────────
  let heightPx = $state<number | null>(loadSetting(`${storeKey}_h`, null));
  let surfaceEl = $state<HTMLDivElement | null>(null);
  let dragging = $state(false);

  function onResizeStart(e: PointerEvent) {
    const startEl = surfaceEl;
    if (!startEl || dragging) return;
    dragging = true;
    const startY = e.clientY;
    const startH = startEl.getBoundingClientRect().height;
    const max = Math.max(220, Math.floor(window.innerHeight * 0.85));
    const move = (ev: PointerEvent) => {
      heightPx = Math.round(Math.min(max, Math.max(180, startH + (startY - ev.clientY))));
    };
    const up = () => {
      dragging = false;
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      saveSetting(`${storeKey}_h`, heightPx);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    e.preventDefault();
  }

  const heightStyle = $derived(
    heightPx ? `${heightPx}px` : 'clamp(260px, calc(100vh - 340px), 72vh)'
  );

  // ─── command input with history ────────────────────────────────────────────
  let input = $state('');
  let commandHistory: string[] = [];
  let historyIdx = $state(-1);

  async function send() {
    const cmd = input.trim();
    if (!cmd || !onsend) return;
    commandHistory.push(cmd);
    historyIdx = -1;
    input = '';
    try {
      await onsend(cmd);
    } catch { /* the page's handler reports its own errors */ }
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter') { void send(); return; }
    if (!onsend) return;
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

  const btnClass = 'flex items-center justify-center w-7 h-7 rounded text-deep-400 hover:text-strong hover:bg-deep-700/50 transition-colors text-[11px] font-mono shrink-0';
  const btnActive = 'text-bedrock-400 bg-deep-700/50';
</script>

<div class="terminal-surface relative border-2 border-deep-600/50 overflow-hidden rounded-sm">
  <!-- chrome -->
  <div class="flex items-center gap-2 px-4 py-2 bg-white/5 border-b border-white/10">
    <span class="w-2.5 h-2.5 bg-red-500/80"></span>
    <span class="w-2.5 h-2.5 bg-yellow-500/80"></span>
    <span class="w-2.5 h-2.5 bg-teal-500/80"></span>
    <span class="text-deep-300 text-[10px] uppercase tracking-wider ml-2 font-mono">{title}</span>
    <span class="text-deep-400 text-[10px] font-mono">{lines.length} lines</span>
    {#if filterActive() && stick}
      <span class="ml-auto text-[10px] font-mono text-deep-300">{filtered.length} / {lines.length}</span>
    {:else if newCount > 0}
      <span class="ml-auto text-[10px] font-mono text-yellow-400">+{newCount} frozen</span>
    {/if}
  </div>

  <!-- controls -->
  <div class="flex flex-wrap items-center gap-1.5 px-3 py-1.5 bg-black/20 border-b border-white/5">
    <div class="relative">
      <Filter size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
      <input type="text" bind:value={filterText} placeholder="Filter"
             class="input pl-7 py-1 text-xs w-36 font-mono" />
    </div>
    <button onclick={() => (useRegex = !useRegex)} title={useRegex ? 'Regex mode on' : 'Regex mode off'}
            class="{btnClass} {useRegex ? btnActive : ''}">
      <Binary size={12} />
    </button>
    <select bind:value={levelFilter}
            class="input py-1 text-xs w-20 font-mono appearance-none bg-deep-900/80 cursor-pointer"
            aria-label="Level filter">
      <option value="">All</option>
      <option value="error">Error</option>
      <option value="warn">Warn</option>
      <option value="info">Info</option>
      <option value="debug">Debug</option>
      <option value="local">Local</option>
    </select>

    {#if matchCount > 0}
      <div class="flex items-center gap-1 text-xs font-mono text-deep-400">
        <button onclick={() => navigateMatch(-1)} class="{btnClass}" title="Previous match"><ArrowUp size={12} /></button>
        <span class="tabular-nums w-14 text-center">{currentMatch + 1}/{matchCount}</span>
        <button onclick={() => navigateMatch(1)} class="{btnClass}" title="Next match"><ArrowDown size={12} /></button>
      </div>
    {/if}

    <div class="flex items-center gap-1 ml-auto">
      <button onclick={() => (wrap = !wrap)} title={wrap ? 'Word wrap on' : 'Word wrap off (horizontal scroll)'}
              class="{btnClass} {wrap ? btnActive : ''}">
        <WrapText size={12} />
      </button>
      <button onclick={() => (timestamps = !timestamps)} title={timestamps ? 'Timestamps on' : 'Timestamps off'}
              class="{btnClass} {timestamps ? btnActive : ''}">
        <Clock size={12} />
      </button>
      <button onclick={downloadLog} class="{btnClass}" title="Download log"><Download size={12} /></button>
      {#if onclear}
        <button onclick={onclear} class="{btnClass}" title="Clear view (backend history keeps flowing)"><Trash2 size={12} /></button>
      {/if}
      <label class="flex items-center gap-1 text-[10px] uppercase tracking-wider text-deep-400 cursor-pointer select-none ml-1">
        <input type="checkbox" bind:checked={autoScroll} class="accent-bedrock-500" />
        Scroll
      </label>
    </div>
  </div>

  <!-- viewport -->
  <div
    bind:this={surfaceEl}
    class="relative"
    style="height: {heightStyle}"
  >
    <div
      bind:this={terminalEl}
      onscroll={onTerminalScroll}
      class="terminal-container h-full overflow-y-auto p-4 font-terminal text-base leading-relaxed {wrap
        ? 'whitespace-pre-wrap break-words'
        : 'whitespace-pre'}"
    >
      {#each visible as f (f.id)}
        <div data-line-id={f.id}
             class={"leading-snug " + (
               f.level === 'error' ? 'terminal-line-error' :
               f.level === 'warn' ? 'terminal-line-warn' :
               f.level === 'debug' ? 'terminal-line-debug' :
               f.level === 'local' ? 'terminal-line-local' :
               'terminal-line-info'
             )}>
          {#if timestamps && timeOf(f)}<span class="text-deep-600 select-none mr-2 text-[0.8em]">{timeOf(f)}</span>{/if}{@html renderLine(f.text)}
        </div>
      {/each}
      {#if visible.length === 0}
        <div class="terminal-line-plain opacity-60 font-mono">
          {filterActive() ? '> No matches' : emptyHint}
        </div>
      {/if}
    </div>
    {#if (!stick && lines.length > 0) || newCount > 0}
      <button onclick={jumpToLatest}
              class="absolute bottom-4 right-4 z-10 flex items-center gap-1.5 px-3 py-1.5 text-[11px] uppercase tracking-wider
                     bg-deep-900/95 border-2 border-bedrock-500/60 text-bedrock-300 hover:text-strong hover:border-bedrock-400 shadow-block transition-colors">
        <ArrowDown size={12} />
        {newCount > 0 ? `Jump to latest (+${newCount})` : 'Jump to latest'}
      </button>
    {/if}
    <!-- drag handle: pull down/up to resize; persists per widget -->
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize terminal"
      class="absolute bottom-0 left-0 right-0 h-1.5 cursor-ns-resize hover:bg-bedrock-500/40 transition-colors {dragging ? 'bg-bedrock-500/60' : ''}"
      onpointerdown={onResizeStart}
    ></div>
  </div>

  <!-- command input -->
  {#if onsend && canSend}
    <div class="flex gap-2 p-3 border-t border-white/10">
      <input type="text" bind:value={input} onkeydown={onKeyDown}
             placeholder={sendPlaceholder}
             class="input flex-1 font-terminal text-base tracking-wider" />
      <button onclick={() => void send()} class="btn-primary flex items-center gap-2 text-xs">
        <Send size={14} /> Send
      </button>
    </div>
  {/if}
</div>

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
</style>
