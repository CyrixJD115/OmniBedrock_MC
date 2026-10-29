<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, getToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { createConsoleFeed, detectLevel, nearBottom, stickToBottom } from '$lib/console';
  import type { FeedLine } from '$lib/console';
  import type { BroadcasterStatus, DeviceCode } from '$types/index';
  import { normalize, overlayModel, visualDefaults } from '$lib/broadcaster/configModel';
  import type { BcVisualConfig } from '$lib/broadcaster/configModel';
  import { load as yamlLoad, dump as yamlDump } from 'js-yaml';
  import {
    Radio, Send, Save, Upload, RefreshCw, Settings2, FileText,
    ArrowDown, Eraser, FileCode
  } from '@lucide/svelte';
  import StatusBanners from '$lib/components/broadcaster/StatusBanners.svelte';
  import ProcessCard from '$lib/components/broadcaster/ProcessCard.svelte';
  import DeviceCodesCard from '$lib/components/broadcaster/DeviceCodesCard.svelte';
  import AccountsCard from '$lib/components/broadcaster/AccountsCard.svelte';
  import VisualConfigEditor from '$lib/components/broadcaster/VisualConfigEditor.svelte';

  let status = $state<BroadcasterStatus | null>(null);
  let codes = $state<Record<string, DeviceCode>>({});
  // Capped feed with stable line keys and rAF batching: see createConsoleFeed.
  // Without stable keys this console rebuilt its entire DOM for every incoming
  // line once the cap was reached, which froze stop/restart until it caught up.
  const feed = createConsoleFeed(500);
  let lines = $state<FeedLine[]>([]);
  feed.subscribe(v => { lines = v; });
  let input = $state('');
  let configText = $state('');
  let configDirty = $state(false);
  let configMode = $state<'visual' | 'raw'>('visual');
  let cfg = $state<BcVisualConfig>(visualDefaults());
  let visualDirty = $state(false);
  let parsedDoc: Record<string, any> = {};
  let busy = $state('');
  let newAccount = $state('');
  let screenshotAccount = $state('main');
  let previewUrl = $state('');
  let terminalEl: HTMLDivElement;
  let screenshotInput = $state<HTMLInputElement | undefined>(undefined);
  let poll: ReturnType<typeof setInterval>;
  const unsubs: Array<() => void> = [];

  function loadSetting<T>(key: string, def: T): T {
    if (typeof localStorage === 'undefined') return def;
    try { return JSON.parse(localStorage.getItem(key) || 'null') ?? def; }
    catch { return def; }
  }
  function saveSetting(key: string, value: unknown) {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, JSON.stringify(value));
  }

  // Console autoscroll: only pin to the bottom when the reader is already
  // there (or the toggle is on and they haven't scrolled away), so a replay
  // or a stray log line never drags the viewport around.
  let autoScroll = $state(loadSetting('omb_bc_console_scroll', true));
  let stick = $state(true);
  $effect(() => { saveSetting('omb_bc_console_scroll', autoScroll); });

  function onTerminalScroll() {
    if (!terminalEl) return;
    stick = nearBottom(terminalEl);
  }

  function jumpToLatest() {
    stick = true;
    if (terminalEl) stickToBottom(terminalEl);
  }

  // Runs after the DOM update, so scrollHeight is final and a replayed batch
  // settles with one scroll instead of one per line.
  $effect(() => {
    void lines.length;
    if (!autoScroll || !stick || !terminalEl) return;
    stickToBottom(terminalEl);
  });

  const canManage = $derived($userPermissions.includes('BROADCASTER_MANAGE'));

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


  function pushLine(text: string, level?: string) {
    feed.push(text, (level || detectLevel(text)) as FeedLine['level']);
  }

  async function refresh() {
    try {
      status = await api.getBroadcasterStatus();
      codes = status.device_codes;
    } catch { /* transient */ }
  }

  /** (Re)load everything for the active instance: console stream, status, config. */
  async function loadAll() {
    for (const unsub of unsubs.splice(0)) unsub();
    feed.clear();
    stick = true;
    const token = getToken();
    const instance = $activeInstance?.id || 'default';
    const query = `${token ? `token=${encodeURIComponent(token)}&` : ''}instance=${encodeURIComponent(instance)}`;
    unsubs.push(wsManager.connect(`/api/v1/broadcaster/ws?${query}`, (data) => {
      if (data.type === 'hello' && data.resumed === false) feed.clear();
      else if (data.type === 'console') {
        pushLine(data.line as string, data.level as string);
      } else if (data.type === 'status') {
        if (status) status.status = data.status as BroadcasterStatus['status'];
        else refresh();
      } else if (data.type === 'device_codes') {
        codes = data.codes as Record<string, DeviceCode>;
      }
    }));
    await refresh();
    configDirty = false;
    visualDirty = false;
    try {
      configText = await api.getBroadcasterConfig();
      parseConfigToModel();
    } catch (e: any) {
      addToast(`Config load failed: ${e.message}`, 'error');
    }
  }

  onMount(() => {
    poll = setInterval(refresh, 5000);
  });

  // Every broadcaster is instance-scoped: switching instances swaps the
  // console stream, status, device codes, config, and screenshots. Keyed on
  // the instance id — the layout's periodic list refresh re-emits
  // activeInstance with a fresh object even when the selection is unchanged,
  // and reloading on every emission would reset the page while you use it.
  let loadedInstanceId: string | null = null;
  $effect(() => {
    const id = $activeInstance?.id ?? 'default';
    if (id === loadedInstanceId) return;
    loadedInstanceId = id;
    void loadAll();
  });

  async function loadPreview(account: string) {
    try {
      const r = await fetch(`/api/v1/broadcaster/screenshot?account=${encodeURIComponent(account)}&token=${getToken()}&instance=${encodeURIComponent($activeInstance?.id || 'default')}`);
      if (!r.ok) throw new Error(String(r.status));
      const blob = await r.blob();
      const url = URL.createObjectURL(blob);
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      previewUrl = url;
    } catch {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      previewUrl = '';
    }
  }

  $effect(() => {
    void loadPreview(screenshotAccount);
  });

  onDestroy(() => {
    clearInterval(poll);
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    for (const unsub of unsubs) unsub();
  });

  async function action(kind: 'start' | 'stop' | 'restart' | 'takeover') {
    if (busy) return;
    busy = kind;
    try {
      const r = await api.broadcasterAction(kind);
      addToast(r.message || kind, 'success');
      await refresh();
    } catch (e: any) { addToast(`${kind} failed: ${e.message}`, 'error'); }
    busy = '';
  }

  async function send() {
    const cmd = input.trim();
    if (!cmd) return;
    input = '';
    pushLine(`> ${cmd}`, 'local');
    try { await api.sendBroadcasterCommand(cmd); } catch (e: any) {
      addToast(`Command failed: ${e.message}`, 'error');
    }
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
      await api.saveBroadcasterConfig(configText);
      configDirty = false;
      parsedDoc = (yamlLoad(configText) as Record<string, any>) ?? {};
      addToast('Config saved; restart the broadcaster to apply', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  async function accountAction(kind: 'add' | 'remove') {
    const id = newAccount.trim();
    if (!id) return;
    try {
      const r = await api.broadcasterAccount(kind, id);
      addToast(r.message || 'Done', 'success');
      newAccount = '';
      setTimeout(refresh, 1500);
    } catch (e: any) { addToast(`${kind} failed: ${e.message}`, 'error'); }
  }

  async function uploadScreenshot(event: Event) {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const r = await api.uploadBroadcasterScreenshot(screenshotAccount, file);
      addToast(r.message || 'Uploaded', 'success');
      await loadPreview(screenshotAccount);
    } catch (e: any) { addToast(`Upload failed: ${e.message}`, 'error'); }
    (event.target as HTMLInputElement).value = '';
  }

  function copy(text: string) {
    navigator.clipboard.writeText(text);
    addToast('Copied', 'success');
  }
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div class="flex items-center gap-3">
      <Radio size={22} class="text-bedrock-400" />
      <div>
        <h1 class="text-lg font-bold text-strong uppercase tracking-widest">MCXboxBroadcast</h1>
        <p class="text-[10px] uppercase tracking-wider text-deep-400 mt-0.5">
          instance: <span class="text-bedrock-300">{$activeInstance?.name ?? $activeInstance?.id ?? 'default'}</span>
          — each server keeps its own broadcaster, accounts, and config
        </p>
        <div class="pixel-divider mt-2 w-24"></div>
      </div>
    </div>
    {#if status}
      <div class="flex items-center gap-2">
        <span class="status-dot {status.status.status}"></span>
        <span class="text-xs uppercase tracking-widest {status.status.status === 'running' ? 'text-teal-400' : 'text-red-400'}">
          {status.status.status}
        </span>
      </div>
    {/if}
  </div>

  <!-- Status warnings: jar, external broadcaster, relay target, ping -->
  <StatusBanners {status} canManage={canManage} {busy} onaction={action} />

  <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
    <!-- Status + controls -->
    <ProcessCard {status} canManage={canManage} {busy} onaction={action} />

    <!-- Device codes -->
    <DeviceCodesCard {codes} oncopy={copy} />

    <!-- Accounts -->
    <AccountsCard accounts={status?.accounts ?? []} canManage={canManage} bind:newAccount onaction={accountAction} />
  </div>

  <!-- Console -->
  <div class="card">
    <div class="flex items-center justify-between flex-wrap gap-2">
      <h2 class="card-header flex items-center gap-2"><FileCode size={13} /> Console</h2>
      <div class="flex items-center gap-3">
        <span class="text-[10px] font-mono text-deep-400">{lines.length} lines</span>
        <label class="flex items-center gap-1.5 text-xs text-deep-400 uppercase tracking-wider cursor-pointer select-none">
          <input type="checkbox" bind:checked={autoScroll} class="accent-bedrock-500" />
          Scroll
        </label>
        <button onclick={() => feed.clear()} class="btn-ghost p-1.5" title="Clear view (backend history keeps flowing)">
          <Eraser size={12} />
        </button>
        <button onclick={refresh} class="btn-ghost p-1.5" title="Refresh"><RefreshCw size={12} /></button>
      </div>
    </div>
    <div class="relative">
      <div bind:this={terminalEl} onscroll={onTerminalScroll}
           class="terminal-container h-72 md:h-96 overflow-y-auto p-4 font-terminal text-base leading-relaxed terminal-surface border-2 border-deep-600/50">
        {#each lines as line (line.id)}
          <div class={"terminal-line leading-snug " + (
            line.level === 'error' ? 'terminal-line-error' :
            line.level === 'warn' ? 'terminal-line-warn' :
            line.level === 'local' ? 'terminal-line-local' :
            'terminal-line-info'
          )}>{line.text}</div>
        {/each}
        {#if lines.length === 0}
          <div class="terminal-line-plain opacity-60 font-mono">&gt; Broadcaster output appears here…</div>
        {/if}
      </div>
      {#if !stick && lines.length > 0}
        <button onclick={jumpToLatest}
                class="absolute bottom-4 right-4 z-10 flex items-center gap-1.5 px-3 py-1.5 text-[11px] uppercase tracking-wider
                       bg-deep-900/95 border-2 border-bedrock-500/60 text-bedrock-300 hover:text-strong hover:border-bedrock-400 shadow-block transition-colors">
          <ArrowDown size={12} /> Jump to latest
        </button>
      {/if}
    </div>
    {#if canManage}
      <div class="flex gap-2 mt-3">
        <input type="text" bind:value={input} onkeydown={(e) => e.key === 'Enter' && send()}
               placeholder="accounts list · restart · dumpsession · stop…"
               class="input flex-1 font-terminal text-base tracking-wider" />
        <button onclick={send} class="btn-primary flex items-center gap-2 text-xs">
          <Send size={14} /> Send
        </button>
      </div>
    {/if}
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
    <!-- Config editor -->
    <div class="card lg:col-span-2">
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
        relayed to, and it doubles as the address the query ping checks. Because the panel runs the
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

    <!-- Screenshot -->
    <div class="card">
      <h2 class="card-header">Screenshot</h2>
      <div class="space-y-3">
        <select bind:value={screenshotAccount} class="input w-full text-xs">
          {#each status?.accounts ?? [{ id: 'main', main: true }] as account (account.id)}
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
  </div>
</div>
