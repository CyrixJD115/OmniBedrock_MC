<script lang="ts">
  /** Floating per-file upload progress and results for the Mods page. */
  import type { UploadQueue } from '$lib/addons/uploadQueue.svelte';
  import { fmtSize } from '$lib/addons/display';
  import { CircleCheck, CircleX, Loader2, X, Upload, Power } from '@lucide/svelte';

  let { queue, isActive, onactivate }: {
    queue: UploadQueue;
    isActive: (uuid: string) => boolean;
    onactivate: (uuid: string) => void;
  } = $props();

  let collapsed = $state(false);
  const done = $derived(queue.items.filter((i) => i.state === 'installed').length);
  const failed = $derived(queue.items.filter((i) => i.state === 'failed' || i.state === 'rejected').length);
</script>

{#if queue.items.length}
  <div class="fixed bottom-4 right-4 z-[60] w-[22rem] max-w-[calc(100vw-2rem)] card !p-0 shadow-block-lg" role="status" aria-live="polite">
    <div class="flex items-center gap-2 px-3 py-2 border-b border-deep-600/40">
      <Upload size={13} class="text-bedrock-400" />
      <button class="flex-1 text-left text-xs font-semibold uppercase tracking-wider text-strong" onclick={() => (collapsed = !collapsed)}>
        Uploads <span class="font-normal text-deep-400 normal-case tracking-normal">{done} installed{failed ? ` · ${failed} failed` : ''}{queue.busy ? ' · working…' : ''}</span>
      </button>
      {#if !queue.busy}<button class="btn-ghost p-1" onclick={() => queue.clearFinished()} aria-label="Clear finished uploads"><X size={13} /></button>{/if}
    </div>
    {#if !collapsed}
      <ul class="max-h-72 overflow-y-auto divide-y divide-deep-700/30">
        {#each queue.items as item (item.id)}
          <li class="px-3 py-2 text-xs space-y-1">
            <div class="flex items-center gap-2">
              {#if item.state === 'installed'}<CircleCheck size={13} class="text-teal-400 shrink-0" />
              {:else if item.state === 'failed' || item.state === 'rejected'}<CircleX size={13} class="text-red-400 shrink-0" />
              {:else}<Loader2 size={13} class="text-bedrock-300 shrink-0 {item.state === 'uploading' ? 'animate-spin' : ''}" />{/if}
              <span class="flex-1 min-w-0 truncate text-strong" title={item.name}>{item.name}</span>
              <span class="font-mono text-[10px] text-deep-400 shrink-0">{fmtSize(item.size)}</span>
              {#if item.state !== 'uploading' && item.state !== 'queued'}
                <button class="btn-ghost p-0.5" onclick={() => queue.dismiss(item.id)} aria-label="Dismiss {item.name}"><X size={11} /></button>
              {/if}
            </div>
            {#if item.state === 'uploading'}
              <div class="mc-progress w-full"><span style="width: {Math.round(item.progress * 100)}%"></span></div>
              <p class="text-[10px] text-deep-400">{item.progress < 1 ? `Uploading ${Math.round(item.progress * 100)}%` : 'Installing…'} into {item.world}</p>
            {:else if item.state === 'queued'}
              <p class="text-[10px] text-deep-400">Waiting…</p>
            {:else if item.state === 'installed'}
              {#each item.installed as p (p.uuid)}
                <div class="flex items-center gap-2 pl-5 text-[11px]">
                  <span class="flex-1 min-w-0 truncate text-deep-200">{p.name} <span class="text-deep-400">· {p.pack_type === 'behavior_packs' ? 'BP' : 'RP'} v{p.version.join('.')}</span></span>
                  {#if isActive(p.uuid)}
                    <span class="text-[10px] text-teal-300">active</span>
                  {:else}
                    <button class="btn-ghost text-[10px] px-1.5 py-0.5 flex items-center gap-1 text-bedrock-300" onclick={() => onactivate(p.uuid)}><Power size={10} /> Activate</button>
                  {/if}
                </div>
              {/each}
            {:else}
              <p class="text-[10px] text-red-300 pl-5">{item.message}</p>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}
