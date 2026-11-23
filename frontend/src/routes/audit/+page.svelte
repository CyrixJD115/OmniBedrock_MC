<script lang="ts">
  /**
   * Audit trail: who changed what, where and when. Filters apply as you
   * type (debounced, server-side); older entries load in pages.
   */
  import { api } from '$lib/api/client';
  import { instances } from '$stores/instances';
  import { Search, RotateCw, ClipboardList, Loader2 } from '@lucide/svelte';

  type Entry = Awaited<ReturnType<typeof api.listAudit>>[number];
  const PAGE = 100;
  const CATEGORIES = ['addons', 'auth', 'backup', 'broadcaster', 'console', 'files', 'general', 'instances', 'player', 'plugins', 'properties', 'server', 'settings'];
  const COLORS: Record<string, string> = {
    server: 'text-teal-300', instances: 'text-teal-300', console: 'text-teal-300',
    player: 'text-bedrock-300', auth: 'text-red-300', settings: 'text-purple-300',
    backup: 'text-green-300', files: 'text-yellow-300', properties: 'text-yellow-300',
    addons: 'text-bedrock-300', plugins: 'text-bedrock-300', broadcaster: 'text-purple-300',
  };

  let entries = $state<Entry[]>([]);
  let loading = $state(true);
  let more = $state(false);
  let error = $state('');
  let user = $state('');
  let action = $state('');
  let category = $state('');

  const instanceName = (id: string) => $instances.find((i) => i.id === id)?.name ?? id;

  let epoch = 0;
  async function load(append = false) {
    const mine = ++epoch;
    loading = true;
    error = '';
    try {
      const page = await api.listAudit({
        username: user.trim() || undefined,
        action: action.trim() || undefined,
        category: category || undefined,
        limit: PAGE,
        offset: append ? entries.length : 0,
      });
      if (mine !== epoch) return;
      entries = append ? [...entries, ...page] : page;
      more = page.length === PAGE;
    } catch (e: any) {
      if (mine === epoch) error = e.message.replace(/^API Error \d+: /, '');
    } finally {
      if (mine === epoch) loading = false;
    }
  }

  // Re-query when a filter changes (debounced for typing).
  $effect(() => {
    user; action; category;
    const t = setTimeout(() => void load(), 250);
    return () => clearTimeout(t);
  });

  function when(iso: string): string {
    const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
    if (mins < 1) return 'just now';
    if (mins < 60) return `${mins} min ago`;
    if (mins < 1440) return `${Math.round(mins / 60)} h ago`;
    return new Date(iso).toLocaleDateString();
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Audit Log</h1>
      <div class="pixel-divider mt-2 w-28"></div>
    </div>
    <button onclick={() => load()} class="btn-ghost p-2" aria-label="Refresh" disabled={loading}><RotateCw size={14} /></button>
  </div>

  <div class="card">
    <div class="flex flex-wrap gap-2 mb-4">
      <div class="relative flex-1 min-w-[10rem]">
        <Search size={13} class="absolute left-3 top-1/2 -translate-y-1/2 text-deep-400" />
        <input type="text" bind:value={user} placeholder="User (exact)" aria-label="Filter by user" class="input pl-8 py-1.5 text-xs w-full" />
      </div>
      <input type="text" bind:value={action} placeholder="Action contains…" aria-label="Filter by action" class="input py-1.5 text-xs w-48" />
      <select bind:value={category} class="input py-1.5 text-xs w-40" aria-label="Filter by category">
        <option value="">All categories</option>
        {#each CATEGORIES as c (c)}<option value={c}>{c}</option>{/each}
      </select>
    </div>

    {#if error}
      <p class="text-xs text-red-300 py-6 text-center">{error}</p>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-xs">
          <thead>
            <tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider text-[10px]">
              <th class="text-left py-2 px-3 font-medium">When</th>
              <th class="text-left py-2 px-3 font-medium">User</th>
              <th class="text-left py-2 px-3 font-medium">Category</th>
              <th class="text-left py-2 px-3 font-medium">Action</th>
              <th class="text-left py-2 px-3 font-medium hidden md:table-cell">Instance</th>
              <th class="text-left py-2 px-3 font-medium">Detail</th>
            </tr>
          </thead>
          <tbody>
            {#each entries as e, i (i)}
              <tr class="border-b border-deep-700/20 hover:bg-deep-800/30 align-top">
                <td class="py-1.5 px-3 text-deep-300 whitespace-nowrap" title={new Date(e.timestamp).toLocaleString()}>{when(e.timestamp)}</td>
                <td class="py-1.5 px-3 font-medium text-strong">{e.username}</td>
                <td class="py-1.5 px-3"><span class="uppercase tracking-wider text-[10px] font-semibold {COLORS[e.category] ?? 'text-deep-300'}">{e.category}</span></td>
                <td class="py-1.5 px-3 font-mono">{e.action}</td>
                <td class="py-1.5 px-3 text-deep-400 hidden md:table-cell">{e.instance ? instanceName(e.instance) : '—'}</td>
                <td class="py-1.5 px-3 text-deep-300 max-w-sm truncate" title={e.detail}>{e.detail}</td>
              </tr>
            {:else}
              <tr><td colspan="6" class="text-center py-10 text-deep-400">
                {#if loading}<Loader2 size={14} class="inline animate-spin" /> Loading…{:else}<ClipboardList size={16} class="inline mr-1" /> No entries match these filters{/if}
              </td></tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if more}
        <button onclick={() => load(true)} disabled={loading} class="btn-ghost w-full text-xs py-2 mt-2">
          {loading ? 'Loading…' : 'Load older entries'}
        </button>
      {/if}
    {/if}
  </div>
</div>
