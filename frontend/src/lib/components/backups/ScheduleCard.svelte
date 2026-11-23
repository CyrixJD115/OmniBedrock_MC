<script lang="ts">
  /** Scheduled backups: cadence, worlds, retention, and live scheduler status. */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import type { SchedulerConfig } from '$types/index';
  import { CalendarClock, Save } from '@lucide/svelte';

  let { worlds, canEdit, instanceId }: { worlds: string[]; canEdit: boolean; instanceId: string } = $props();

  let cfg = $state<SchedulerConfig | null>(null);
  let saving = $state(false);
  let dirty = $state(false);

  async function load() {
    try {
      const c = await api.getSchedulerConfig();
      cfg = {
        ...c,
        interval_minutes: c.interval_minutes ?? 30,
        keep_count: c.keep_count ?? 10,
        keep_days: c.keep_days ?? 0,
        trash_days: c.trash_days ?? 7,
        skip_if_idle: c.skip_if_idle ?? false,
        verify: c.verify ?? true,
        // Older settings could hold full_backup=false with no include list,
        // which produced empty archives; scheduled backups are whole worlds.
        full_backup: true,
        compression: c.compression === 'store' ? 'store' : 'deflate',
        worlds: c.worlds ?? [],
      };
      dirty = false;
    } catch (e: any) {
      addToast(`Failed to load schedule: ${e.message}`, 'error');
    }
  }

  $effect(() => {
    instanceId;
    void load();
    const t = setInterval(() => { if (!dirty) void load(); }, 30_000);
    return () => clearInterval(t);
  });

  function touch() { dirty = true; }

  function toggleWorld(w: string) {
    if (!cfg) return;
    const list = cfg.worlds ?? [];
    cfg.worlds = list.includes(w) ? list.filter((x) => x !== w) : [...list, w];
    touch();
  }

  async function save() {
    if (!cfg) return;
    saving = true;
    try {
      const { status: _status, ...body } = cfg;
      await api.updateScheduler(body as SchedulerConfig);
      addToast('Schedule saved', 'success');
      await load();
    } catch (e: any) {
      addToast(`Save failed: ${e.message.replace(/^API Error \d+: /, '')}`, 'error');
    }
    saving = false;
  }

  function rel(iso: string | null | undefined): string {
    if (!iso) return '—';
    const d = new Date(iso);
    const mins = Math.round((d.getTime() - Date.now()) / 60_000);
    const abs = Math.abs(mins);
    const span = abs < 60 ? `${abs} min` : abs < 1440 ? `${Math.round(abs / 60)} h` : `${Math.round(abs / 1440)} d`;
    return `${d.toLocaleString()} (${mins >= 0 ? 'in ' + span : span + ' ago'})`;
  }
</script>

