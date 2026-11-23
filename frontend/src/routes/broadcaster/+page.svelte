<script lang="ts">
  /**
   * MCXboxBroadcast for the active instance. Overview (status, sign-in codes,
   * accounts, live console), Configuration (config.yml) and Screenshot tabs.
   * The config editor stays mounted while hidden so unsaved edits survive
   * switching tabs.
   */
  import { onDestroy, onMount } from 'svelte';
  import { api, getInstance } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import { detectLevel, type FeedLine } from '$lib/console';
  import Terminal from '$lib/components/terminal/Terminal.svelte';
  import {
    broadcasterFeed,
    handleBroadcastEvent,
    clearView as clearBroadcastView,
    resumeSuffix,
    isReconnect,
    markLoaded,
    resetFor,
  } from '$lib/broadcaster/console.ts';
  import type { BroadcasterStatus, DeviceCode, BroadcasterJarInfo } from '$types/index';
  import { Radio, LayoutDashboard, Settings2, Image } from '@lucide/svelte';
  import StatusBanners from '$lib/components/broadcaster/StatusBanners.svelte';
  import ProcessCard from '$lib/components/broadcaster/ProcessCard.svelte';
  import DeviceCodesCard from '$lib/components/broadcaster/DeviceCodesCard.svelte';
  import AccountsCard from '$lib/components/broadcaster/AccountsCard.svelte';
  import ConfigEditorCard from '$lib/components/broadcaster/ConfigEditorCard.svelte';
  import ScreenshotCard from '$lib/components/broadcaster/ScreenshotCard.svelte';

  type Tab = 'overview' | 'config' | 'screenshot';
  let tab = $state<Tab>('overview');
  let status = $state<BroadcasterStatus | null>(null);
  let codes = $state<Record<string, DeviceCode>>({});
  // Module-level feed: the view (and any Clear) survives tab switches; see
  // $lib/broadcaster/console for the replay/resume bookkeeping.
  const feed = broadcasterFeed;
  let lines = $state<FeedLine[]>([]);
  feed.subscribe(v => { lines = v; });
  let jarInfo = $state<BroadcasterJarInfo | null>(null);
  let editor: ReturnType<typeof ConfigEditorCard> | undefined = $state();
  let configDirty = $state(false);
  // The instance the page currently shows; async work for any other
  // instance (a poll still in flight across a switch) is discarded.
  let currentId = $state<string>('');
  let busy = $state('');
  let newAccount = $state('');
  let poll: ReturnType<typeof setInterval>;
  const unsubs: Array<() => void> = [];

  const canManage = $derived($userPermissions.includes('BROADCASTER_MANAGE'));

  function pushLine(text: string, level?: string) {
    feed.push(text, (level || detectLevel(text)) as FeedLine['level']);
  }

  function instanceId(): string {
    // Before the instance list arrives, follow the id requests are already
    // scoped to (the persisted selection) so the first load is not 'default'.
    return $activeInstance?.id || getInstance() || 'default';
  }

  async function refresh() {
    const forId = currentId;
    try {
      const next = await api.getBroadcasterStatus();
      // A response for an instance we already left would paint the other
      // server's broadcaster here.
      if (forId !== currentId) return;
      status = next;
      codes = next.device_codes;
      await editor?.noticeEtag(next.config_etag);
    } catch { /* transient */ }
  }

  async function loadJarInfo(refreshLatest = false) {
    const forId = currentId;
    try {
      const info = await api.getBroadcasterJar(refreshLatest);
      if (forId === currentId) jarInfo = info;
    } catch { /* optional */ }
  }

  async function updateJar() {
    if (busy) return;
    busy = 'jar';
    try {
      const r = await api.updateBroadcasterJar();
      addToast(r.message, 'success');
      await loadJarInfo(true);
    } catch (e: any) {
      addToast(`Jar update failed: ${e.message}`, 'error');
    }
    busy = '';
  }

  /** (Re)load everything for the active instance: console stream, status, config. */
  async function loadAll() {
    for (const unsub of unsubs.splice(0)) unsub();
    // The WebSocket manager appends a fresh short-lived download token.
    const instance = instanceId();
    currentId = instance;
    // Switching to a different instance resets the view and its replay
    // bookkeeping; returning to a page we were just on is a RECONNECT — the
    // module-level feed already holds that view, so the socket resumes with
    // after=<lastSeq> and the backend replays nothing (a cleared view stays
    // cleared instead of resurrecting from the journal).
    const fresh = !isReconnect(instance);
    if (fresh) {
      broadcasterFeed.clear();
      resetFor(instance);
    }
    markLoaded(instance);
    const query = `instance=${encodeURIComponent(instance)}${fresh ? '' : resumeSuffix(instance)}`;
    unsubs.push(wsManager.connect(`/api/v1/broadcaster/ws?${query}`, (data) => {
      if (data.type === 'hello' || data.type === 'console') {
        handleBroadcastEvent(instance, data);
      } else if (data.type === 'status') {
        if (status) status.status = data.status as BroadcasterStatus['status'];
        else refresh();
      } else if (data.type === 'device_codes') {
        codes = data.codes as Record<string, DeviceCode>;
      }
    }));
    jarInfo = null;
    void loadJarInfo();
    await refresh();
    try {
      await editor?.load();
    } catch (e: any) {
      addToast(`Config load failed: ${e.message}`, 'error');
    }
  }

  onMount(() => {
    poll = setInterval(() => { if (!document.hidden) void refresh(); }, 5000);
  });

  // Every broadcaster is instance-scoped: switching instances swaps the
  // console stream, status, device codes, config, and screenshots. Keyed on
  // the instance id — the layout's periodic list refresh re-emits
  // activeInstance with a fresh object even when the selection is unchanged,
  // and reloading on every emission would reset the page while you use it.
  let loadedInstanceId: string | null = null;
  $effect(() => {
    const id = $activeInstance?.id || getInstance() || 'default';
    if (id === loadedInstanceId) return;
    loadedInstanceId = id;
    void loadAll();
  });

  onDestroy(() => {
    clearInterval(poll);
    for (const unsub of unsubs) unsub();
  });

  async function action(kind: 'start' | 'stop' | 'restart' | 'takeover') {
    if (busy) return;
    busy = kind;
    try {
      const r = await api.broadcasterAction(kind);
      addToast(r.message || kind, 'success');
      await refresh();
      if (kind !== 'stop') setTimeout(() => void loadJarInfo(), 4000);
    } catch (e: any) { addToast(`${kind} failed: ${e.message}`, 'error'); }
    busy = '';
  }

  async function send(cmd: string) {
    pushLine(`> ${cmd}`, 'local');
    try { await api.sendBroadcasterCommand(cmd); } catch (e: any) {
      addToast(`Command failed: ${e.message}`, 'error');
    }
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

  <div class="flex flex-wrap gap-1 border-b border-deep-600/30 pb-px" role="tablist">
    {#each [
      { id: 'overview' as const, label: 'Overview', icon: LayoutDashboard },
      { id: 'config' as const, label: 'Configuration', icon: Settings2 },
      { id: 'screenshot' as const, label: 'Screenshot', icon: Image },
    ] as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}
              class="text-xs px-4 py-2 uppercase tracking-wider font-semibold transition rounded-t flex items-center gap-1.5
                     {tab === t.id ? 'bg-deep-800/60 text-strong border-b-2 border-bedrock-500' : 'text-deep-400 hover:text-deep-200'}">
        <t.icon size={12} /> {t.label}
        {#if t.id === 'config' && configDirty}<span class="w-1.5 h-1.5 rounded-full bg-yellow-400" title="Unsaved changes"></span>{/if}
      </button>
    {/each}
  </div>

  {#if tab === 'overview'}
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
      <ProcessCard {status} canManage={canManage} {busy} onaction={action} {jarInfo} onupdatejar={updateJar} />
      <DeviceCodesCard {codes} oncopy={copy} />
      <AccountsCard accounts={status?.accounts ?? []} canManage={canManage} bind:newAccount onaction={accountAction} />
    </div>

    <!-- Console: same shared terminal widget as the Console page -->
    <Terminal
      title="broadcaster-console"
      heightKey="broadcaster"
      {lines}
      emptyHint="> Broadcaster output appears here…"
      onclear={() => clearBroadcastView(instanceId())}
      onsend={send}
      canSend={canManage}
      sendPlaceholder="accounts list · restart · dumpsession · stop…"
    />
  {:else if tab === 'screenshot'}
    <div class="max-w-xl">
      <ScreenshotCard canManage={canManage} instanceId={currentId} accounts={status?.accounts ?? []} />
    </div>
  {/if}
  <div class:hidden={tab !== 'config'}>
    <ConfigEditorCard bind:this={editor} bind:dirty={configDirty} canManage={canManage} server={status?.server} />
  </div>
</div>
