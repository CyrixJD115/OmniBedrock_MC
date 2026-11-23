<script lang="ts">
  /**
   * The per-pack editors (manifest JSON, version, UUID, folder rename) and
   * their modals. The page opens them through the exported functions and is
   * told what changed via `onchanged(oldPath, newPath)` so it can reload and
   * keep the preview drawer pointing at the right folder.
   */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import type { Addon } from '$types/index';
  import ManifestEditorModal from './ManifestEditorModal.svelte';
  import VersionEditorModal from './VersionEditorModal.svelte';
  import UuidEditorModal from './UuidEditorModal.svelte';
  import RenameModal from './RenameModal.svelte';

  let { log, onchanged }: {
    log: (msg: string) => void;
    onchanged: (oldPath: string, newPath: string) => Promise<void> | void;
  } = $props();

  let manifestOpen = $state(false);
  let manifestPath = $state('');
  let manifestEditor: ReturnType<typeof ManifestEditorModal> | undefined = $state();

  let versionOpen = $state(false);
  let versionAddon = $state<Addon | null>(null);
  let major = $state(0);
  let minor = $state(0);
  let patch = $state(0);

  let uuidOpen = $state(false);
  let uuidPath = $state('');
  let uuidValue = $state('');
  let uuidMode = $state<'change' | 'randomize'>('change');

  let renameOpen = $state(false);
  let renamePath = $state('');
  let renameValue = $state('');

  export function openManifest(a: Addon) {
    manifestPath = a.path;
    manifestEditor?.openWith(a.manifest);
  }

  export function openVersion(a: Addon) {
    const v = a.version ?? [0, 0, 0];
    [major, minor, patch] = [v[0] ?? 0, v[1] ?? 0, v[2] ?? 0];
    versionAddon = a;
    versionOpen = true;
  }

  export function openUuid(a: Addon, mode: 'change' | 'randomize') {
    uuidMode = mode;
    uuidPath = a.path;
    uuidValue = mode === 'randomize' ? crypto.randomUUID() : a.uuid;
    uuidOpen = true;
  }

  export function openRename(a: Addon) {
    renamePath = a.path;
    renameValue = a.name;
    renameOpen = true;
  }

  async function saveManifest(raw: string) {
    try {
      await api.updateManifest(manifestPath, JSON.parse(raw));
      addToast('Manifest saved', 'success');
      log(`Manifest saved: ${manifestPath}`);
      manifestOpen = false;
      await onchanged(manifestPath, manifestPath);
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
    }
  }

  async function saveVersion() {
    if (!versionAddon) return;
    const manifest: Record<string, any> = versionAddon.manifest
      ? JSON.parse(JSON.stringify(versionAddon.manifest))
      : {};
    if (!manifest.header) manifest.header = {};
    manifest.header.version = [major, minor, patch];
    const path = versionAddon.path;
    try {
      await api.updateManifest(path, manifest);
      addToast(`Version set to ${major}.${minor}.${patch}`, 'success');
      log(`Version: ${versionAddon.name} → ${major}.${minor}.${patch}`);
      versionOpen = false;
      await onchanged(path, path);
    } catch (e: any) {
      addToast(`Failed: ${e.message}`, 'error');
    }
  }

  async function saveUuid() {
    try {
      if (uuidMode === 'randomize') await api.randomizeUuid(uuidPath);
      else await api.changeUuid(uuidPath, uuidValue);
      addToast(`UUID ${uuidMode === 'randomize' ? 'randomized' : 'updated'}`, 'success');
      log(`UUID ${uuidMode} for ${uuidPath}`);
      uuidOpen = false;
      await onchanged(uuidPath, uuidPath);
    } catch (e: any) {
      addToast(`Failed: ${e.message}`, 'error');
    }
  }

  async function saveRename() {
    const name = renameValue.trim();
    if (!name) return;
    try {
      const r = await api.renameAddon(renamePath, name);
      addToast(`Renamed to ${name}`, 'success');
      log(`Renamed: ${renamePath} → ${r.new_path}`);
      renameOpen = false;
      await onchanged(renamePath, r.new_path);
    } catch (e: any) {
      addToast(`Rename failed: ${e.message}`, 'error');
    }
  }
</script>

<ManifestEditorModal bind:open={manifestOpen} bind:this={manifestEditor} onsave={saveManifest} />
{#if versionOpen && versionAddon}
  <VersionEditorModal bind:open={versionOpen} name={versionAddon.name} bind:major bind:minor bind:patch onsave={saveVersion} />
{/if}
<UuidEditorModal bind:open={uuidOpen} mode={uuidMode} bind:value={uuidValue} onsave={saveUuid} />
<RenameModal bind:open={renameOpen} bind:value={renameValue} onsave={saveRename} />
