<!-- Console: the instance's server console (shared Terminal widget) plus the
     error-tracking tab. The terminal owns filters, ANSI rendering, match
     navigation, scroll behavior, and the command input. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { consoleLines, errorStats, serverStatus, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { Send, AlertTriangle, Play, Square, RotateCw } from '@lucide/svelte';
  import { detectLevel } from '$lib/console';
  import type { FeedLine } from '$lib/console';
  import Terminal from '$lib/components/terminal/Terminal.svelte';
  import type { ErrorStat } from '$types/index';

  function loadSetting<T>(key: string, def: T): T {
    if (typeof localStorage === 'undefined') return def;
    try { return JSON.parse(localStorage.getItem(key) || 'null') ?? def; }
    catch { return def; }
  }
  function saveSetting(key: string, value: unknown) {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, JSON.stringify(value));
  }

  let tab = $state<'console' | 'errors'>(
    loadSetting('omb_console_tab', 'console' as string) === 'errors' ? 'errors' : 'console'
  );
  $effect(() => { saveSetting('omb_console_tab', tab); });

  let lines = $state<FeedLine[]>([]);
  let errors = $state<ErrorStat[]>([]);

  consoleLines.subscribe(v => { lines = v; });

  const unsubErrors = errorStats.subscribe(v => {
    errors = v;
  });

  onDestroy(() => { unsubErrors(); });

  onMount(async () => {
    try {
      serverStatus.set(await api.getServerStatus());
    } catch { /* ignore */ }
  });

  async function send(cmd: string) {
    try {
      await api.sendCommand(cmd);
    } catch (e: any) {
      addToast(`Command failed: ${e.message}`, 'error');
    }
  }

  function clearLines() {
    consoleLines.clear();
    addToast('Console cleared', 'info');
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

  let errorCount = $derived(errors.reduce((a, e) => a + e.count, 0));

  let errorSortKey = $state<'count' | 'signature' | 'last_seen'>('count');
  let errorSortDir = $state<'desc' | 'asc'>('desc');

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
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-4">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Console{#if $activeInstance && $activeInstance.id !== 'default'} <span class="text-deep-400 text-sm">· {$activeInstance.name}</span>{/if}</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    {#if $userPermissions.includes('SERVER_MANAGE')}
      <div class="flex gap-2 flex-wrap justify-end">
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
    <Terminal
      title="server-console"
      heightKey="console"
      {lines}
      emptyHint="> Console output appears here..."
      onclear={clearLines}
      onsend={send}
      canSend={$userPermissions.includes('CONSOLE_SEND')}
      sendPlaceholder="Enter command..."
    />
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
</div>
