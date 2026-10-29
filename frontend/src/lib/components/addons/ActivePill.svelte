<!-- "Active" pill shown when the pack is in the world's order. -->
<script lang="ts">
  let { active }: { active: boolean } = $props();
</script>

{#if active}
  <span class="shrink-0 px-1.5 py-px text-[8px] font-bold uppercase tracking-wider rounded-sm bg-green-500/15 text-green-400 border border-green-500/30"
        title="Active in this world's pack order">Active</span>
{/if}
