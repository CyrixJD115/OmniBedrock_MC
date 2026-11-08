<!-- Instance file explorer: confined browsing of the instance tree with
     upload/mkdir/rename/delete and a small text editor. Write actions are
     additionally enforced server-side (FILES_WRITE); hiding them here is
     only UX. -->
<script lang="ts">
  import { Check, Download, FolderPlus, Pencil, Trash2, Upload, RotateCw, Folder, FileText, TriangleAlert } from '@lucide/svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { baseName, breadcrumbs, joinPath, normalizePath, parentPath, isTextFile } from '$lib/files/paths';
  import { fmtDate, fmtSize } from '$lib/addons/display';
  import ConfirmDialog from '$lib/components/ui/ConfirmDialog.svelte';
  import type { ExplorerEntry, ExplorerListing } from '$types/index';

  let { instanceId, canWrite, serverRunning }: {
    instanceId: string;
    canWrite: boolean;
    serverRunning: boolean;
  } = $props();

  /** Editor cap: bigger (or binary) files are download-only. */
  const MAX_EDIT_BYTES = 1024 * 1024;

  let path = $state('');
  let pathInput = $state('');
  let listing = $state<ExplorerListing | null>(null);
  let loading = $state(true);
  let dragOver = $state(false);
  let restartNeeded = $state(false);
  let uploading = $state(false);

  // Inline editor state.
  let editPath = $state('');
  let editContent = $state('');

  // Confirm/delete + rename/mkdir modal state.
  let confirmOpen = $state(false);
  let confirmTitle = $state('');
  let confirmMessage = $state('');
  let confirmRecursive = $state(false);
  let pendingDelete = $state<ExplorerEntry | null>(null);
  let modalOpen = $state(false);
  let modalMode = $state<'mkdir' | 'rename'>('mkdir');
  let modalName = $state('');
  let modalTarget = $state('');

  let fileInput = $state<HTMLInputElement | null>(null);

  const crumbs = $derived(breadcrumbs(path));

  // Re-anchor to the root whenever the active instance changes; stale loads
  // of the previous instance are discarded via a monotonic epoch.
  let epoch = 0;
  $effect(() => {
    instanceId;
    const mine = ++epoch;
    path = '';
    pathInput = '';
    restartNeeded = false;
    closeEditor();
    void load(mine, '');
  });

  function closeEditor() {
    editPath = '';
    editContent = '';
  }

  async function load(mine: number = epoch, target?: string) {
    const dest = target ?? path;
    loading = true;
    try {
      const result = await api.listExplorer(instanceId, dest);
      if (mine !== epoch) return;
      listing = result;
      path = dest;
      pathInput = dest;
    } catch (e: any) {
      if (mine !== epoch) return;
      addToast(`Failed to list: ${e.message}`, 'error');
      if (dest !== '') {
        // The directory may have vanished (deleted elsewhere); fall back up.
        const parent = parentPath(dest);
        const result = await api.listExplorer(instanceId, parent).catch(() => null);
        if (result && mine === epoch) {
          listing = result;
          path = parent;
          pathInput = parent;
        }
      }
    } finally {
      if (mine === epoch) loading = false;
    }
  }

  function jump(input: string) {
    const norm = normalizePath(input);
    if (norm === null) {
      addToast("Path cannot contain '..'", 'error');
      return;
    }
    void load(epoch, norm);
  }

  function open(entry: ExplorerEntry) {
    const target = joinPath(path, entry.name);
    if (entry.is_dir) {
      void load(epoch, target);
      return;
    }
    if (!isTextFile(entry.name)) {
      addToast(`${entry.name} is not a text file — use download`, 'info');
      return;
    }
    if (entry.size > MAX_EDIT_BYTES) {
      addToast(`${entry.name} is over 1 MiB — use download instead`, 'info');
      return;
    }
    void (async () => {
      try {
        const text = await api.readExplorerFile(instanceId, target);
        if (text.includes('\0')) {
          addToast(`${entry.name} looks binary — use download instead`, 'info');
          return;
        }
        editPath = target;
        editContent = text;
      } catch (e: any) {
        addToast(`Failed to open: ${e.message}`, 'error');
      }
    })();
  }

  async function saveEditor() {
    if (!editPath) return;
    try {
      const res = await api.writeExplorerFile(instanceId, editPath, editContent);
      addToast(res.message, 'success');
      if (res.restart_required) restartNeeded = true;
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
    }
  }

  async function upload(files: FileList | File[]) {
    if (!canWrite) return;
    uploading = true;
    try {
      for (const file of Array.from(files)) {
        try {
          const res = await api.uploadExplorerFile(instanceId, path, file);
          addToast(res.message, 'success');
          if (res.restart_required) restartNeeded = true;
        } catch (e: any) {
          addToast(`${file.name}: ${e.message}`, 'error');
        }
      }
    } finally {
      uploading = false;
      void load();
    }
  }

  function askDelete(entry: ExplorerEntry) {
    pendingDelete = entry;
    confirmRecursive = false;
    confirmTitle = entry.is_dir ? 'Delete folder' : 'Delete file';
    confirmMessage = entry.is_dir
      ? `Delete '${entry.name}'? Empty folders only; non-empty ones need an extra confirmation.`
      : `Delete '${entry.name}'? This cannot be undone.`;
    confirmOpen = true;
  }

  async function doDelete() {
    const entry = pendingDelete;
    confirmOpen = false;
    if (!entry) return;
    const target = joinPath(path, entry.name);
    try {
      const res = await api.deleteExplorer(instanceId, target, confirmRecursive);
      addToast(res.message, 'success');
      if (res.restart_required) restartNeeded = true;
      if (editPath === target || editPath.startsWith(`${target}/`)) closeEditor();
    } catch (e: any) {
      const message: string = e.message ?? '';
      if (entry.is_dir && message.includes('not empty')) {
        // Escalate once: explicit recursive+confirm is required by design.
        if (window.confirm(`'${entry.name}' is not empty — delete it with ALL contents?`)) {
          confirmRecursive = true;
          pendingDelete = entry;
          await doDelete();
        }
        return;
      }
      addToast(`Delete failed: ${message}`, 'error');
    } finally {
      pendingDelete = null;
      void load();
    }
  }

  function askMkdir() {
    modalMode = 'mkdir';
    modalName = '';
    modalTarget = '';
    modalOpen = true;
  }

  function askRename(entry: ExplorerEntry) {
    modalMode = 'rename';
    modalName = entry.name;
    modalTarget = joinPath(path, entry.name);
    modalOpen = true;
  }

  async function submitModal(e: SubmitEvent) {
    e.preventDefault();
    modalOpen = false;
    try {
      if (modalMode === 'mkdir') {
        const name = normalizePath(modalName);
        if (!name) throw new Error("Folder name cannot contain '..'");
        const res = await api.mkdirExplorer(instanceId, joinPath(path, name));
        addToast(res.message, 'success');
      } else {
        const name = normalizePath(modalName);
        if (!name) throw new Error("Name cannot contain '..'");
        const res = await api.renameExplorer(instanceId, modalTarget, joinPath(path, name));
        addToast(res.message, 'success');
        if (res.restart_required) restartNeeded = true;
        if (editPath === modalTarget) editPath = joinPath(path, name);
      }
    } catch (err: any) {
      addToast(`${modalMode === 'mkdir' ? 'Create' : 'Rename'} failed: ${err.message}`, 'error');
    }
    void load();
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    if (!canWrite) return;
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) void upload(files);
  }
