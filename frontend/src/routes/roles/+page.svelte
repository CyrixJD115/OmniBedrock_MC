<!-- The Users & Roles page hosts both panels now; this route keeps old
     links, bookmarks, and muscle memory alive. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  onMount(() => {
    void goto('/users#roles', { replaceState: true });
  });
</script>
