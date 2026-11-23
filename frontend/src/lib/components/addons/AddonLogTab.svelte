<script lang="ts">
  /**
   * Mods activity: this session's actions plus the persistent audit trail of
   * pack changes on the active instance (who uploaded, deleted, reordered),
   * which survives reloads and shows other admins' changes too.
   */
  import { api } from '$lib/api/client';
  import { userPermissions } from '$stores/auth';
  import { RotateCw } from '@lucide/svelte';

  let { actionLog = $bindable([]), instanceId }: { actionLog?: string[]; instanceId: string } = $props();

  type Entry = { timestamp: string; username: string; action: string; detail: string; instance: string };
  let history = $state<Entry[]>([]);
  let error = $state('');
  const canAudit = $derived($userPermissions.includes('AUDIT_VIEW'));

  async function load() {
    if (!canAudit) return;
    try {
      const all = await api.listAudit({ category: 'addons', limit: 300 });
      history = all.filter((e) => !e.instance || e.instance === instanceId);
      error = '';
    } catch (e: any) {
      error = e.message;
    }
  }

  $effect(() => {
    instanceId;
    void load();
  });

  function label(action: string): string {
    return action.replace(/^addons\./, '').replace(/[._-]/g, ' ');
  }
</script>

<div class="grid grid-cols-1 xl:grid-cols-2 gap-4">
  <div class="card">
    <div class="flex items-center justify-between mb-2">
      <h2 class="card-header !mb-0">This session</h2>
      <button onclick={() => (actionLog = [])} class="btn-ghost text-xs px-2 py-1 border border-deep-600/30">Clear</button>
    </div>
    <div class="h-80 overflow-y-auto terminal-surface p-3 font-mono text-xs space-y-1">
      {#each actionLog as entry, i (i)}
        <div class="text-deep-300">{entry}</div>
      {:else}
        <p class="text-deep-400 text-center py-8">No actions yet in this session</p>
      {/each}
    </div>
  </div>
  <div class="card">
    <div class="flex items-center justify-between mb-2">
      <h2 class="card-header !mb-0">History (audit)</h2>
      {#if canAudit}<button onclick={load} class="btn-ghost p-1.5" title="Refresh"><RotateCw size={12} /></button>{/if}
    </div>
    {#if !canAudit}
      <p class="text-xs text-deep-400 py-8 text-center">Needs the Audit permission.</p>
    {:else if error}
      <p class="text-xs text-red-300">{error}</p>
    {:else}
      <div class="h-80 overflow-y-auto">
        <table class="w-full text-xs">
          <tbody>
            {#each history as e, i (i)}
              <tr class="border-b border-deep-700/20 align-top">
                <td class="py-1.5 pr-2 text-deep-400 whitespace-nowrap font-mono text-[10px]" title={new Date(e.timestamp).toLocaleString()}>{new Date(e.timestamp).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</td>
                <td class="py-1.5 pr-2 text-bedrock-300 whitespace-nowrap">{e.username}</td>
                <td class="py-1.5 pr-2 text-strong capitalize whitespace-nowrap">{label(e.action)}</td>
                <td class="py-1.5 text-deep-300 break-all">{e.detail}</td>
              </tr>
            {:else}
              <tr><td class="py-8 text-center text-deep-400">No recorded pack changes</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>