</script>

<div class="space-y-3">
  <!-- toolbar: breadcrumbs / path jump / actions -->
  <div class="flex flex-wrap items-center gap-2">
    <div class="flex items-center gap-1 flex-1 min-w-0 overflow-x-auto">
      {#each crumbs as crumb, i}
        {#if i > 0}<span class="text-deep-500 text-xs">/</span>{/if}
        <button
          onclick={() => void load(epoch, crumb.path)}
          class="text-xs px-1 py-0.5 truncate transition-colors {i === crumbs.length - 1
            ? 'text-strong font-semibold'
            : 'text-bedrock-400 hover:text-bedrock-300'}"
        >{crumb.name}</button>
      {/each}
    </div>
    <form
      class="flex items-center gap-1"
      onsubmit={(e) => { e.preventDefault(); jump(pathInput); }}
    >
      <input
        class="input text-xs py-1.5 w-56 font-mono"
        bind:value={pathInput}
        placeholder="Jump to path…"
        aria-label="Path"
      />
      <button type="submit" class="btn-ghost text-xs px-2 py-1.5">Go</button>
    </form>
    {#if canWrite}
      <button onclick={askMkdir} class="btn-secondary flex items-center gap-1.5 text-xs">
        <FolderPlus size={13} /> New folder
      </button>
      <button
        onclick={() => fileInput?.click()}
        disabled={uploading}
        class="btn-primary flex items-center gap-1.5 text-xs"
      >
        <Upload size={13} /> {uploading ? 'Uploading…' : 'Upload'}
      </button>
      <input
        bind:this={fileInput}
        type="file"
        multiple
        class="hidden"
        onchange={(e) => {
          const files = (e.currentTarget as HTMLInputElement).files;
          if (files && files.length > 0) void upload(files);
          (e.currentTarget as HTMLInputElement).value = '';
        }}
      />
    {/if}
    <button onclick={() => void load()} class="btn-ghost p-1.5" title="Refresh">
      <RotateCw size={14} />
    </button>
  </div>

  <!-- status line: live indicator + caps + restart banner -->
  <div class="flex flex-wrap items-center gap-3 text-[10px] uppercase tracking-wider text-deep-400">
    {#if serverRunning}
      <span class="flex items-center gap-1.5 text-teal-400">
        <span class="status-dot running"></span> Server running — files are live
      </span>
    {/if}
    {#if listing?.truncated}
      <span class="text-yellow-400">Listing truncated at 2000 entries</span>
    {/if}
  </div>
  {#if restartNeeded}
    <div class="flex items-center justify-between gap-3 border border-yellow-500/40 bg-yellow-500/5 px-3 py-2">
      <p class="flex items-center gap-2 text-xs text-yellow-300">
        <TriangleAlert size={14} class="shrink-0" />
        Changes under worlds/ or plugins/ need a server restart to take effect.
      </p>
      <button onclick={() => (restartNeeded = false)} class="text-yellow-400 hover:text-yellow-300 text-xs uppercase tracking-wider">
        <Check size={14} />
      </button>
    </div>
  {/if}

  <!-- directory table -->
  <div
    class="card"
    role="region"
    aria-label="Instance files"
    ondragover={(e) => { e.preventDefault(); if (canWrite) dragOver = true; }}
    ondragleave={() => (dragOver = false)}
    ondrop={onDrop}
  >
    <div class={dragOver ? 'border-2 border-dashed border-bedrock-400/60 -m-1 p-1' : ''}>
      <table class="w-full text-xs">
        <thead>
          <tr class="text-left text-deep-400 uppercase tracking-wider text-[10px]">
            <th class="py-2 pr-2">Name</th>
            <th class="py-2 pr-2 w-24">Size</th>
            <th class="py-2 pr-2 w-28">Modified</th>
            <th class="py-2 w-24 text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#if loading}
            <tr><td colspan="4" class="py-8 text-center text-deep-400">Loading…</td></tr>
          {:else if listing && listing.entries.length > 0}
            {#each listing.entries as entry (entry.name)}
              <tr class="group border-t border-deep-700/40 hover:bg-deep-800/40 transition-colors">
                <td class="py-1.5 pr-2">
                  <button
                    class="flex items-center gap-2 min-w-0 w-full text-left"
                    ondblclick={() => open(entry)}
                    onkeydown={(e) => e.key === 'Enter' && open(entry)}
                    title={entry.name}
                  >
                    {#if entry.is_dir}
                      <Folder size={13} class="text-bedrock-400 shrink-0" />
                    {:else}
                      <FileText size={13} class="text-deep-400 shrink-0" />
                    {/if}
                    <span class="truncate {entry.is_dir ? 'text-bedrock-300' : 'text-deep-200'}">{entry.name}</span>
                  </button>
                </td>
                <td class="py-1.5 pr-2 font-mono text-deep-400">{entry.is_dir ? '—' : fmtSize(entry.size)}</td>
                <td class="py-1.5 pr-2 text-deep-400">{fmtDate(entry.modified)}</td>
                <td class="py-1.5 text-right">
                  <div class="flex items-center justify-end gap-0.5 opacity-60 group-hover:opacity-100 transition-opacity">
                    {#if !entry.is_dir}
                      <button
                        onclick={() => void api.downloadExplorerFile(instanceId, joinPath(path, entry.name)).catch((e: any) => addToast(`Download failed: ${e.message}`, 'error'))}
                        class="btn-ghost p-1"
                        title="Download"
                      ><Download size={12} /></button>
                    {/if}
                    {#if canWrite}
                      <button onclick={() => askRename(entry)} class="btn-ghost p-1" title="Rename">
                        <Pencil size={12} />
                      </button>
                      <button onclick={() => askDelete(entry)} class="btn-ghost p-1 text-red-400 hover:text-red-300" title="Delete">
                        <Trash2 size={12} />
                      </button>
                    {/if}
                  </div>
                </td>
              </tr>
            {/each}
          {:else}
            <tr>
              <td colspan="4" class="py-10 text-center text-deep-400 text-xs">
                {#if canWrite}
                  This folder is empty — drop files here or use Upload
                {:else}
                  This folder is empty
                {/if}
              </td>
            </tr>
          {/if}
        </tbody>
      </table>
      {#if path !== ''}
        <div class="pt-2">
          <button onclick={() => void load(epoch, parentPath(path))} class="text-[10px] uppercase tracking-wider text-bedrock-400 hover:text-bedrock-300 px-2 flex items-center gap-1">
            .. up one level
          </button>
        </div>
      {/if}
    </div>
  </div>

  <!-- inline text editor -->
  {#if editPath}
    <div class="card">
      <div class="flex items-center justify-between mb-3">
        <h3 class="card-header mb-0 font-mono">{editPath}</h3>
        <div class="flex gap-2">
          <button onclick={saveEditor} class="btn-primary text-xs">Save</button>
          <button onclick={closeEditor} class="btn-secondary text-xs">Close</button>
        </div>
      </div>
      <textarea bind:value={editContent} class="input w-full h-[45vh] font-mono text-sm" spellcheck="false"></textarea>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirmOpen}
  title={confirmTitle}
  message={confirmMessage}
  confirmText={confirmRecursive ? 'Delete everything' : 'Delete'}
  onconfirm={doDelete}
  oncancel={() => { confirmOpen = false; pendingDelete = null; }}
/>

{#if modalOpen}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center p-4 modal-bg"
    style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);"
    role="dialog"
    aria-modal="true"
  >
    <form onsubmit={submitModal} class="bg-deep-900 border-2 border-deep-600/60 p-6 w-full max-w-sm shadow-block-lg">
      <h2 class="text-sm font-bold text-strong uppercase tracking-widest mb-3">
        {modalMode === 'mkdir' ? 'New folder' : 'Rename'}
      </h2>
      {#if modalMode === 'rename'}
        <p class="text-xs text-deep-400 mb-2 font-mono truncate">{modalTarget}</p>
      {/if}
      <input
        class="input w-full text-xs font-mono mb-4"
        bind:value={modalName}
        placeholder={modalMode === 'mkdir' ? 'folder-name' : 'new-name'}
        aria-label="Name"
      />
      <div class="flex gap-2 justify-end">
        <button type="button" onclick={() => (modalOpen = false)} class="btn-secondary text-xs">Cancel</button>
        <button type="submit" class="btn-primary text-xs">
          {modalMode === 'mkdir' ? 'Create' : 'Rename'}
        </button>
      </div>
    </form>
  </div>
{/if}
