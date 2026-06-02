<script lang="ts">
  import { api, setToken } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, authToken } from '$stores/auth';
  import { goto } from '$app/navigation';

  let username = $state('');
  let password = $state('');
  let loading = $state(false);
  let error = $state('');

  async function handleSubmit(e: Event) {
    e.preventDefault();
    if (!username.trim() || !password.trim()) { error = 'Enter username and password'; return; }
    loading = true;
    error = '';
    try {
      const res = await api.login(username.trim(), password.trim());
      setToken(res.token);
      authToken.set(res.token);
      currentUser.set(res.user as any);
      localStorage.setItem('omb_token', res.token);
      localStorage.setItem('omb_user', JSON.stringify(res.user));
      addToast('Logged in', 'success');
      goto('/');
    } catch (e: any) {
      error = e.message.includes('401') ? 'Invalid username or password' : 'Server unreachable';
    }
    loading = false;
  }
</script>

<div class="min-h-screen bg-deep-950 flex items-center justify-center p-4"
     style="background-image: radial-gradient(ellipse at 50% 30%, rgba(6,145,178,0.06) 0%, transparent 60%);">
  <div class="bg-deep-900 border-2 border-deep-600/50 p-8 w-full max-w-sm shadow-block-lg shadow-black/50"
       style="box-shadow: inset 2px 2px 0 rgba(255,255,255,0.03), inset -1px -1px 0 rgba(0,0,0,0.3), 6px 6px 0 rgba(0,0,0,0.5);">
    <div class="flex items-center gap-3 mb-6 pb-4 border-b border-deep-600/30">
      <div class="w-5 h-5 bg-bedrock-500" style="box-shadow: 0 0 16px rgba(6,182,212,0.4);"></div>
      <div>
        <h1 class="text-white font-bold text-sm uppercase tracking-widest">OmniBedrock</h1>
        <p class="text-deep-400 text-xs uppercase tracking-wider">Control Panel</p>
      </div>
    </div>

    <p class="text-deep-300 text-xs uppercase tracking-wider mb-4">Sign In</p>

    <form onsubmit={handleSubmit}>
      <input type="text" bind:value={username} placeholder="Username"
             class="input w-full mb-3 text-sm" />
      <input type="password" bind:value={password} placeholder="Password"
             class="input w-full mb-3 text-sm" />
      {#if error}
        <p class="text-red-400 text-xs mb-3 uppercase tracking-wider">{error}</p>
      {/if}
      <button type="submit" disabled={loading} class="btn-primary w-full text-xs">
        {loading ? 'Signing in...' : 'Sign In'}
      </button>
    </form>
  </div>
</div>
