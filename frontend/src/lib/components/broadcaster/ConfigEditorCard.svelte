<script lang="ts">
  /**
   * config.yml editor for the active instance's broadcaster: a visual form
   * over the known schema or the raw YAML, drift hints against
   * server.properties, and ETag-guarded saves that refuse to overwrite a
   * file changed on disk since it was loaded.
   */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { normalize, overlayModel, visualDefaults, serverDrift } from '$lib/broadcaster/configModel';
  import type { BcVisualConfig } from '$lib/broadcaster/configModel';
  import type { BroadcasterStatus } from '$types/index';
  import { load as yamlLoad, dump as yamlDump } from 'js-yaml';
  import { Save, RefreshCw, Settings2, FileText, GitCompareArrows } from '@lucide/svelte';
  import VisualConfigEditor from './VisualConfigEditor.svelte';

  let { canManage, server, dirty = $bindable(false) }: {
    canManage: boolean;
    server: BroadcasterStatus['server'] | undefined;
    /** Mirrors unsaved edits so the page can mark its tab. */
    dirty?: boolean;
  } = $props();

  let configText = $state('');
  let configDirty = $state(false);
  let configMode = $state<'visual' | 'raw'>('visual');
  let cfg = $state<BcVisualConfig>(visualDefaults());
  let visualDirty = $state(false);
  let parsedDoc: Record<string, any> = {};
  // Version tag of the loaded config.yml; a different tag reported by a
  // status poll means the file changed on disk.
  let configEtag = '';
  let diskChanged = $state(false);
  let loadToken = 0;

  $effect(() => { dirty = configDirty; });

  function parseConfigToModel() {
    try {
      parsedDoc = (yamlLoad(configText) as Record<string, any>) ?? {};
      cfg = normalize(parsedDoc);
      visualDirty = false;
    } catch (e: any) {
      addToast(`Cannot open Visual editor: ${e.message}. Editing raw YAML.`, 'error');
      configMode = 'raw';
    }
  }

  function serializeModelToText() {
    configText = yamlDump(overlayModel(parsedDoc, cfg), { indent: 2, lineWidth: -1, noRefs: true });
    visualDirty = false;
    configDirty = true;
  }

  function switchMode(mode: 'visual' | 'raw') {
    if (mode === configMode) return;
    if (mode === 'visual') {
      parseConfigToModel();
    } else if (visualDirty) {
      serializeModelToText();
    }
    configMode = mode;
  }

  function markVisualDirty() {
    visualDirty = true;
    configDirty = true;
  }

  function tabClass(active: boolean): string {
    return 'flex items-center gap-1.5 px-3 py-1.5 text-[11px] uppercase tracking-wider transition-colors '
      + (active ? 'bg-bedrock-500/20 text-bedrock-300' : 'text-deep-400 hover:text-strong');
  }

  /** (Re)load config.yml into the editor and remember its version tag. */
  export async function load() {
    const mine = ++loadToken;
    configEtag = '';
    const loaded = await api.getBroadcasterConfig();
    if (mine !== loadToken) return;
    configText = loaded.text;
    configEtag = loaded.etag;
    configDirty = false;
    visualDirty = false;
    diskChanged = false;
    parseConfigToModel();
  }

  /** Called with the etag a status poll reports. */
  export async function noticeEtag(etag: string | undefined) {
    if (!etag || !configEtag || etag === configEtag) return;
    if (configDirty) {
      diskChanged = true;
    } else {
      await load();
      addToast('config.yml changed on disk — editor reloaded', 'info');
    }
  }

  async function reloadFromDisk() {
    try {
      await load();
      addToast('Reloaded config.yml from disk', 'success');
    } catch (e: any) {
      addToast(`Reload failed: ${e.message}`, 'error');
    }
  }

  // config.yml vs server.properties drift (operational first).
  const drift = $derived(serverDrift(cfg, server));
  function applyDrift(items: ReturnType<typeof serverDrift>) {
    for (const item of items) item.apply(cfg);
    markVisualDirty();
    addToast(`Applied ${items.length} value${items.length === 1 ? '' : 's'} from server.properties — review and Save`, 'info');
  }

  async function saveConfig() {
    if (configMode === 'visual' || visualDirty) {
      if (cfg.session['update-interval'] < 20 || cfg['friend-sync']['update-interval'] < 20) {
        addToast('Update intervals must be at least 20 seconds (Xbox rate limits)', 'error');
        return;
      }
      serializeModelToText();
    }
    try {
      const r = await api.saveBroadcasterConfig(configText, configEtag);
      configDirty = false;
      diskChanged = false;
      if (r.etag) configEtag = r.etag;
      parsedDoc = (yamlLoad(configText) as Record<string, any>) ?? {};
      addToast('Config saved; restart the broadcaster to apply', 'success');
    } catch (e: any) {
      if (String(e.message).includes('409')) {
        diskChanged = true;
        addToast('config.yml changed on disk since you opened it — reload, then re-apply your edit', 'error');
      } else {
        addToast(`Save failed: ${e.message}`, 'error');
      }
    }
  }
