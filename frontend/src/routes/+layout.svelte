<script lang="ts">
  import '../app.css';
  import Sidebar from '$components/layout/Sidebar.svelte';
  import ToastContainer from '$components/ui/ToastContainer.svelte';
  import { serverStatus, consoleLines, errorStats, metrics } from '$stores/index';
  import { detectLevel } from '$lib/console';
  import { currentUser, authToken, userPermissions } from '$stores/auth';
  import { onMount, onDestroy } from 'svelte';
  import { api, getToken, setToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { initShortcuts, registerShortcut } from '$lib/shortcuts';
  import { initTheme } from '$lib/theme';
  import { instances, activeInstanceId, loadInstances, loadHost, refreshInstances } from '$stores/instances';
  import type { ConsoleLine } from '$types/index';

  let { children }: { children: import('svelte').Snippet } = $props();

  let sidebarOpen = $state(true);
  let checkingAuth = $state(true);
  let mounted = $state(false);
  let connectedToken = '';

  $effect(() => {
    const token = $authToken;
    // A token is optional: with login disabled the backend authenticates
    // every request (including WebSockets) as the owner account.
    if (mounted && $currentUser && !isLoginPage && token !== connectedToken) {
      connectedToken = token;
      void boot();
    }
    if (!token) connectedToken = '';
  });

  // Reconnect instance-scoped streams whenever the active instance changes.
  $effect(() => {
    const id = $activeInstanceId;
    if (mounted && $currentUser && !isLoginPage && connectedInstances !== '') {
      if (id !== connectedInstances) {
        connectedInstances = id;
        void reconnectInstanceStreams();
      }
    }
  });
  let connectedInstances = '';

  const unsubs: Array<() => void> = [];
  // Theme listeners live for the whole page, not the login session: logout()
  // drains `unsubs`, and theme switching must survive a logout → login.
  let themeUnsub: (() => void) | null = null;

  onDestroy(() => {
    for (const unsub of unsubs) unsub();
    unsubs.length = 0;
    themeUnsub?.();
    themeUnsub = null;
    wsManager.disconnectAll();
  });

  // Keeps <html data-theme/data-mode> in sync with the persisted preference
  // (app.html already applied the saved values before first paint).
  themeUnsub = initTheme();

  function logout() {
    localStorage.removeItem('omb_token');
    localStorage.removeItem('omb_user');
    setToken('');
    instances.set([]);
    connectedInstances = '';
    consoleLines.clear();
    errorStats.set([]);
    metrics.set(null);
    currentUser.set(null as any);
    authToken.set('');
    for (const unsub of unsubs) unsub();
    unsubs.length = 0;
    wsManager.disconnectAll();
    goto('/login');
  }

  let showShortcuts = $state(false);

  onMount(async () => {
    const cleanup = initShortcuts();
    if (cleanup) unsubs.push(cleanup);

    registerShortcut({ key: '?', description: 'Show keyboard shortcuts', handler: () => showShortcuts = !showShortcuts });
    registerShortcut({ key: '1', ctrl: true, description: 'Dashboard', handler: () => goto('/') });
    registerShortcut({ key: '2', ctrl: true, description: 'Console', handler: () => goto('/console') });
    registerShortcut({ key: '3', ctrl: true, description: 'Properties', handler: () => goto('/properties') });
    registerShortcut({ key: '4', ctrl: true, description: 'Backups', handler: () => goto('/backups') });
    registerShortcut({ key: '5', ctrl: true, description: 'Players', handler: () => goto('/players') });
    registerShortcut({ key: '6', ctrl: true, description: 'Performance', handler: () => goto('/performance') });

    const finish = () => { checkingAuth = false; mounted = true; };

    // Public endpoint: tells us whether a login is even required.
    let panel: { login_enabled: boolean } | null = null;
    try { panel = await api.getPanelSettings(); } catch { /* backend still starting */ }
    loginDisabled = panel?.login_enabled === false;

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
      // Quick retries while the backend finishes booting (250ms steps).
      for (let attempt = 0; attempt < 40; attempt++) {
        try {
          const user = await api.getMe();
          currentUser.set(user as any);
          localStorage.setItem('omb_user', JSON.stringify(user));
          finish();
          return;
        } catch {
          if (attempt < 39) await new Promise(r => setTimeout(r, 250));
        }
      }
      logout();
    } else if (panel && panel.login_enabled === false) {
      // Login is disabled: the backend serves every request as Admin.
      try {
        const user = await api.getMe();
        currentUser.set(user as any);
        localStorage.setItem('omb_user', JSON.stringify(user));
        finish();
        if ($page.url.pathname === '/login') goto('/');
        return;
      } catch { /* fall through to the login page */ }
    }
    finish();
    if (!$currentUser && !isLoginPage) {
      goto('/login');
    }
  });

  $effect(() => {
    if (loginDisabled && isLoginPage) goto('/');
  });
  let loginDisabled = $state(false);

  async function boot() {
    try {
      await loadInstances();
    } catch { /* backend might not be up yet */ }
    connectedInstances = $activeInstanceId;
    await reconnectInstanceStreams();
    void loadHost();
    // Keep instance statuses fresh for the switcher without a page reload.
    const poll = setInterval(() => void refreshInstances(), 8000);
    unsubs.push(() => clearInterval(poll));
  }

  /** Point every live stream at the active instance. */
  async function reconnectInstanceStreams() {
    for (const unsub of unsubs.splice(0)) unsub();
    wsManager.disconnectAll();
    consoleLines.clear();
    try {
      const status = await api.getServerStatus();
      serverStatus.set(status);
    } catch { /* instance might not be reachable yet */ }
    // The WebSocket manager appends a fresh short-lived download token.
    const instance = $activeInstanceId || 'default';
    const query = `instance=${encodeURIComponent(instance)}`;
    if ($userPermissions.includes('CONSOLE_VIEW')) unsubs.push(wsManager.connect(`/api/v1/console/ws?${query}`, onConsoleMessage));
    if ($userPermissions.includes('SERVER_VIEW')) unsubs.push(wsManager.connect(`/api/v1/performance/ws?${query}`, onMetricsMessage));
  }

  function onConsoleMessage(data: Record<string, unknown>) {
    if (data.type === 'hello' && data.resumed === false) {
      consoleLines.clear();
    } else if (data.type === 'status') {
      serverStatus.update(status => ({ ...status, status: data.status as typeof status.status }));
    } else if (data.type === 'error_stats') {
      errorStats.set(data.errors as never);
    } else if (data.type === 'gap') {
      addToast(`Console history gap: events ${data.from}–${data.to} expired`, 'error');
    } else if (data.type === 'console') {
      const text = data.line as string;
      consoleLines.push(text, (data.level || detectLevel(text)) as ConsoleLine['level'], {
        timestamp: data.timestamp as number,
        sequence: data.sequence as number,
        stream_id: data.stream_id as string,
        source: data.source as ConsoleLine['source'],
        continued: data.continued as boolean,
      });
    }
  }

  function onMetricsMessage(data: Record<string, unknown>) {
    if (data.type === 'metrics') metrics.set(data.data as never);
  }

  let isLoginPage = $derived($page.url.pathname === '/login');
