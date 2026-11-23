<script lang="ts">
  /** Reason / message / level prompt for one player action. */
  import type { PlayerAction } from '$types/players';
  import { X } from '@lucide/svelte';

  let { action, target, busy = false, onsubmit, oncancel }: {
    action: PlayerAction | null;
    target: string;
    busy?: boolean;
    onsubmit: (extra: { reason?: string; level?: string }) => void;
    oncancel: () => void;
  } = $props();

  let text = $state('');
  let level = $state('member');

  $effect(() => {
    action;
    text = '';
    level = 'member';
  });

  const meta = $derived.by(() => {
    switch (action) {
      case 'kick': return { title: `Kick ${target}`, label: 'Reason (optional, shown to the player)', button: 'Kick', danger: true, needsText: false };
      case 'ban': return { title: `Ban ${target}`, label: 'Reason (optional)', button: 'Ban', danger: true, needsText: false };
      case 'tell': return { title: `Message ${target}`, label: 'Message (sent with /tell)', button: 'Send', danger: false, needsText: true };
      case 'permission': return { title: `Permission level for ${target}`, label: '', button: 'Apply', danger: false, needsText: false };
      default: return null;
    }
  });

  function submit(e: Event) {
    e.preventDefault();
    if (!meta || (meta.needsText && !text.trim())) return;
    onsubmit(action === 'permission' ? { level } : { reason: text.trim() });
  }
</script>

{#if action && meta}
  <div class="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4" role="presentation"
       onclick={(e) => { if (e.target === e.currentTarget) oncancel(); }}>
    <div class="card w-full max-w-md" role="dialog" aria-modal="true" aria-label={meta.title}>
    <form class="space-y-4" onsubmit={submit}>
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold uppercase tracking-wider text-strong">{meta.title}</h2>
        <button type="button" class="btn-ghost p-1" onclick={oncancel} aria-label="Close"><X size={14} /></button>
      </div>
      {#if action === 'permission'}
        <div class="grid grid-cols-3 gap-2">
          {#each [['visitor', 'Visitor', 'Look only'], ['member', 'Member', 'Default play'], ['operator', 'Operator', 'Commands & cheats']] as [v, label, hint] (v)}
            <label class="mc-slot p-2 text-xs cursor-pointer {level === v ? 'ring-1 ring-bedrock-400' : ''}">
              <input type="radio" name="level" value={v} bind:group={level} class="sr-only" />
              <span class="block text-strong font-semibold">{label}</span>
              <span class="block text-[10px] text-deep-400">{hint}</span>
            </label>
          {/each}
        </div>
      {:else}
        <label class="block text-xs text-deep-300">
          {meta.label}
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input w-full mt-1 text-sm" bind:value={text} maxlength="256" autofocus />
        </label>
      {/if}
      <div class="flex justify-end gap-2">
        <button type="button" class="btn-ghost text-xs px-3 py-1.5" onclick={oncancel}>Cancel</button>
        <button type="submit" disabled={busy || (meta.needsText && !text.trim())}
                class="{meta.danger ? 'btn-danger' : 'btn-primary'} text-xs px-3 py-1.5 disabled:opacity-40">{meta.button}</button>
      </div>
    </form>
    </div>
  </div>
{/if}
