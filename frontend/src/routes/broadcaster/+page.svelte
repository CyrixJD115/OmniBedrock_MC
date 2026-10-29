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
  import { load as yamlLoad, dump as yamlDump } from 'js-yaml';
  import {
    Play, Square, RotateCw, Send, Copy, ExternalLink, Plus, Trash2, Save,
    Upload, Radio, KeyRound, UserRound, FileCode, RefreshCw, Settings2, FileText,
    ArrowDown, Eraser, TriangleAlert
  } from '@lucide/svelte';

  /** Known config.yml shape (MCXboxBroadcast config-version 7). */
  interface BcVisualConfig {
    session: {
      'remote-address': string;
      'remote-port': string;
      'update-interval': number;
      'query-server': boolean;
      'web-query-fallback': boolean;
      'config-fallback': boolean;
      'session-info': {
        'host-name': string;
        'world-name': string;
        'game-mode': string;
        players: number;
        'max-players': number;
      };
      'hide-old-sessions': boolean;
      'ice-port-range': { min: number; max: number };
    };
    'friend-sync': {
      'update-interval': number;
      'auto-friend': boolean;
      'initial-invite': boolean;
      expiry: { enabled: boolean; days: number; check: number };
    };
    notifications: {
      enabled: boolean;
      'webhook-url': string;
      'session-expired-message': string;
      'friend-restriction-message': string;
    };
    'debug-mode': boolean;
    'suppress-session-update-message': boolean;
  }

  function visualDefaults(): BcVisualConfig {
    return {
      session: {
        // The offer-relay target: loopback, because the panel runs the
        // broadcaster on the same host as the server (config-version 7;
        // pre-v7 session-info.ip/port are migrated into these keys by the jar).
        'remote-address': '127.0.0.1',
        'remote-port': 'auto',
        'update-interval': 30,
        'query-server': true,
        'web-query-fallback': false,
        'config-fallback': false,
        'session-info': {
          'host-name': '',
          'world-name': '',
          'game-mode': 'Survival',
          players: 0,
          'max-players': 20,
        },
        'hide-old-sessions': true,
        'ice-port-range': { min: 0, max: 0 },
      },
      'friend-sync': {
        'update-interval': 60,
        'auto-friend': true,
        'initial-invite': true,
        expiry: { enabled: true, days: 15, check: 1800 },
      },
      notifications: {
        enabled: false,
        'webhook-url': '',
        'session-expired-message': '',
        'friend-restriction-message': '',
      },
      'debug-mode': false,
      'suppress-session-update-message': false,
    };
  }

  function num(value: unknown, fallback: number): number {
    const n = Number(value);
    return Number.isFinite(n) ? n : fallback;
  }
  function str(value: unknown, fallback: string): string {
    return typeof value === 'string' ? value : fallback;
  }
  function bool(value: unknown, fallback: boolean): boolean {
    return typeof value === 'boolean' ? value : fallback;
  }

  /** Fill the form model from a parsed YAML document, defaulting unknown fields. */
  function normalize(doc: Record<string, any>): BcVisualConfig {
    const d = visualDefaults();
    const s = doc['session'] ?? {};
    const info = s['session-info'] ?? {};
    const ice = s['ice-port-range'] ?? {};
    const f = doc['friend-sync'] ?? {};
    const expiry = f['expiry'] ?? {};
    const n = doc['notifications'] ?? {};
    d.session['remote-address'] = str(s['remote-address'], '127.0.0.1');
    d.session['remote-port'] = str(s['remote-port'], 'auto');
    d.session['update-interval'] = num(s['update-interval'], 30);
    d.session['query-server'] = bool(s['query-server'], true);
    d.session['web-query-fallback'] = bool(s['web-query-fallback'], false);
    d.session['config-fallback'] = bool(s['config-fallback'], false);
    d.session['hide-old-sessions'] = bool(s['hide-old-sessions'], true);
    d.session['session-info']['host-name'] = str(info['host-name'], '');
    d.session['session-info']['world-name'] = str(info['world-name'], '');
    d.session['session-info']['game-mode'] = str(info['game-mode'], 'Survival');
    d.session['session-info']['players'] = num(info['players'], 0);
    d.session['session-info']['max-players'] = num(info['max-players'], 20);
    d.session['ice-port-range'].min = num(ice['min'], 0);
    d.session['ice-port-range'].max = num(ice['max'], 0);
    d['friend-sync']['update-interval'] = num(f['update-interval'], 60);
    d['friend-sync']['auto-friend'] = bool(f['auto-friend'], true);
    d['friend-sync']['initial-invite'] = bool(f['initial-invite'], true);
    d['friend-sync'].expiry.enabled = bool(expiry['enabled'], true);
    d['friend-sync'].expiry.days = num(expiry['days'], 15);
    d['friend-sync'].expiry.check = num(expiry['check'], 1800);
    d.notifications.enabled = bool(n['enabled'], false);
    d.notifications['webhook-url'] = str(n['webhook-url'], '');
    d.notifications['session-expired-message'] = str(n['session-expired-message'], '');
    d.notifications['friend-restriction-message'] = str(n['friend-restriction-message'], '');
    d['debug-mode'] = bool(doc['debug-mode'], false);
    d['suppress-session-update-message'] = bool(doc['suppress-session-update-message'], false);
    return d;
  }

  /** Overlay the form model onto the last parsed document so unknown keys survive. */
  function overlayModel(doc: Record<string, any>, model: BcVisualConfig): Record<string, any> {
    doc['session'] = {
      ...(doc['session'] ?? {}),
      'remote-address': model.session['remote-address'],
      'remote-port': model.session['remote-port'],
      'update-interval': model.session['update-interval'],
      'query-server': model.session['query-server'],
      'web-query-fallback': model.session['web-query-fallback'],
      'config-fallback': model.session['config-fallback'],
      'hide-old-sessions': model.session['hide-old-sessions'],
      'session-info': {
        ...(doc['session']?.['session-info'] ?? {}),
        ...model.session['session-info'],
      },
      'ice-port-range': {
        ...(doc['session']?.['ice-port-range'] ?? {}),
        ...model.session['ice-port-range'],
      },
    };
    doc['friend-sync'] = {
      ...(doc['friend-sync'] ?? {}),
      'update-interval': model['friend-sync']['update-interval'],
      'auto-friend': model['friend-sync']['auto-friend'],
      'initial-invite': model['friend-sync']['initial-invite'],
      expiry: { ...(doc['friend-sync']?.['expiry'] ?? {}), ...model['friend-sync'].expiry },
    };
    doc['notifications'] = {
      ...(doc['notifications'] ?? {}),
      ...model.notifications,
    };
    doc['debug-mode'] = model['debug-mode'];
    doc['suppress-session-update-message'] = model['suppress-session-update-message'];
    return doc;
  }

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

  function fmtUptime(s: number): string {
    if (s < 60) return `${Math.floor(s)}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
    return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
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

  {#if status && !status.jar_exists}
    <div class="card border-red-500/40">
      <div class="flex items-start gap-3">
        <Radio size={18} class="text-red-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-red-300 uppercase tracking-wider">Broadcaster jar not found</p>
          <p class="font-mono break-all">{status.jar}</p>
          <p>Place <span class="font-mono text-bedrock-300">MCXboxBroadcastStandalone.jar</span> there (download it from the
            <a href="https://github.com/MCXboxBroadcast/Broadcaster/releases" target="_blank" rel="noreferrer" class="text-bedrock-400 hover:underline inline-flex items-center gap-1">
              releases page <ExternalLink size={10} /></a>) or set <span class="font-mono">BROADCASTER_JAR</span>.</p>
        </div>
      </div>
    </div>
  {/if}

  {#if status?.external && status.status.status !== 'running'}
    <div class="card border-yellow-500/40">
      <div class="flex items-start justify-between gap-3 flex-wrap">
        <div class="flex items-start gap-3">
          <Radio size={18} class="text-yellow-400 mt-0.5 shrink-0" />
          <div class="text-xs text-deep-300 space-y-1">
            <p class="font-bold text-yellow-300 uppercase tracking-wider">
              Running (external){status.external.orphaned ? ' — left over from a previous panel run' : ''}
            </p>
            <p>
              A broadcaster java is already running for this instance (PID
              <span class="font-mono text-strong">{status.external.pid}</span>) but the panel does not
              own it. Starting a second one would make both fight over the same Xbox session.
            </p>
          </div>
        </div>
        {#if canManage}
          <div class="flex gap-2">
            <button onclick={() => action('stop')} disabled={!!busy}
                    class="btn-ghost flex items-center gap-1.5 text-xs">
              <Square size={13} /> {busy === 'stop' ? 'Stopping…' : 'Stop external'}
            </button>
            <button onclick={() => action('takeover')} disabled={!!busy}
                    class="btn-primary flex items-center gap-1.5 text-xs">
              <RotateCw size={13} /> {busy === 'takeover' ? 'Taking over…' : 'Take over'}
            </button>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  {#if status?.relay && (status.relay.relay?.public || status.relay.legacy?.public)}
    <div class="card border-red-500/40">
      <div class="flex items-start gap-3">
        <TriangleAlert size={18} class="text-red-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-red-300 uppercase tracking-wider">Public IP in the relay target</p>
          <p>
            The broadcaster runs on the same host as the server, so the relay target must be
            <span class="font-mono text-strong">127.0.0.1</span>. A public IP there loops through the
            router (hairpin NAT is refused) and every friends-list join fails with
            <span class="font-mono">"Failed to relay offer … connection refused"</span>
            (client-side InitialConnection-81).
          </p>
          <p class="font-mono break-all text-deep-400">
            {#if status.relay.relay?.public}session.remote-address: {status.relay.relay.text}{/if}
            {#if status.relay.relay?.public && status.relay.legacy?.public} · {/if}
            {#if status.relay.legacy?.public}session-info.ip: {status.relay.legacy.text} (legacy){/if}
          </p>
        </div>
      </div>
    </div>
  {/if}

  {#if status?.ping?.ok === false}
    <div class="card border-yellow-500/40">
      <div class="flex items-start gap-3">
        <TriangleAlert size={18} class="text-yellow-400 mt-0.5 shrink-0" />
        <div class="text-xs text-deep-300 space-y-1">
          <p class="font-bold text-yellow-300 uppercase tracking-wider">
            Broadcaster cannot reach the server ping target
          </p>
          <p>
            The query ping against <span class="font-mono text-strong">{status.ping.target ?? '—'}</span>
            fails, so the session advertises stale player/world values
            (<span class="font-mono">"Failed to ping server, falling back to config values"</span> in the console).
            Check that the server is running and that the relay target points at this host's server-port.
          </p>
          {#if status.ping.failing_since}
            <p class="text-deep-400">failing since {new Date(status.ping.failing_since * 1000).toLocaleTimeString()}</p>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
    <!-- Status + controls -->
    <div class="card">
      <h2 class="card-header">Process</h2>
      <div class="space-y-3">
        {#if status}
          <div class="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
            <span class="text-deep-400">State</span>
            <span class="text-strong capitalize">
              {status.status.status}{status.external && status.status.status !== 'running' ? ' (external)' : ''}
            </span>
            <span class="text-deep-400">PID</span><span class="font-mono text-strong">{status.status.pid ?? '—'}</span>
            <span class="text-deep-400">Uptime</span><span class="font-mono text-strong">{status.status.status === 'running' ? fmtUptime(status.status.uptime) : '—'}</span>
            <span class="text-deep-400">Java</span>
            <span class="font-mono text-strong truncate" title={status.java ?? ''}>{status.java ?? 'not detected'}</span>
            <span class="text-deep-400">Relay target</span>
            <span class="font-mono text-strong truncate" title={status.relay?.relay?.text ?? ''}>
              {status.relay?.relay?.text ?? 'auto (127.0.0.1)'}
              {#if status.relay?.relay?.public}<span class="text-red-400"> public!</span>{/if}
            </span>
            <span class="text-deep-400">Ping</span>
            <span class="font-mono {status.ping?.ok === false ? 'text-red-400' : status.ping?.ok === true ? 'text-teal-400' : 'text-strong'}">
              {status.ping?.ok === true ? 'ok' : status.ping?.ok === false ? 'failing' : '—'}
            </span>
            <span class="text-deep-400">Dir</span>
            <span class="font-mono text-deep-300 break-all" title={status.dir}>{status.dir}</span>
            <span class="text-deep-400">Jar</span>
            <span class="font-mono text-deep-300 break-all" title={status.jar}>{status.jar}</span>
          </div>
        {:else}
          <p class="text-xs text-deep-400">Loading…</p>
        {/if}
        {#if canManage}
          <div class="flex gap-2 pt-1">
            <button onclick={() => action('start')} disabled={!!busy || status?.status.status === 'running' || !!status?.external}
                    class="btn-primary flex-1 flex items-center justify-center gap-1.5 text-xs">
              <Play size={13} /> Start
            </button>
            <button onclick={() => action('stop')} disabled={!!busy || (status?.status.status !== 'running' && !status?.external)}
                    class="btn-ghost flex-1 flex items-center justify-center gap-1.5 text-xs">
              <Square size={13} /> Stop
            </button>
            <button onclick={() => action('restart')} disabled={!!busy || status?.status.status !== 'running'}
                    class="btn-ghost flex items-center justify-center gap-1.5 text-xs px-3" title="Restart">
              <RotateCw size={13} />
            </button>
          </div>
          {#if status?.external && status.status.status !== 'running'}
            <p class="text-[10px] text-yellow-400 leading-snug">
              An external broadcaster (PID {status.external.pid}) is running — use the banner above
              to stop it or take over.
            </p>
          {/if}
        {/if}
      </div>
    </div>

    <!-- Device codes -->
    <div class="card">
      <h2 class="card-header flex items-center gap-2"><KeyRound size={13} /> Sign-in codes</h2>
      <div class="space-y-3">
        {#if Object.keys(codes).length === 0}
          <p class="text-xs text-deep-400">
            When an account needs Microsoft authentication, the code and verification link appear here and in the console below.
          </p>
        {/if}
        {#each Object.values(codes).sort((a, b) => b.at - a.at) as code (code.account)}
          <div class="border-2 border-bedrock-500/40 bg-bedrock-950/30 p-3 space-y-2">
            <div class="flex items-center justify-between">
              <span class="text-[10px] uppercase tracking-widest text-bedrock-300">{code.account}</span>
              <span class="text-[10px] text-deep-400">{new Date(code.at * 1000).toLocaleTimeString()}</span>
            </div>
            <div class="flex items-center gap-3">
              <code class="text-xl font-mono font-bold text-strong tracking-[0.2em]">{code.code}</code>
              <button onclick={() => copy(code.code)} class="btn-ghost p-1.5" title="Copy code"><Copy size={12} /></button>
            </div>
            <a href={code.url} target="_blank" rel="noreferrer"
               class="text-xs text-bedrock-400 hover:underline inline-flex items-center gap-1 break-all">
              {code.url} <ExternalLink size={10} class="shrink-0" />
            </a>
          </div>
        {/each}
      </div>
    </div>

    <!-- Accounts -->
    <div class="card">
      <h2 class="card-header flex items-center gap-2"><UserRound size={13} /> Accounts</h2>
      <div class="space-y-2">
        {#each status?.accounts ?? [] as account (account.id)}
          <div class="flex items-center justify-between px-3 py-2 border border-deep-600/30 bg-deep-900/40">
            <div class="flex items-center gap-2">
              <span class="font-mono text-xs text-strong">{account.id}</span>
              {#if account.main}<span class="text-[9px] uppercase tracking-wider bg-bedrock-500/20 text-bedrock-300 px-1.5 py-0.5">main</span>{/if}
            </div>
            {#if canManage && !account.main}
              <button onclick={() => accountAction('remove')} disabled={!newAccount.trim() || newAccount.trim() !== account.id}
                      class="btn-ghost p-1.5 text-red-400" title={`Type "${account.id}" above to enable removal`}>
                <Trash2 size={12} />
              </button>
            {/if}
          </div>
        {/each}
        {#if canManage}
          <div class="flex gap-2 pt-1">
            <input type="text" bind:value={newAccount} placeholder="account id"
                   class="input flex-1 text-xs font-mono" onkeydown={(e) => e.key === 'Enter' && accountAction('add')} />
            <button onclick={() => accountAction('add')} disabled={!newAccount.trim()}
                    class="btn-primary flex items-center gap-1 text-xs px-3">
              <Plus size={13} /> Add
            </button>
          </div>
          <p class="text-[10px] text-deep-400">Extra accounts become sub-sessions; each triggers its own sign-in code.</p>
        {/if}
      </div>
    </div>
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
        <div class="space-y-4 max-h-[28rem] overflow-y-auto pr-1">
          <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
            <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Session</legend>
            <div class="border border-bedrock-500/30 bg-bedrock-950/20 p-3 space-y-2">
              <p class="text-[10px] uppercase tracking-widest text-bedrock-300">Relay target (operational)</p>
              <div class="grid grid-cols-2 gap-3">
                <label class="space-y-1 block">
                  <span class="text-[10px] text-deep-400 uppercase tracking-wider">Relay address (session.remote-address)</span>
                  <input type="text" bind:value={cfg.session['remote-address']} onchange={markVisualDirty}
                         class="input w-full text-xs font-mono" />
                </label>
                <label class="space-y-1 block">
                  <span class="text-[10px] text-deep-400 uppercase tracking-wider">Relay port (session.remote-port)</span>
                  <input type="text" inputmode="numeric" bind:value={cfg.session['remote-port']} onchange={markVisualDirty}
                         class="input w-full text-xs font-mono" placeholder="auto or the server-port" />
                </label>
              </div>
              <p class="text-[10px] text-deep-400 leading-snug">
                Join offers are relayed here. The panel runs the broadcaster on the same host as the
                server, so keep <span class="font-mono text-bedrock-300">127.0.0.1</span> — a public IP
                fails every friends-list join (hairpin NAT is refused; client-side InitialConnection-81).
                The panel warns at start when a public IP is configured.
              </p>
            </div>
            <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Update interval (s)</span>
                <input type="number" min="20" bind:value={cfg.session['update-interval']} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Game mode</span>
                <select bind:value={cfg.session['session-info']['game-mode']} onchange={markVisualDirty}
                        class="input w-full text-xs">
                  {#each ['Survival', 'Creative', 'Adventure'] as mode (mode)}
                    <option value={mode}>{mode}</option>
                  {/each}
                </select>
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">ICE port min</span>
                <input type="number" min="0" max="65535" bind:value={cfg.session['ice-port-range'].min} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">ICE port max</span>
                <input type="number" min="0" max="65535" bind:value={cfg.session['ice-port-range'].max} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
            </div>
            <div class="flex flex-wrap gap-x-6 gap-y-2">
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg.session['query-server']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Query server
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg.session['web-query-fallback']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Web query fallback
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg.session['config-fallback']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Config fallback
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg.session['hide-old-sessions']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Hide old sessions
              </label>
            </div>
            <div class="grid grid-cols-2 md:grid-cols-4 gap-3 border-t border-deep-600/20 pt-3">
              <label class="space-y-1 block md:col-span-2">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Host name</span>
                <input type="text" bind:value={cfg.session['session-info']['host-name']} onchange={markVisualDirty}
                       class="input w-full text-xs" />
              </label>
              <label class="space-y-1 block md:col-span-2">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">World name</span>
                <input type="text" bind:value={cfg.session['session-info']['world-name']} onchange={markVisualDirty}
                       class="input w-full text-xs" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Players (fallback)</span>
                <input type="number" min="0" bind:value={cfg.session['session-info'].players} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Max players</span>
                <input type="number" min="1" bind:value={cfg.session['session-info']['max-players']} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
            </div>
          </fieldset>

          <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
            <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Friend sync</legend>
            <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Update interval (s)</span>
                <input type="number" min="20" bind:value={cfg['friend-sync']['update-interval']} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Expiry days</span>
                <input type="number" min="1" bind:value={cfg['friend-sync'].expiry.days} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Expiry check (s)</span>
                <input type="number" min="60" bind:value={cfg['friend-sync'].expiry.check} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
            </div>
            <div class="flex flex-wrap gap-x-6 gap-y-2">
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg['friend-sync']['auto-friend']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Auto-accept friends
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg['friend-sync']['initial-invite']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Initial invite
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg['friend-sync'].expiry.enabled} onchange={markVisualDirty} class="accent-bedrock-500" />
                Unfriend expired players
              </label>
            </div>
          </fieldset>

          <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
            <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Notifications</legend>
            <label class="flex items-center gap-2 text-xs text-deep-300">
              <input type="checkbox" bind:checked={cfg.notifications.enabled} onchange={markVisualDirty} class="accent-bedrock-500" />
              Send webhook notifications (Discord/Slack)
            </label>
            {#if cfg.notifications.enabled}
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Webhook URL <span class="text-deep-400">(add /slack for Discord)</span></span>
                <input type="text" bind:value={cfg.notifications['webhook-url']} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" placeholder="https://discord.com/api/webhooks/…/slack" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Session expired message <span class="text-deep-400">(%s link, %s code)</span></span>
                <textarea bind:value={cfg.notifications['session-expired-message']} oninput={markVisualDirty}
                          class="input w-full h-20 text-xs font-mono"></textarea>
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Friend restriction message</span>
                <input type="text" bind:value={cfg.notifications['friend-restriction-message']} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
            {/if}
          </fieldset>

          <fieldset class="border border-deep-600/30 p-3" disabled={!canManage}>
            <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Debug</legend>
            <div class="flex flex-wrap gap-x-6 gap-y-2">
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg['debug-mode']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Debug logging
              </label>
              <label class="flex items-center gap-2 text-xs text-deep-300">
                <input type="checkbox" bind:checked={cfg['suppress-session-update-message']} onchange={markVisualDirty} class="accent-bedrock-500" />
                Suppress "Updated session!" spam
              </label>
            </div>
          </fieldset>

          <p class="text-[10px] text-deep-400">
            Visual edits rewrite the document without YAML comments; unknown keys are preserved.
            {#if visualDirty}<span class="text-yellow-400">Pending visual changes will replace the raw text on save.</span>{/if}
          </p>
        </div>
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