{#if cfg}
  <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
    <div class="card lg:col-span-2 space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="card-header !mb-0">Schedule</h2>
        <label class="flex items-center gap-2 text-xs cursor-pointer select-none">
          <input type="checkbox" bind:checked={cfg.enabled} onchange={touch} disabled={!canEdit} class="accent-bedrock-500" />
          <span class="uppercase tracking-wider font-semibold {cfg.enabled ? 'text-teal-300' : 'text-deep-300'}">{cfg.enabled ? 'Enabled' : 'Disabled'}</span>
        </label>
      </div>
      <div class="grid grid-cols-2 sm:grid-cols-4 gap-3 text-xs">
        <label class="block"><span class="block text-deep-400 uppercase tracking-wider mb-1">Every (min)</span>
          <input type="number" min="1" max="525600" bind:value={cfg.interval_minutes} oninput={touch} disabled={!canEdit} class="input w-full text-xs py-1.5" /></label>
        <label class="block"><span class="block text-deep-400 uppercase tracking-wider mb-1">Keep newest</span>
          <input type="number" min="1" max="10000" bind:value={cfg.keep_count} oninput={touch} disabled={!canEdit} class="input w-full text-xs py-1.5" /></label>
        <label class="block"><span class="block text-deep-400 uppercase tracking-wider mb-1" title="0 = off; the newest backup is always kept">Max age (days)</span>
          <input type="number" min="0" max="3650" bind:value={cfg.keep_days} oninput={touch} disabled={!canEdit} class="input w-full text-xs py-1.5" /></label>
        <label class="block"><span class="block text-deep-400 uppercase tracking-wider mb-1" title="Trashed backups are deleted for good after this many days (0 = never)">Empty trash after</span>
          <input type="number" min="0" max="3650" bind:value={cfg.trash_days} oninput={touch} disabled={!canEdit} class="input w-full text-xs py-1.5" /></label>
      </div>
      <div class="flex flex-wrap gap-x-5 gap-y-2 text-xs text-deep-300">
        <label class="flex items-center gap-2 cursor-pointer"><input type="checkbox" bind:checked={cfg.verify} onchange={touch} disabled={!canEdit} class="accent-bedrock-500" /> Verify each backup</label>
        <label class="flex items-center gap-2 cursor-pointer" title="When the server is stopped and no world file changed since the last backup, skip that world">
          <input type="checkbox" bind:checked={cfg.skip_if_idle} onchange={touch} disabled={!canEdit} class="accent-bedrock-500" /> Skip unchanged worlds</label>
        <label class="flex items-center gap-2">Compression
          <select bind:value={cfg.compression} onchange={touch} disabled={!canEdit} class="input text-xs py-1">
            <option value="deflate">Compressed</option><option value="store">Stored</option>
          </select></label>
      </div>
      <div>
        <span class="block text-deep-400 text-xs uppercase tracking-wider mb-2">Worlds <span class="normal-case tracking-normal">(none selected = every world)</span></span>
        <div class="flex flex-wrap gap-2">
          {#each worlds as w (w)}
            <label class="flex items-center gap-1.5 text-xs cursor-pointer px-2 py-1 mc-slot {cfg.worlds?.includes(w) ? 'ring-1 ring-bedrock-400/60' : ''}">
              <input type="checkbox" checked={cfg.worlds?.includes(w)} onchange={() => toggleWorld(w)} disabled={!canEdit} class="accent-bedrock-500" /> {w}
            </label>
          {/each}
        </div>
      </div>
      {#if canEdit}
        <div class="flex items-center gap-3">
          <button onclick={save} disabled={saving || !dirty} class="btn-primary text-xs py-1.5 px-4 flex items-center gap-1.5 disabled:opacity-40"><Save size={12} /> Save schedule</button>
          {#if dirty}<span class="text-[10px] text-yellow-300 uppercase tracking-wider">Unsaved changes</span>{/if}
        </div>
      {/if}
    </div>

    <div class="card text-xs space-y-3">
      <h2 class="card-header flex items-center gap-2"><CalendarClock size={14} /> Status</h2>
      <div class="space-y-2">
        <div class="flex justify-between gap-2"><span class="text-deep-400">State</span><span class={cfg.enabled ? 'text-teal-300' : 'text-deep-300'}>{cfg.enabled ? (cfg.status?.deferred ? 'Waiting for another operation' : 'Active') : 'Disabled'}</span></div>
        <div><span class="text-deep-400 block">Next run</span><span class="text-strong">{cfg.enabled ? rel(cfg.status?.next_run) : '—'}</span></div>
        <div><span class="text-deep-400 block">Last run</span><span class="text-strong">{rel(cfg.status?.last_run)}</span></div>
        {#if cfg.status?.last_result}<div><span class="text-deep-400 block">Last result</span><span class={cfg.status.last_result.startsWith('failed') ? 'text-red-300' : 'text-deep-200'}>{cfg.status.last_result}</span></div>{/if}
      </div>
      <p class="text-[11px] text-deep-400 leading-relaxed">
        The schedule continues from the last run, so restarting the panel or editing settings does not postpone it.
        Scheduled archives beyond "keep newest" or older than "max age" go to the trash; manual backups are never removed automatically.
      </p>
    </div>
  </div>
{:else}
  <div class="card h-40 animate-pulse"></div>
{/if}