</script>

{#if checkingAuth}
  <div class="min-h-screen bg-deep-950 flex items-center justify-center">
    <div class="w-5 h-5 bg-bedrock-500 animate-pulse shadow-glow-accent"></div>
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
    <div class="modal-backdrop fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1"
         onclick={() => showShortcuts = false}
         onkeydown={(e) => { if (e.key === 'Escape') showShortcuts = false; }}>
      <div class="bg-deep-900 border-2 border-deep-600/50 p-6 w-full max-w-md shadow-block-lg" role="none"
           onmousedown={(e) => e.stopPropagation()}>
        <h2 class="mc-heading text-sm font-bold uppercase tracking-widest mb-4">Keyboard Shortcuts</h2>
        <div class="space-y-2 text-xs">
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">?</kbd> — Help</span><span class="text-deep-400">Toggle this overlay</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+1</kbd> — Dashboard</span><span class="text-deep-400">Go to Dashboard</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+2</kbd> — Console</span><span class="text-deep-400">Go to Console</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+3</kbd> — Properties</span><span class="text-deep-400">Go to Properties</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+4</kbd> — Backups</span><span class="text-deep-400">Go to Backups</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+5</kbd> — Players</span><span class="text-deep-400">Go to Players</span></div>
          <div class="flex justify-between"><span class="text-deep-300"><kbd class="text-bedrock-400">Ctrl+6</kbd> — Performance</span><span class="text-deep-400">Go to Performance</span></div>
        </div>
        <p class="text-[10px] text-deep-400 mt-4 text-center uppercase tracking-wider">Press ? or click anywhere to close</p>
      </div>
    </div>
  {/if}
{/if}

<style>
  :global(body) { margin: 0; padding: 0; }
</style>
