<script lang="ts">
  /** Per-instance supervisor: auto-restart, crash limit, stop/kill timeouts. */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { activeInstance } from '$stores/instances';
  import { Save } from '@lucide/svelte';

  let { canEditSettings }: { canEditSettings: boolean } = $props();
  let serverSettings = $state<Record<string, unknown>>({});

  $effect(() => {
    $activeInstance?.id;
    void (async () => {
      try { serverSettings = await api.getServerSettings(); } catch { /* silent */ }
    })();
  });

  async function saveServerSettings() {
    try {
      const r = await api.updateServerSettings(serverSettings);
      addToast(r.message || 'Saved', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }
  function num(field: string): number {
    return Number(serverSettings[field] ?? 0);
  }
  function setNum(field: string, value: number) {
    serverSettings = { ...serverSettings, [field]: value };
  }
  function toggleAutoRestart(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    serverSettings = { ...serverSettings, auto_restart: checked };
  }
</script>

  <div class="card">
    <div class="flex items-center justify-between">
      <h2 class="card-header">Supervisor</h2>
      {#if canEditSettings}
        <button onclick={saveServerSettings} class="btn-primary flex items-center gap-1.5 text-xs">
          <Save size={13} /> Save
        </button>
      {/if}
    </div>
    <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
      <label class="flex items-center justify-between gap-3 text-xs text-deep-300">
        Auto-restart
        <input type="checkbox" checked={!!serverSettings.auto_restart}
               onchange={toggleAutoRestart}
               class="accent-bedrock-500 w-4 h-4" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Restart delay (s)</span>
        <input type="number" min="1" max="3600" value={num('auto_restart_delay')}
               onchange={(e) => setNum('auto_restart_delay', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Max crashes before giving up</span>
        <input type="number" min="1" max="100" value={num('max_crashes')}
               onchange={(e) => setNum('max_crashes', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Graceful stop timeout (s)</span>
        <input type="number" min="1" max="600" value={num('stop_timeout')}
               onchange={(e) => setNum('stop_timeout', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-xs text-deep-400">Kill timeout (s)</span>
        <input type="number" min="1" max="600" value={num('kill_timeout')}
               onchange={(e) => setNum('kill_timeout', Number((e.target as HTMLInputElement).value))}
               class="input w-full text-xs font-mono" />
      </label>
    </div>
  </div>
