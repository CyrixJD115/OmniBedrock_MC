<script lang="ts">
  import { onMount } from 'svelte';
  import { api, setToken } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, authToken } from '$stores/auth';
  import { goto } from '$app/navigation';

  let username = $state('');
  let password = $state('');
  let loading = $state(false);
  let error = $state('');
  let defaultCredentials = $state(false);
  let loginEnabled = $state(true);

  onMount(async () => {
    try {
      const panel = await api.getPanelSettings();
      loginEnabled = panel.login_enabled;
      defaultCredentials = panel.default_credentials;
      if (!loginEnabled) {
        // No login required; the panel serves requests as Admin.
        goto('/', { replaceState: true });
        return;
      }
      if (defaultCredentials) {
        username = 'Admin';
        password = '123456';
      }
    } catch { /* backend may still be starting */ }
  });

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
     style="background-image: radial-gradient(ellipse at 50% 30%, rgb(var(--bedrock-500) / 0.07) 0%, transparent 60%);">
  <div class="card w-full max-w-sm p-8">
    <div class="flex items-center gap-3 mb-6 pb-4 border-b-2 border-deep-600/40">
      <img src="/favicon.png" alt="" class="w-8 h-8" />
      <div>
        <h1 class="mc-heading text-strong font-bold text-sm uppercase tracking-widest">OmniBedrock</h1>
        <p class="text-deep-400 text-xs uppercase tracking-wider">Control Panel</p>
      </div>
    </div>

    <p class="text-deep-300 text-xs uppercase tracking-wider mb-4">Sign In</p>

    <form onsubmit={handleSubmit} class="space-y-3">
      <input type="text" bind:value={username} placeholder="Username"
             class="input w-full text-sm" autocomplete="username" />
      <input type="password" bind:value={password} placeholder="Password"
             class="input w-full text-sm" autocomplete="current-password" />
      {#if error}
        <p class="text-red-400 text-xs uppercase tracking-wider" role="alert">{error}</p>
      {/if}
      <button type="submit" disabled={loading} class="btn-primary w-full text-xs">
        {loading ? 'Signing in...' : 'Sign In'}
      </button>
    </form>

    {#if defaultCredentials}
      <div class="mt-5 pt-4 border-t border-deep-600/30">
        <p class="text-[10px] text-yellow-400/90 uppercase tracking-wider leading-relaxed">
          Default credentials are active (Admin / 123456) — they are prefilled above.
          Change the password in Users after signing in.
        </p>
      </div>
    {/if}
  </div>
</div>
