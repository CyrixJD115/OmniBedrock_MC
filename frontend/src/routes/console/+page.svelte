<script lang="ts">
  import { onDestroy } from 'svelte';
  import { consoleLines, errorStats } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { Send, Trash2, Filter, AlertTriangle, ChevronDown, ChevronRight } from '@lucide/svelte';
  import { parseAnsi, detectLevel } from '$lib/console';
  import type { ErrorStat } from '$types/index';

  let input = $state('');
  let autoScroll = $state(true);
  let filterText = $state('');
  let terminalEl: HTMLDivElement;
  let commandHistory: string[] = [];
  let historyIdx = $state(-1);
  let showErrors = $state(false);
  let errorSortKey = $state<'count' | 'signature' | 'last_seen'>('count');
  let errorSortDir = $state<'desc' | 'asc'>('desc');

  let lines = $state<{ text: string; level: string; timestamp: number }[]>([]);
  let errors = $state<ErrorStat[]>([]);

  const unsubLines = consoleLines.subscribe(v => {
    lines = v;
    if (autoScroll && terminalEl) {
      requestAnimationFrame(() => {
        terminalEl.scrollTop = terminalEl.scrollHeight;
      });
    }
  });

  const unsubErrors = errorStats.subscribe(v => {
    errors = v;
  });

  onDestroy(() => { unsubLines(); unsubErrors(); });

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
    consoleLines.set([]);
    addToast('Console cleared', 'info');
  }

  let filtered = $derived(
    !filterText ? lines : lines.filter(l => l.text.toLowerCase().includes(filterText.toLowerCase()))
  );
</script>

<style>
  .terminal-container {
    -webkit-font-smoothing: none;
    -moz-osx-font-smoothing: unset;
    font-smooth: never;
  }
</style>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Console</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex items-center gap-2">
      <div class="relative">
        <Filter size={13} class="absolute left-3 top-1/2 -translate-y-1/2 text-deep-400" />
        <input type="text" bind:value={filterText} placeholder="Filter"
               class="input pl-8 py-1.5 text-xs w-36 font-mono" />
      </div>
      <button onclick={clearLines} class="btn-ghost p-2 text-xs" title="Clear">
        <Trash2 size={14} />
      </button>
      <label class="flex items-center gap-1.5 text-xs text-deep-400 uppercase tracking-wider cursor-pointer select-none">
        <input type="checkbox" bind:checked={autoScroll} class="accent-bedrock-500" />
        Scroll
      </label>
    </div>
  </div>

  <div class="bg-black/60 border-2 border-deep-600/50 overflow-hidden" style="box-shadow: inset 2px 2px 0 rgba(255,255,255,0.02), inset -1px -1px 0 rgba(0,0,0,0.4), 4px 4px 0 rgba(0,0,0,0.5);">
    <div class="flex items-center gap-2 px-4 py-2 bg-deep-900/80 border-b border-deep-600/30">
      <span class="w-2.5 h-2.5 bg-red-500/80"></span>
      <span class="w-2.5 h-2.5 bg-yellow-500/80"></span>
      <span class="w-2.5 h-2.5 bg-teal-500/80"></span>
      <span class="text-deep-500 text-[10px] uppercase tracking-wider ml-2 font-mono">server-console</span>
    </div>
    <div bind:this={terminalEl}
         class="terminal-container h-[55vh] overflow-y-auto p-4 font-terminal text-base leading-relaxed">
      {#each filtered as line, i (i)}
        <div class={"leading-snug " + (
          line.level === 'error' ? 'terminal-line-error' :
          line.level === 'warn' ? 'terminal-line-warn' :
          line.level === 'debug' ? 'terminal-line-debug' :
          line.level === 'local' ? 'terminal-line-local' :
          'terminal-line-info'
        )}>{@html parseAnsi(line.text)}</div>
      {/each}
      {#if filtered.length === 0}
        <div class="text-deep-600 font-mono">
          {filterText ? '> No matches' : '> Console output appears here...'}
        </div>
      {/if}
    </div>
  </div>

  {#if errors.length > 0}
    <div class="border-2 border-deep-600/50 bg-black/40">
      <button onclick={() => showErrors = !showErrors}
              class="w-full flex items-center gap-2 px-4 py-2 bg-deep-900/80 border-b border-deep-600/30 text-xs uppercase tracking-wider text-deep-300 hover:text-white transition-colors">
        {#if showErrors}<ChevronDown size={13} />{:else}<ChevronRight size={13} />{/if}
        <AlertTriangle size={13} class="text-red-400" />
        Error Tracking
        <span class="ml-auto text-red-400 font-bold">{errorCount} total</span>
      </button>
      {#if showErrors}
        <div class="overflow-x-auto">
          <table class="w-full text-[11px] font-mono">
            <thead>
              <tr class="border-b border-deep-600/20 text-deep-400">
                <th onclick={() => toggleSort('signature')} class="text-left px-3 py-1.5 cursor-pointer hover:text-white transition-colors">
                  Signature{sortArrow('signature')}
                </th>
                <th onclick={() => toggleSort('count')} class="text-right px-3 py-1.5 cursor-pointer hover:text-white transition-colors w-16">
                  Count{sortArrow('count')}
                </th>
                <th onclick={() => toggleSort('last_seen')} class="text-right px-3 py-1.5 cursor-pointer hover:text-white transition-colors w-20">
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

  <div class="flex gap-2">
    <input type="text" bind:value={input} onkeydown={onKeyDown}
           placeholder="Enter command..."
           class="input flex-1 font-terminal text-base tracking-wider" />
    <button onclick={send} class="btn-primary flex items-center gap-2 text-xs">
      <Send size={14} /> Send
    </button>
  </div>
</div>
