<script lang="ts">
  import '../app.css';
  import Sidebar from '$components/layout/Sidebar.svelte';
  import Toast from '$components/ui/Toast.svelte';
  import { serverStatus } from '$stores/index';
  import { onMount } from 'svelte';
  import { api, setToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';

  let { children }: { children: import('svelte').Snippet } = $props();

  let sidebarOpen = $state(true);
  let authToken = $state(typeof localStorage !== 'undefined' ? localStorage.getItem('auth_token') || '' : '');
  let showAuthPrompt = $derived(!authToken);

  function handleAuth(token: string) {
    authToken = token;
    showAuthPrompt = false;
    if (typeof localStorage !== 'undefined') localStorage.setItem('auth_token', token);
    setToken(token);
    initConnections();
  }

  async function initConnections() {
    try {
      const status = await api.getServerStatus();
      serverStatus.set(status);
    } catch { /* retry */ }

    wsManager.connect('/api/v1/console/ws', (data) => {
      if (data.type === 'console') {
        import('$stores/index').then(m => {
          m.consoleLines.update(lines => {
            lines.push({
              text: data.line as string,
              level: 'info',
              timestamp: data.timestamp as number,
            });
            if (lines.length > 1000) lines.shift();
            return lines;
          });
        });
      }
    });

    wsManager.connect('/api/v1/performance/ws', (data) => {
      if (data.type === 'metrics') {
        import('$stores/index').then(m => {
          m.metrics.set(data.data as never);
        });
      }
    });
  }

  onMount(() => {
    if (authToken) {
      setToken(authToken);
      initConnections();
    }
  });
</script>

{#if showAuthPrompt}
  <div class="min-h-screen bg-surface-950 flex items-center justify-center">
    <div class="bg-surface-900 p-8 rounded-xl border border-surface-700 w-96">
      <h1 class="text-2xl font-bold text-white mb-2">OmniBedrock MC</h1>
      <p class="text-surface-400 mb-6">Enter your authentication token</p>
      <form onsubmit={(e) => { e.preventDefault(); handleAuth(authToken); }}>
        <input
          type="password"
          bind:value={authToken}
          placeholder="Auth Token"
          class="w-full bg-surface-800 border border-surface-600 rounded-lg px-4 py-2 text-white mb-4 focus:outline-none focus:border-neon-green"
        />
        <button type="submit" class="w-full bg-bedrock-600 hover:bg-bedrock-500 text-white rounded-lg py-2 font-medium transition-colors">
          Connect
        </button>
      </form>
      <p class="text-xs text-surface-500 mt-4">
        Token is shown in the backend console output on startup
      </p>
    </div>
  </div>
{:else}
  <div class="flex h-screen overflow-hidden bg-surface-950">
    <Sidebar {sidebarOpen} />
    <main class="flex-1 overflow-auto p-6">
      {@render children()}
    </main>
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
    padding: 0;
  }
</style>
