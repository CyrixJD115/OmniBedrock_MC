<!-- Source badge: CurseForge store install vs manual upload. -->
<script lang="ts">
  import type { Addon } from '$types/index';

  let { addon }: { addon: Addon } = $props();
</script>

{#if addon.source === 'curseforge'}
  <span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-bedrock-500/15 text-bedrock-300 border border-bedrock-500/30"
        title="Installed from the CurseForge Store">CF</span>
{:else}
  <span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-deep-700/40 text-deep-400 border border-deep-600/40"
        title="Manual upload">MAN</span>
{/if}
