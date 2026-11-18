<script lang="ts">
  /**
   * Personal API keys. Keys authenticate on the panel's existing URL and
   * port — no extra listener — with two headers:
   *   X-API-User: <username>
   *   X-API-Key:  omb_…
   * A key acts with its owner's permissions (read-only keys: GET only), the
   * plaintext is shown exactly once, and keys can't manage keys.
   */
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { currentUser, userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import type { ApiKeyRecord } from '$types/index';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import { KeyRound, Copy, Plus, Trash2, Eye, BookOpen } from '@lucide/svelte';

  let keys = $state<ApiKeyRecord[]>([]);
  let loading = $state(true);
  let showAll = $state(false);
  let name = $state('');
  let scope = $state<'full' | 'read_only'>('full');
  let expiresDays = $state(0);
  let busy = $state(false);
  let created = $state<{ key: string; record: ApiKeyRecord } | null>(null);
  let revokeTarget = $state<ApiKeyRecord | null>(null);

  const canSeeAll = $derived($userPermissions.includes('USERS_EDIT'));
  const username = $derived(($currentUser?.username as string | undefined) ?? 'Admin');
  const origin = typeof window !== 'undefined' ? window.location.origin : 'http://localhost:17754';
  const baseUrl = `${origin}/api/v1`;
  const instanceId = $derived($activeInstance?.id ?? 'default');

  function curlExample(key: string): string {
    return `curl -s ${baseUrl}/server/status \\\n  -H "X-API-User: ${username}" \\\n  -H "X-API-Key: ${key}" \\\n  -H "X-Instance: ${instanceId}"`;
  }

  async function load() {
    loading = true;
    try {
      keys = await api.listApiKeys(showAll && canSeeAll);
    } catch (e: any) {
      addToast(`Could not load API keys: ${e.message}`, 'error');
    }
    loading = false;
  }

  async function create() {
    if (!name.trim() || busy) return;
    busy = true;
    try {
      created = await api.createApiKey({
        name: name.trim(),
        scope,
        expires_days: expiresDays || undefined,
      });
      name = '';
      addToast('API key created — copy it now, it will not be shown again', 'success');
      await load();
    } catch (e: any) {
      addToast(`Create failed: ${e.message}`, 'error');
    }
    busy = false;
  }

  async function revoke() {
    const target = revokeTarget;
    revokeTarget = null;
    if (!target) return;
    try {
      await api.revokeApiKey(target.id);
      if (created?.record.id === target.id) created = null;
      addToast(`Revoked '${target.name}'`, 'success');
      await load();
    } catch (e: any) {
      addToast(`Revoke failed: ${e.message}`, 'error');
    }
  }

  function copy(text: string, what = 'Copied') {
    navigator.clipboard?.writeText(text).then(
      () => addToast(what, 'success'),
      () => addToast('Clipboard unavailable — select and copy manually', 'error'),
    );
  }

  function when(iso: string): string {
    if (!iso) return '—';
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  onMount(load);
</script>

<div class="card">
  <div class="flex items-center justify-between gap-2">
    <h2 class="card-header flex items-center gap-2"><KeyRound size={13} /> API Keys</h2>
    <a href="/api-docs" target="_blank" rel="noopener" class="text-[10px] uppercase tracking-wider text-bedrock-300 hover:underline flex items-center gap-1">
      <BookOpen size={11} /> API reference
    </a>
  </div>

  <div class="space-y-4">
    <!-- Where to call -->
    <div class="mc-slot p-3 space-y-2 text-xs">
      <div class="flex items-center justify-between gap-2">
        <span class="text-deep-400 uppercase tracking-wider text-[10px]">Base URL — same address &amp; port as this panel</span>
        <button onclick={() => copy(baseUrl, 'Base URL copied')} class="text-bedrock-400 hover:text-bedrock-300" title="Copy base URL"><Copy size={12} /></button>
      </div>
      <code class="block font-mono text-strong break-all">{baseUrl}</code>
      <div class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[11px] pt-1">
        <span class="text-deep-400">X-API-User</span><span class="text-strong">{username}</span>
        <span class="text-deep-400">X-API-Key</span><span class="text-strong">omb_…</span>
        <span class="text-deep-400">X-Instance</span><span class="text-deep-300">{instanceId} <span class="text-deep-500">(optional, picks the server)</span></span>
      </div>
      <p class="text-[10px] text-deep-400 leading-snug">
        <span class="font-mono">Authorization: Bearer omb_…</span> works too. WebSockets: call
        <span class="font-mono">POST /auth/download-token</span> with your key and pass the result as <span class="font-mono">?dt=</span>.
      </p>
    </div>

    <!-- Freshly created key (shown once) -->
    {#if created}
      <div class="border-2 border-teal-500/50 bg-teal-500/5 p-3 space-y-2">
        <p class="text-[10px] uppercase tracking-widest text-teal-300 flex items-center gap-1.5">
          <Eye size={12} /> New key "{created.record.name}" — copy it now, it is never shown again
        </p>
        <div class="flex items-center gap-2">
          <code class="flex-1 font-mono text-xs text-strong break-all select-all bg-black/30 px-2 py-1.5">{created.key}</code>
          <button onclick={() => copy(created!.key, 'API key copied')} class="btn-primary text-xs px-3 py-1.5 flex items-center gap-1.5"><Copy size={12} /> Copy</button>
        </div>
        <pre class="font-mono text-[10px] text-deep-300 bg-black/30 p-2 overflow-x-auto whitespace-pre">{curlExample(created.key)}</pre>
        <div class="flex justify-between items-center">
          <button onclick={() => copy(curlExample(created!.key), 'curl example copied')} class="text-[10px] text-bedrock-400 hover:underline">copy curl example</button>
          <button onclick={() => (created = null)} class="text-[10px] text-deep-400 hover:text-strong">I saved it — hide</button>
        </div>
      </div>
    {/if}

    <!-- Create -->
    <form class="flex flex-wrap items-end gap-2" onsubmit={(e) => { e.preventDefault(); void create(); }}>
      <label class="space-y-1 block flex-1 min-w-40">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Name</span>
        <input bind:value={name} maxlength="64" placeholder="e.g. discord-bot, uptime-check" class="input w-full text-xs" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Access</span>
        <select bind:value={scope} class="input text-xs">
          <option value="full">Full (your permissions)</option>
          <option value="read_only">Read-only (GET)</option>
        </select>
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Expires</span>
        <select bind:value={expiresDays} class="input text-xs">
          <option value={0}>Never</option>
          <option value={30}>30 days</option>
          <option value={90}>90 days</option>
          <option value={365}>1 year</option>
        </select>
      </label>
      <button type="submit" disabled={busy || !name.trim()} class="btn-primary text-xs px-3 py-2 flex items-center gap-1.5 disabled:opacity-40">
        <Plus size={13} /> {busy ? 'Creating…' : 'Create key'}
      </button>
    </form>

    <!-- List -->
    <div class="space-y-2">
      <div class="flex items-center justify-between">
        <span class="text-[10px] uppercase tracking-wider text-deep-400">{keys.length} key{keys.length === 1 ? '' : 's'}</span>
        {#if canSeeAll}
          <label class="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-deep-400">
            <input type="checkbox" bind:checked={showAll} onchange={load} class="accent-bedrock-500" /> all accounts
          </label>
        {/if}
      </div>
      {#if loading}
        <p class="text-xs text-deep-400">Loading…</p>
      {:else if keys.length === 0}
        <p class="text-xs text-deep-400">No API keys yet.</p>
      {:else}
        <ul class="divide-y divide-deep-600/20 border border-deep-600/30">
          {#each keys as key (key.id)}
            <li class="flex flex-wrap items-center justify-between gap-2 px-3 py-2 text-xs">
              <div class="min-w-0 space-y-0.5">
                <div class="flex items-center gap-2 flex-wrap">
                  <span class="text-strong font-semibold">{key.name}</span>
                  <span class="font-mono text-[10px] text-deep-400">{key.prefix}…</span>
                  <span class="text-[9px] uppercase tracking-wider px-1.5 py-0.5 border {key.scope === 'full' ? 'border-bedrock-500/40 text-bedrock-300' : 'border-teal-500/40 text-teal-300'}">
                    {key.scope === 'full' ? 'full' : 'read-only'}
                  </span>
                  {#if key.expired}<span class="text-[9px] uppercase tracking-wider px-1.5 py-0.5 border border-red-500/40 text-red-300">expired</span>{/if}
                  {#if showAll}<span class="text-[10px] text-deep-400">· {key.user}</span>{/if}
                </div>
                <div class="text-[10px] text-deep-400">
                  created {when(key.created_at)} · last used {when(key.last_used)}{key.last_ip ? ` from ${key.last_ip}` : ''}
                  {#if key.expires_at} · expires {when(key.expires_at)}{/if}
                </div>
              </div>
              <button onclick={() => (revokeTarget = key)} class="btn-ghost text-[11px] px-2 py-1 flex items-center gap-1 text-red-400" title="Revoke">
                <Trash2 size={12} /> Revoke
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
    <p class="text-[10px] text-deep-400 leading-snug">
      Keys act with the permissions of the account that owns them — never more — and stop working when that
      account is deleted. Only a hash is stored. Keys cannot create, list, or revoke other keys.
    </p>
  </div>
</div>

<ConfirmDialog
  open={revokeTarget !== null}
  title="Revoke API key"
  message={revokeTarget ? `Revoke "${revokeTarget.name}" (${revokeTarget.prefix}…)? Anything using it stops working immediately.` : ''}
  confirmText="Revoke"
  onconfirm={revoke}
  oncancel={() => (revokeTarget = null)}
/>
