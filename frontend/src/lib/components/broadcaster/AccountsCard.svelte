<script lang="ts">
  /**
   * Broadcaster accounts card: the instance's Xbox accounts with the
   * confirm-by-typing remove button and the add-account form. The pending
   * account id is two-way bound so the page's add/remove action sees the
   * same value the form shows.
   */
  import { Plus, Trash2, UserRound } from '@lucide/svelte';
  import type { BroadcasterAccount } from '$types/index';

  let { accounts, canManage, newAccount = $bindable(''), onaction }: {
    accounts: BroadcasterAccount[];
    canManage: boolean;
    newAccount: string;
    onaction: (kind: 'add' | 'remove') => void;
  } = $props();
</script>

<div class="card">
  <h2 class="card-header flex items-center gap-2"><UserRound size={13} /> Accounts</h2>
  <div class="space-y-2">
    {#each accounts as account (account.id)}
      <div class="flex items-center justify-between px-3 py-2 border border-deep-600/30 bg-deep-900/40">
        <div class="flex items-center gap-2">
          <span class="font-mono text-xs text-strong">{account.id}</span>
          {#if account.main}<span class="text-[9px] uppercase tracking-wider bg-bedrock-500/20 text-bedrock-300 px-1.5 py-0.5">main</span>{/if}
        </div>
        {#if canManage && !account.main}
          <button onclick={() => onaction('remove')} disabled={!newAccount.trim() || newAccount.trim() !== account.id}
                  class="btn-ghost p-1.5 text-red-400" title={`Type "${account.id}" above to enable removal`}>
            <Trash2 size={12} />
          </button>
        {/if}
      </div>
    {/each}
    {#if canManage}
      <div class="flex gap-2 pt-1">
        <input type="text" bind:value={newAccount} placeholder="account id"
               class="input flex-1 text-xs font-mono" onkeydown={(e) => e.key === 'Enter' && onaction('add')} />
        <button onclick={() => onaction('add')} disabled={!newAccount.trim()}
                class="btn-primary flex items-center gap-1 text-xs px-3">
          <Plus size={13} /> Add
        </button>
      </div>
      <p class="text-[10px] text-deep-400">Extra accounts become sub-sessions; each triggers its own sign-in code.</p>
    {/if}
  </div>
</div>
