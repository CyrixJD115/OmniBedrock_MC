<script lang="ts">
  import '../app.css';
  import Sidebar from '$components/layout/Sidebar.svelte';
  import ToastContainer from '$components/ui/ToastContainer.svelte';
  import { serverStatus } from '$stores/index';
  import { currentUser, authToken } from '$stores/auth';
  import { onMount, onDestroy } from 'svelte';
  import { api, setToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { initShortcuts, registerShortcut } from '$lib/shortcuts';

  let { children }: { children: import('svelte').Snippet } = $props();

  let sidebarOpen = $state(true);
  let checkingAuth = $state(true);

  const unsubs: Array<() => void> = [];

  onDestroy(() => {
    for (const unsub of unsubs) unsub();
    unsubs.length = 0;
    wsManager.disconnect('/api/v1/console/ws');
    wsManager.disconnect('/api/v1/performance/ws');
  });

  function logout() {
    localStorage.removeItem('omb_token');
    localStorage.removeItem('omb_user');
    setToken('');
    currentUser.set(null as any);
    authToken.set('');
    for (const unsub of unsubs) unsub();
    unsubs.length = 0;
    wsManager.disconnect('/api/v1/console/ws');
    wsManager.disconnect('/api/v1/performance/ws');
  }

  let showShortcuts = $state(false);

  onMount(async () => {
    const cleanup = initShortcuts();
    if (cleanup) onDestroy(cleanup);

    registerShortcut({ key: '?', description: 'Show keyboard shortcuts', handler: () => showShortcuts = !showShortcuts });
    registerShortcut({ key: '1', ctrl: true, description: 'Dashboard', handler: () => goto('/') });
    registerShortcut({ key: '2', ctrl: true, description: 'Console', handler: () => goto('/console') });
    registerShortcut({ key: '3', ctrl: true, description: 'Properties', handler: () => goto('/properties') });
    registerShortcut({ key: '4', ctrl: true, description: 'Backups', handler: () => goto('/backups') });
    registerShortcut({ key: '5', ctrl: true, description: 'Players', handler: () => goto('/players') });
    registerShortcut({ key: '6', ctrl: true, description: 'Performance', handler: () => goto('/performance') });

    const savedToken = typeof localStorage !== 'undefined' ? localStorage.getItem('omb_token') || '' : '';
    const savedUser = typeof localStorage !== 'undefined' ? localStorage.getItem('omb_user') || '' : '';

    if (savedToken) {
      setToken(savedToken);
      authToken.set(savedToken);
      if (savedUser) {
        try {
          currentUser.set(JSON.parse(savedUser));
        } catch { /* ignore */ }
      }
      try {
        const user = await api.getMe();
        currentUser.set(user as any);
        localStorage.setItem('omb_user', JSON.stringify(user));
        await boot();
        checkingAuth = false;
        return;
      } catch {
        logout();
      }
    }
    checkingAuth = false;
    if (!isLoginPage) {
      goto('/login');
    }
  });

  async function boot() {
    try {
      const status = await api.getServerStatus();
      serverStatus.set(status);
    } catch { /* server might not be up yet */ }

    unsubs.push(wsManager.connect('/api/v1/console/ws', onConsoleMessage));
    unsubs.push(wsManager.connect('/api/v1/performance/ws', onMetricsMessage));
  }

  function onConsoleMessage(data: Record<string, unknown>) {
    if (data.type !== 'console') return;
    import('$lib/console').then(c => {
      import('$stores/index').then(m => {
        const level = (data.level as string) || c.detectLevel(data.line as string);
        m.consoleLines.update(lines => {
          lines.push({ text: data.line as string, level: level as never, timestamp: data.timestamp as number });
          if (lines.length > 2000) lines.splice(0, lines.length - 2000);
          return lines;
        });
      });
    });
  }

  function onMetricsMessage(data: Record<string, unknown>) {
    if (data.type === 'metrics') {
      import('$stores/index').then(m => m.metrics.set(data.data as never));
    } else if (data.type === 'error_stats') {
      import('$stores/index').then(m => m.errorStats.set(data.errors as never));
    }
  }

  let isLoginPage = $derived($page.url.pathname === '/login');
</script>

{#if checkingAuth}
  <div class="min-h-screen bg-deep-950 flex items-center justify-center">
    <div class="w-5 h-5 bg-bedrock-500 animate-pulse" style="box-shadow: 0 0 16px rgba(6,182,212,0.4);"></div>
  </div>
{:else if isLoginPage}
  {@render children()}
  <ToastContainer />
{:else if $currentUser}
  <div class="flex h-screen overflow-hidden bg-deep-950">
    <Sidebar {sidebarOpen} onlogout={logout} />
    <main class="flex-1 overflow-auto p-6">
      <div class="max-w-7xl mx-auto animate-fade-in">
        {@render children()}
      </div>
    </main>
  </div>
  <ToastContainer />

  {#if showShortcuts}
    <div class="fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1"
         style="background: rgba(3,8,16,0.85); backdrop-filter: blur(4px);"
         onclick={() => showShortcuts = false}
         onkeydown={(e) => { if (e.key === 'Escape') showShortcuts = false; }}>
      <div class="bg-deep-900 border-2 border-deep-600/50 p-6 w-full max-w-md shadow-block-lg shadow-black/50" role="none"
           onmousedown={(e) => e.stopPropagation()}>
        <h2 class="text-sm font-bold text-white uppercase tracking-widest mb-4">Keyboard Shortcuts</h2>
        <div class="space-y-2 text-xs">
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">?</kbd> — Help</span><span class="text-deep-400">Toggle this overlay</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+1</kbd> — Dashboard</span><span class="text-deep-400">Go to Dashboard</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+2</kbd> — Console</span><span class="text-deep-400">Go to Console</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+3</kbd> — Properties</span><span class="text-deep-400">Go to Properties</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+4</kbd> — Backups</span><span class="text-deep-400">Go to Backups</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+5</kbd> — Players</span><span class="text-deep-400">Go to Players</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+6</kbd> — Performance</span><span class="text-deep-400">Go to Performance</span></div>
        </div>
        <p class="text-[10px] text-deep-500 mt-4 text-center uppercase tracking-wider">Press ? or click anywhere to close</p>
      </div>
    </div>
  {/if}
{/if}

<style>
  :global(body) { margin: 0; padding: 0; }
</style>
