<script lang="ts">
  /** Club screenshot per broadcast account: preview and upload. */
  import { api, rawHeaders } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { onDestroy } from 'svelte';
  import { Upload } from '@lucide/svelte';

  let { canManage, instanceId, accounts }: {
    canManage: boolean;
    instanceId: string;
    accounts: { id: string; main?: boolean }[];
  } = $props();

  let screenshotAccount = $state('main');
  let previewUrl = $state('');
  let screenshotInput = $state<HTMLInputElement | undefined>(undefined);

  async function loadPreview(account: string, instance: string) {
    if (!instance) return;
    try {
      const r = await fetch(
        `/api/v1/broadcaster/screenshot?account=${encodeURIComponent(account)}&instance=${encodeURIComponent(instance)}`,
        { headers: rawHeaders() },
      );
      if (instance !== instanceId) return;
      if (!r.ok) throw new Error(String(r.status));
      const url = URL.createObjectURL(await r.blob());
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      previewUrl = url;
    } catch {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      previewUrl = '';
    }
  }

  // Keyed on the account and the instance id only.
  $effect(() => {
    void loadPreview(screenshotAccount, instanceId);
  });

  onDestroy(() => { if (previewUrl) URL.revokeObjectURL(previewUrl); });

  async function uploadScreenshot(event: Event) {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const r = await api.uploadBroadcasterScreenshot(screenshotAccount, file);
      addToast(r.message || 'Uploaded', 'success');
      await loadPreview(screenshotAccount, instanceId);
    } catch (e: any) { addToast(`Upload failed: ${e.message}`, 'error'); }
    (event.target as HTMLInputElement).value = '';
  }
</script>

    <!-- Screenshot -->
    <div class="card">
      <h2 class="card-header">Screenshot</h2>
      <div class="space-y-3">
        <select bind:value={screenshotAccount} class="input w-full text-xs">
          {#each accounts.length ? accounts : [{ id: 'main', main: true }] as account (account.id)}
            <option value={account.id}>{account.id}</option>
          {/each}
        </select>
        <div class="relative border-2 border-deep-600/40 bg-black/40 aspect-video flex items-center justify-center overflow-hidden">
          {#if previewUrl}
            <img src={previewUrl} alt="screenshot preview" class="w-full h-full object-cover" />
          {:else}
            <span class="text-[10px] text-deep-400 uppercase tracking-wider">no screenshot uploaded</span>
          {/if}
        </div>
        {#if canManage}
          <input type="file" accept="image/jpeg,image/png" bind:this={screenshotInput} class="hidden" onchange={uploadScreenshot} />
          <button onclick={() => screenshotInput?.click()}
                  class="btn-ghost w-full flex items-center justify-center gap-2 text-xs">
            <Upload size={13} /> Upload JPEG/PNG (1200×675 recommended)
          </button>
        {/if}
        <p class="text-[10px] text-deep-400">Xbox Live may take a few minutes to refresh the club image.</p>
      </div>
    </div>
