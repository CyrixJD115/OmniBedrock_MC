<!-- Pack icon with pixel-crisp rendering; falls back to a kind-colored glyph. -->
<script lang="ts">
  import { Package } from '@lucide/svelte';
  import { kindOf, packColorClass } from '$lib/addons/display';
  import type { Addon } from '$types/index';

  let { icon, pack, size = 'w-10 h-10', onbroken }: {
    icon: string;
    pack: Addon;
    size?: string;
    onbroken: () => void;
  } = $props();
</script>

{#if icon}
  <img src={icon} alt="" class="pack-icon {size} rounded object-cover shrink-0 bg-deep-800 border border-deep-700/60"
       onerror={onbroken} />
{:else}
  <div class="{size} rounded bg-deep-800/80 border border-deep-700/60 flex items-center justify-center shrink-0">
    <Package size={16} class={packColorClass(kindOf(pack))} />
  </div>
{/if}

<style>
  /* Minecraft pack icons are pixel art — keep them crisp when scaled up. */
  .pack-icon {
    image-rendering: pixelated;
  }
</style>
