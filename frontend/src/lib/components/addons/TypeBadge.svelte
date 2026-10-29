<!-- BP/RP type badge for a pack row. -->
<script lang="ts">
  import { kindOf } from '$lib/addons/display';
  import type { Addon } from '$types/index';

  let { addon }: { addon: Addon } = $props();
</script>

<span class="shrink-0 px-1 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm border
             {kindOf(addon) === 'bp'
               ? 'bg-bedrock-500/15 text-bedrock-300 border-bedrock-500/30'
               : 'bg-teal-500/15 text-teal-300 border-teal-500/30'}"
      title={addon.pack_type === 'behavior_packs' ? 'Behavior pack' : 'Resource pack'}>
  {kindOf(addon) === 'bp' ? 'BP' : 'RP'}
</span>