</script>

    <div class="card">
      <div class="flex items-center justify-between">
        <h2 class="card-header">config.yml</h2>
        <div class="flex items-center gap-2">
          {#if configDirty}<span class="text-[10px] uppercase tracking-wider text-yellow-400">unsaved</span>{/if}
          <div class="flex border-2 border-deep-600/40 overflow-hidden">
            <button onclick={() => switchMode('visual')} class={tabClass(configMode === 'visual')}
                    title="Form editor over the known config schema">
              <Settings2 size={12} /> Visual
            </button>
            <button onclick={() => switchMode('raw')} class={tabClass(configMode === 'raw') + ' border-l-2 border-deep-600/40'}
                    title="Edit the raw YAML document">
              <FileText size={12} /> Raw
            </button>
          </div>
          {#if canManage}
            <button onclick={saveConfig} disabled={!configDirty}
                    class="btn-primary flex items-center gap-1.5 text-xs">
              <Save size={13} /> Save
            </button>
          {/if}
        </div>
      </div>

      {#if diskChanged}
        <div class="mb-3 flex items-center justify-between gap-3 border-2 border-yellow-500/40 bg-yellow-500/5 px-3 py-2 text-xs">
          <span class="text-yellow-300">config.yml changed on disk while you were editing. Saving now is refused so the newer file is not overwritten.</span>
          <button onclick={reloadFromDisk} class="btn-ghost flex items-center gap-1.5 text-xs shrink-0">
            <RefreshCw size={12} /> Reload from disk
          </button>
        </div>
      {/if}
      {#if drift.length > 0 && configMode === 'visual'}
        <div class="mb-3 border-2 border-bedrock-500/30 bg-bedrock-950/20 p-3 space-y-2">
          <div class="flex items-center justify-between gap-2">
            <p class="flex items-center gap-1.5 text-[10px] uppercase tracking-widest text-bedrock-300">
              <GitCompareArrows size={12} /> Differs from server.properties
            </p>
            {#if canManage}
              <button onclick={() => applyDrift(drift)} class="btn-ghost text-[11px] px-2 py-1">Sync from server.properties</button>
            {/if}
          </div>
          <ul class="space-y-1">
            {#each drift as item (item.key)}
              <li class="flex items-center justify-between gap-2 text-[11px]">
                <span class="text-deep-300">
                  <span class="{item.kind === 'operational' ? 'text-yellow-300' : 'text-deep-400'}">{item.label}</span>:
                  <span class="font-mono text-strong">{item.config}</span>
                  <span class="text-deep-400">→</span>
                  <span class="font-mono text-teal-300">{item.server}</span>
                  {#if item.kind === 'fallback'}<span class="text-deep-500"> (fallback only)</span>{/if}
                </span>
                {#if canManage}
                  <button onclick={() => applyDrift([item])} class="text-bedrock-400 hover:underline shrink-0">apply</button>
                {/if}
              </li>
            {/each}
          </ul>
          <p class="text-[10px] text-deep-400 leading-snug">
            With query-server on, the Xbox session shows the server's live name, level, and player count;
            the session-info values here only appear when the query cannot answer. Applying only changes
            the form — review, then Save.
          </p>
        </div>
      {/if}
      {#if configMode === 'visual'}
        <VisualConfigEditor bind:cfg visualDirty={visualDirty} canManage={canManage} onmarkdirty={markVisualDirty} />
      {:else}
        <textarea bind:value={configText} oninput={() => (configDirty = true)}
                  spellcheck="false"
                  class="input w-full h-80 font-mono text-xs leading-relaxed whitespace-pre overflow-auto"
                  disabled={!canManage}></textarea>
      {/if}
      <p class="text-[10px] text-deep-400 mt-2 leading-relaxed">
        <span class="text-yellow-400 uppercase tracking-wider">Relay target:</span>
        <span class="font-mono text-deep-400">session.remote-address</span> is where join offers are
        relayed to, and it doubles as the address the query ping checks (NetherNet HTTP signaling on
        TCP server-port first, RakNet on UDP as fallback). Because the panel runs the
        broadcaster on the same host as the server, keep it at
        <span class="font-mono text-strong">127.0.0.1</span> with the instance's server-port — Endstone
        advertises its own public address to clients. A public IP here sends every relay through the
        router's hairpin loop, which is refused: friends-list joins fail with
        <span class="font-mono">"Failed to relay offer … connection refused"</span> while everything
        else looks healthy. Older configs with <span class="font-mono">session-info.ip</span> are
        migrated into the relay keys by the jar itself (config-version 7).
      </p>
      <p class="text-[10px] text-deep-400 mt-1">
        Full MCXboxBroadcast config. Save, then restart the broadcaster to apply — the panel itself needs no restart.
      </p>
    </div>
