<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { formatBytes } from '$lib/utils';
  import { activeInstance } from '$stores/instances';
  import { userPermissions } from '$stores/auth';
  import { backupEvents, backupRunning, backupSettings, connectBackupWs, disconnectBackupWs } from '$stores/backup';
  import CommandEditor from '$lib/components/backups/CommandEditor.svelte';
  import BackupTable from '$lib/components/backups/BackupTable.svelte';
  import CreateBackupCard from '$lib/components/backups/CreateBackupCard.svelte';
  import ScheduleCard from '$lib/components/backups/ScheduleCard.svelte';
  import TrashModal from '$lib/components/backups/TrashModal.svelte';
  import RestoreModal from '$lib/components/backups/RestoreModal.svelte';
  import type { Backup, CommandEntry } from '$types/index';
  import { Archive, CalendarClock, ListChecks, Plus, RefreshCw, ScrollText, TerminalSquare, Trash2 } from '@lucide/svelte';

  type Tab = 'backups' | 'create' | 'schedule' | 'hooks' | 'activity';
  let tab = $state<Tab>('backups');
  let worlds = $state<string[]>([]);
  let backups = $state<Backup[]>([]);
  let filterWorld = $state('');
  let filterKind = $state<'all' | 'auto' | 'manual'>('all');
  let loading = $state(true);
  let trashOpen = $state(false);
  let restoreTarget = $state<Backup | null>(null);
  let restoring = $state(false);
  let verifying = $state<string | null>(null);
  let cmdBefore = $state<CommandEntry[]>([]);
  let cmdAfter = $state<CommandEntry[]>([]);
  let logFilter = $state('');

  const can = $derived({
    create: $userPermissions.includes('BACKUPS_CREATE'),
    restore: $userPermissions.includes('BACKUPS_RESTORE'),
    del: $userPermissions.includes('BACKUPS_DELETE'),
    settings: $userPermissions.includes('SETTINGS_EDIT'),
  });

  // Everything here is instance-scoped; the epoch drops stale responses.
  let epoch = 0;
  $effect(() => {
    $activeInstance?.id;
    const mine = ++epoch;
    void loadAll(mine);
  });
  onDestroy(() => disconnectBackupWs());

  async function loadAll(mine: number) {
    loading = true;
    try {
      const [w, s] = await Promise.all([api.listWorlds(), api.getBackupSettings()]);
      if (mine !== epoch) return;
      worlds = w;
      backupSettings.set(s);
      cmdBefore = s.pre_post?.before ?? [];
      cmdAfter = s.pre_post?.after ?? [];
      await loadBackups();
    } catch (e: any) {
      if (mine === epoch) addToast(`Failed to load backups: ${e.message}`, 'error');
    }
    if (mine !== epoch) return;
    loading = false;
    connectBackupWs();
  }

  async function loadBackups() {
    try { backups = (await api.listBackups(filterWorld || undefined)).backups; } catch { /* keep the last list */ }
  }

  // Refresh the list when a job finishes.
  let lastDone = 0;
  $effect(() => {
    const events = $backupEvents;
    const done = events.filter((e) => e.type === 'done').length;
    if (done !== lastDone) { lastDone = done; void loadBackups(); }
  });

  const shown = $derived(backups.filter((b) => filterKind === 'all' || b.kind === filterKind));
  const totalBytes = $derived(backups.reduce((a, b) => a + b.size_bytes, 0));
  const newest = $derived(backups[0] ?? null);
  const unverified = $derived(backups.filter((b) => b.meta?.verified === false).length);

  async function del(b: Backup) {
    try {
      await api.deleteBackup(b.world, b.filename);
      await loadBackups();
      addToast('Moved to trash', 'info', 6000, {
        label: 'Undo',
        callback: async () => {
          try { await api.restoreBackup(b.world, b.filename); await loadBackups(); } catch (e: any) { addToast(`Undo failed: ${e.message}`, 'error'); }
        },
      });
    } catch (e: any) { addToast(`Delete failed: ${e.message}`, 'error'); }
  }

  async function verify(b: Backup) {
    verifying = b.filename;
    try {
      const r = await api.verifyBackup(b.world, b.filename);
      addToast(r.ok ? `Verified: ${r.entries} entries OK (${r.elapsed_ms} ms)` : `Verification failed: ${r.error}`, r.ok ? 'success' : 'error');
      await loadBackups();
    } catch (e: any) { addToast(`Verify failed: ${e.message}`, 'error'); }
    verifying = null;
  }

  async function download(b: Backup) {
    try { await api.downloadBackup(b.world, b.filename); } catch (e: any) { addToast(`Download failed: ${e.message}`, 'error'); }
  }

  async function restore(startAfter: boolean) {
    if (!restoreTarget) return;
    restoring = true;
    try {
      const r = await api.restoreToWorld(restoreTarget.world, restoreTarget.filename, startAfter);
      addToast(r.message, 'success');
      restoreTarget = null;
    } catch (e: any) { addToast(`Restore failed: ${e.message.replace(/^API Error \d+: /, '')}`, 'error'); }
    restoring = false;
  }

  async function saveHooks() {
    try {
      await api.updateBackupSettings({ pre_post: { before: cmdBefore, after: cmdAfter } });
      addToast('Hooks saved', 'success');
    } catch (e: any) { addToast(`Failed to save: ${e.message.replace(/^API Error \d+: /, '')}`, 'error'); }
  }

  const filteredEvents = $derived($backupEvents.filter((e) => !logFilter || e.type === logFilter));

  const tabs = [
    { id: 'backups' as const, label: 'Backups', icon: Archive },
    { id: 'create' as const, label: 'New backup', icon: Plus },
    { id: 'schedule' as const, label: 'Schedule', icon: CalendarClock },
    { id: 'hooks' as const, label: 'Hooks', icon: TerminalSquare },
    { id: 'activity' as const, label: 'Activity', icon: ScrollText },
  ];
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Backups</h1>
      <div class="pixel-divider mt-2 w-32"></div>
    </div>
    {#if $backupRunning}
      <span class="flex items-center gap-1.5 text-bedrock-300 text-xs uppercase tracking-wider"><span class="w-2 h-2 rounded-full bg-bedrock-500 animate-pulse"></span> Backup running</span>
    {/if}
  </div>

  <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
    <div class="card !p-3"><p class="text-[10px] uppercase tracking-wider text-deep-400">Backups</p><p class="mt-1 text-xl font-bold font-mono text-strong">{backups.length}</p></div>
    <div class="card !p-3"><p class="text-[10px] uppercase tracking-wider text-deep-400">Storage used</p><p class="mt-1 text-xl font-bold font-mono text-strong">{formatBytes(totalBytes)}</p></div>
    <div class="card !p-3"><p class="text-[10px] uppercase tracking-wider text-deep-400">Newest</p><p class="mt-1 text-sm font-semibold text-strong truncate">{newest ? new Date(newest.modified).toLocaleString() : 'none'}</p></div>
    <div class="card !p-3"><p class="text-[10px] uppercase tracking-wider text-deep-400">Integrity</p>
      <p class="mt-1 text-sm font-semibold {unverified ? 'text-red-300' : 'text-teal-300'}">{unverified ? `${unverified} failed verification` : 'No failures'}</p></div>
  </div>

  <div class="flex flex-wrap gap-1 border-b border-deep-600/30 pb-px" role="tablist">
    {#each tabs as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}
              class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t flex items-center gap-1.5
                     {tab === t.id ? 'bg-deep-800/60 text-strong border-b-2 border-bedrock-500' : 'text-deep-400 hover:text-deep-200'}">
        <t.icon size={12} /> {t.label}
      </button>
    {/each}
  </div>

  {#if tab === 'backups'}
    <div class="card">
      <div class="flex flex-wrap items-center gap-2 mb-3">
        <select bind:value={filterWorld} onchange={loadBackups} class="input w-44 text-xs py-1.5" aria-label="World">
          <option value="">All worlds</option>
          {#each worlds as w (w)}<option value={w}>{w}</option>{/each}
        </select>
        <select bind:value={filterKind} class="input w-32 text-xs py-1.5" aria-label="Kind">
          <option value="all">All kinds</option><option value="manual">Manual</option><option value="auto">Scheduled</option>
        </select>
        <button onclick={loadBackups} class="btn-ghost p-2" title="Refresh"><RefreshCw size={13} /></button>
        <span class="flex-1"></span>
        <button onclick={() => (trashOpen = true)} class="btn-ghost text-xs px-3 py-1.5 border border-deep-600/30 flex items-center gap-1"><Trash2 size={12} /> Trash</button>
        {#if can.create}<button onclick={() => (tab = 'create')} class="btn-primary text-xs px-3 py-1.5 flex items-center gap-1"><Plus size={12} /> New backup</button>{/if}
      </div>
      {#if loading && backups.length === 0}
        <div class="space-y-2">{#each [0, 1, 2] as j (j)}<div class="h-10 bg-deep-800/40 rounded animate-pulse"></div>{/each}</div>
      {:else}
        <BackupTable backups={shown} canRestore={can.restore} canDelete={can.del} {verifying}
          ondownload={download} onrestore={(b) => (restoreTarget = b)} ondelete={del} onverify={verify} />
      {/if}
    </div>
  {:else if tab === 'create'}
    <CreateBackupCard {worlds} canCreate={can.create} oncreated={loadBackups} />
  {:else if tab === 'schedule'}
    <ScheduleCard {worlds} canEdit={can.settings} instanceId={$activeInstance?.id ?? 'default'} />
  {:else if tab === 'hooks'}
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
      <div class="card lg:col-span-2">
        <div class="flex items-center justify-between mb-3">
          <h2 class="card-header !mb-0">Before / after hooks</h2>
          {#if can.settings}<button onclick={saveHooks} class="btn-primary text-xs py-1.5 px-4">Save hooks</button>{/if}
        </div>
        <CommandEditor label="" before={cmdBefore} after={cmdAfter} onchange={(b, a) => { cmdBefore = b; cmdAfter = a; }} />
      </div>
      <div class="card text-xs space-y-2 text-deep-300 leading-relaxed">
        <h2 class="card-header flex items-center gap-2"><ListChecks size={14} /> Optional</h2>
        <p>Hooks are optional. When the server is running, the panel itself sends <code>save hold</code>, waits for <code>save query</code>, copies the held snapshot, and always sends <code>save resume</code>, even if the backup fails.</p>
        <p>Use hooks for extras: announce the backup in chat (<code>send</code> → <code>say Backing up…</code>), wait, or run a host script (<code>command</code>, if enabled on this host).</p>
        <p class="text-deep-400">Existing <code>save hold</code> / <code>save resume</code> hooks keep working and are not sent twice.</p>
      </div>
    </div>
  {:else if tab === 'activity'}
    <div class="card">
      <div class="flex items-center gap-2 mb-3">
        <select bind:value={logFilter} class="input text-xs py-1.5 w-36" aria-label="Event type">
          <option value="">All events</option><option value="status">Status</option><option value="progress">Progress</option>
          <option value="output">Output</option><option value="done">Done</option><option value="error">Errors</option>
        </select>
        <span class="flex-1"></span>
        <button onclick={() => backupEvents.set([])} class="btn-ghost text-xs px-2 py-1 border border-deep-600/30">Clear</button>
      </div>
      <div class="h-96 overflow-y-auto terminal-surface p-3 font-mono text-xs space-y-0.5">
        {#each filteredEvents as ev, i (i)}
          {@const e = ev as Record<string, any>}
          <div class="flex gap-2">
            <span class="shrink-0 w-10 text-right {e.type === 'error' ? 'text-red-400' : e.type === 'done' ? 'text-teal-300' : e.type === 'progress' ? 'text-bedrock-300' : 'text-deep-400'}">{String(e.type).toUpperCase().slice(0, 4)}</span>
            {#if e.percent !== undefined}<span class="text-deep-400 shrink-0 w-10">{e.percent}%</span>{/if}
            <span class="text-deep-200 break-all">{e.line ?? e.message ?? e.phase ?? ''}</span>
          </div>
        {:else}
          <p class="text-deep-400 text-center py-8">No events yet. Run a backup to see its steps here.</p>
        {/each}
      </div>
    </div>
  {/if}
</div>

<TrashModal bind:open={trashOpen} world={filterWorld} canDelete={can.del} onchanged={loadBackups} />
<RestoreModal target={restoreTarget} busy={restoring} onconfirm={restore} oncancel={() => (restoreTarget = null)} />
