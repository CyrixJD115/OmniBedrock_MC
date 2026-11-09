<!-- FILES: the instance-scoped file explorer. The old panel-level
     "Config Files" editor was removed — it read the panel data dir, which
     the security denylist had already emptied; the explorer plus the
     Properties tab cover everything it used to do. The page itself is
     gated by FILES_VIEW (sidebar); write actions gate again on
     FILES_WRITE server-side. -->
<script lang="ts">
  import { userPermissions } from '$stores/auth';
  import { isServerRunning } from '$stores/index';
  import { activeInstanceId } from '$stores/instances';
  import { FolderTree } from '@lucide/svelte';
  import FileExplorer from '$lib/components/files/FileExplorer.svelte';

  let canWrite = $derived($userPermissions.includes('FILES_WRITE'));
</script>

<div class="space-y-4">
  <div>
    <h1 class="text-lg font-bold text-strong uppercase tracking-widest flex items-center gap-2">
      <FolderTree size={18} class="text-bedrock-400" /> Files
    </h1>
    <div class="pixel-divider mt-2 w-28"></div>
  </div>

  <FileExplorer
    instanceId={$activeInstanceId ?? 'default'}
    canWrite={canWrite}
    serverRunning={$isServerRunning}
  />
</div>
