<script lang="ts">
  /**
   * Broadcaster sign-in codes card: one tile per pending Microsoft device
   * code, newest first, with copy and verification-link actions.
   */
  import { Copy, ExternalLink, KeyRound } from '@lucide/svelte';
  import type { DeviceCode } from '$types/index';

  let { codes, oncopy }: {
    codes: Record<string, DeviceCode>;
    oncopy: (text: string) => void;
  } = $props();
</script>

<div class="card">
  <h2 class="card-header flex items-center gap-2"><KeyRound size={13} /> Sign-in codes</h2>
  <div class="space-y-3">
    {#if Object.keys(codes).length === 0}
      <p class="text-xs text-deep-400">
        When an account needs Microsoft authentication, the code and verification link appear here and in the console below.
      </p>
    {/if}
    {#each Object.values(codes).sort((a, b) => b.at - a.at) as code (code.account)}
      <div class="border-2 border-bedrock-500/40 bg-bedrock-950/30 p-3 space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-[10px] uppercase tracking-widest text-bedrock-300">{code.account}</span>
          <span class="text-[10px] text-deep-400">{new Date(code.at * 1000).toLocaleTimeString()}</span>
        </div>
        <div class="flex items-center gap-3">
          <code class="text-xl font-mono font-bold text-strong tracking-[0.2em]">{code.code}</code>
          <button onclick={() => oncopy(code.code)} class="btn-ghost p-1.5" title="Copy code"><Copy size={12} /></button>
        </div>
        <a href={code.url} target="_blank" rel="noreferrer"
           class="text-xs text-bedrock-400 hover:underline inline-flex items-center gap-1 break-all">
          {code.url} <ExternalLink size={10} class="shrink-0" />
        </a>
      </div>
    {/each}
  </div>
</div>
