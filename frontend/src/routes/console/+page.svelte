<!-- Console: the instance's server console (shared Terminal widget) plus the
     error-tracking tab. The terminal owns filters, ANSI rendering, match
     navigation, scroll behavior, and the command input; the layout owns the
     feed store and its websocket, so this page only renders store state —
     auto-subscriptions ($consoleLines, $errorStats) replace the manual
     subscribes that used to leak one per visit. -->
<script lang="ts">
  import { consoleLines, errorStats, serverStatus, isServerRunning } from '$stores/index';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { AlertTriangle, Play, RotateCw, Square, TerminalSquare } from '@lucide/svelte';
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
      // No refetch-and-set here: the console websocket pushes status
      // transitions (type "status"), which the layout folds into the store.
    } catch (e: any) {
      addToast(`Action failed: ${e.message}`, 'error');
    }
    acting = null;
  }

  const errors = $derived($errorStats);
  const errorCount = $derived(errors.reduce((a, e) => a + e.count, 0));

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
    return new Date(ts * 1000).toLocaleTimeString();
  }

  const tabs = $derived([
    { id: 'console' as const, label: 'Console', icon: TerminalSquare, count: null as number | null },
    { id: 'errors' as const, label: 'Errors', icon: AlertTriangle, count: errorCount },
  ]);
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Console{#if $activeInstance && $activeInstance.id !== 'default'} <span class="text-deep-400 text-sm">· {$activeInstance.name}</span>{/if}</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    <div class="flex flex-wrap items-center justify-end gap-3 text-xs">
      <span class="{$isServerRunning ? 'badge-running' : 'badge-stopped'} !text-[9px] !py-0.5 inline-flex items-center gap-1.5">
        <span class="w-1.5 h-1.5 rounded-full {$isServerRunning ? 'bg-teal-400 animate-pulse' : 'bg-deep-400'}"></span>
        {$serverStatus.status}{#if $isServerRunning && $serverStatus.version}<span class="font-mono opacity-70 hidden sm:inline">· {$serverStatus.version}</span>{/if}
      </span>
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
  </div>

  <div class="flex flex-wrap gap-1 border-b border-deep-600/30 pb-px" role="tablist" aria-label="Console views">
    {#each tabs as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}
              class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t flex items-center gap-1.5
                     {tab === t.id ? 'bg-deep-800/60 text-strong border-b-2 border-bedrock-500' : 'text-deep-400 hover:text-deep-200'}">
        <t.icon size={12} class={t.id === 'errors' && errorCount > 0 ? 'text-red-400' : ''} />
        {t.label}
        {#if t.count !== null}<span class="font-normal {errorCount > 0 ? 'text-red-400' : 'text-deep-400'}">({t.count})</span>{/if}
      </button>
    {/each}
  </div>

  {#if tab === 'console'}
    <Terminal
      title="server-console"
      heightKey="console"
      lines={$consoleLines}
      emptyHint={$isServerRunning ? '> Console output appears here...' : '> The server is stopped — start it to see console output.'}
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
                <th class="text-left px-3 py-1.5">
                  <button class="uppercase tracking-wider hover:text-strong transition-colors" onclick={() => toggleSort('signature')}>Signature{sortArrow('signature')}</button>
                </th>
                <th class="text-right px-3 py-1.5 w-16">
                  <button class="uppercase tracking-wider hover:text-strong transition-colors" onclick={() => toggleSort('count')}>Count{sortArrow('count')}</button>
                </th>
                <th class="text-right px-3 py-1.5 w-24">
                  <button class="uppercase tracking-wider hover:text-strong transition-colors" onclick={() => toggleSort('last_seen')}>Last{sortArrow('last_seen')}</button>
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
        <p class="mt-3 text-[10px] text-deep-400">Repeated console errors are folded into signatures as the server runs; they reset on restart.</p>
      {/if}
    </div>
  {/if}
</div>
