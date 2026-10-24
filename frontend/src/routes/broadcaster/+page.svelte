<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, getToken } from '$lib/api/client';
  import { wsManager } from '$lib/websocket';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { detectLevel } from '$lib/console';
  import type { BroadcasterStatus, DeviceCode, ConsoleLine } from '$types/index';
  import { load as yamlLoad, dump as yamlDump } from 'js-yaml';
  import {
    Play, Square, RotateCw, Send, Copy, ExternalLink, Plus, Trash2, Save,
    Upload, Radio, KeyRound, UserRound, FileCode, RefreshCw, Settings2, FileText
  } from '@lucide/svelte';

  /** Known config.yml shape (MCXboxBroadcast config-version 5). */
  interface BcVisualConfig {
    session: {
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
        ip: string;
        port: number;
      };
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
          ip: '127.0.0.1',
          port: 19132,
        },
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
    d.session['update-interval'] = num(s['update-interval'], 30);
    d.session['query-server'] = bool(s['query-server'], true);
    d.session['web-query-fallback'] = bool(s['web-query-fallback'], false);
    d.session['config-fallback'] = bool(s['config-fallback'], false);
    d.session['session-info']['host-name'] = str(info['host-name'], '');
    d.session['session-info']['world-name'] = str(info['world-name'], '');
    d.session['session-info']['game-mode'] = str(info['game-mode'], 'Survival');
    d.session['session-info']['players'] = num(info['players'], 0);
    d.session['session-info']['max-players'] = num(info['max-players'], 20);
    d.session['session-info']['ip'] = str(info['ip'], '127.0.0.1');
    d.session['session-info']['port'] = num(info['port'], 19132);
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
      'update-interval': model.session['update-interval'],
      'query-server': model.session['query-server'],
      'web-query-fallback': model.session['web-query-fallback'],
      'config-fallback': model.session['config-fallback'],
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
  let lines = $state<ConsoleLine[]>([]);
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
    lines.push({
      text,
      level: (level || detectLevel(text)) as ConsoleLine['level'],
      timestamp: Date.now() / 1000,
    });
    if (lines.length > 500) lines.splice(0, lines.length - 500);
    lines = lines;
    if (terminalEl) requestAnimationFrame(() => { terminalEl.scrollTop = terminalEl.scrollHeight; });
  }

  async function refresh() {
    try {
      status = await api.getBroadcasterStatus();
      codes = status.device_codes;
    } catch { /* transient */ }
  }

  onMount(async () => {
    await refresh();
    unsubs.push(wsManager.connect('/api/v1/broadcaster/ws', (data) => {
      if (data.type === 'hello' && data.resumed === false) lines = [];
      else if (data.type === 'console') {
        pushLine(data.line as string, data.level as string);
      } else if (data.type === 'status') {
        if (status) status.status = data.status as BroadcasterStatus['status'];
        else refresh();
      } else if (data.type === 'device_codes') {
        codes = data.codes as Record<string, DeviceCode>;
      }
    }));
    try {
      configText = await api.getBroadcasterConfig();
      parseConfigToModel();
    } catch (e: any) {
      addToast(`Config load failed: ${e.message}`, 'error');
    }
    poll = setInterval(refresh, 5000);
  });

  async function loadPreview(account: string) {
    try {
      const r = await fetch(`/api/v1/broadcaster/screenshot?account=${encodeURIComponent(account)}&token=${getToken()}`);
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

  async function action(kind: 'start' | 'stop' | 'restart') {
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

  <div class="grid grid-cols-1 lg:grid-cols-3 gap-4">
    <!-- Status + controls -->
    <div class="card">
      <h2 class="card-header">Process</h2>
      <div class="space-y-3">
        {#if status}
          <div class="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
            <span class="text-deep-400">State</span><span class="text-strong capitalize">{status.status.status}</span>
            <span class="text-deep-400">PID</span><span class="font-mono text-strong">{status.status.pid ?? '—'}</span>
            <span class="text-deep-400">Uptime</span><span class="font-mono text-strong">{status.status.status === 'running' ? fmtUptime(status.status.uptime) : '—'}</span>
            <span class="text-deep-400">Java</span>
            <span class="font-mono text-strong truncate" title={status.java ?? ''}>{status.java ?? 'not detected'}</span>
          </div>
        {:else}
          <p class="text-xs text-deep-400">Loading…</p>
        {/if}
        {#if canManage}
          <div class="flex gap-2 pt-1">
            <button onclick={() => action('start')} disabled={!!busy || status?.status.status === 'running'}
                    class="btn-primary flex-1 flex items-center justify-center gap-1.5 text-xs">
              <Play size={13} /> Start
            </button>
            <button onclick={() => action('stop')} disabled={!!busy || status?.status.status !== 'running'}
                    class="btn-ghost flex-1 flex items-center justify-center gap-1.5 text-xs">
              <Square size={13} /> Stop
            </button>
            <button onclick={() => action('restart')} disabled={!!busy}
                    class="btn-ghost flex items-center justify-center gap-1.5 text-xs px-3" title="Restart">
              <RotateCw size={13} />
            </button>
          </div>
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
    <div class="flex items-center justify-between">
      <h2 class="card-header flex items-center gap-2"><FileCode size={13} /> Console</h2>
      <button onclick={refresh} class="btn-ghost p-1.5" title="Refresh"><RefreshCw size={12} /></button>
    </div>
    <div bind:this={terminalEl}
         class="terminal-container h-64 overflow-y-auto p-4 font-terminal text-base leading-relaxed terminal-surface border-2 border-deep-600/50">
      {#each lines as line, i (i)}
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
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Server IP</span>
                <input type="text" bind:value={cfg.session['session-info'].ip} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
              </label>
              <label class="space-y-1 block">
                <span class="text-[10px] text-deep-400 uppercase tracking-wider">Port</span>
                <input type="number" min="1" max="65535" bind:value={cfg.session['session-info'].port} onchange={markVisualDirty}
                       class="input w-full text-xs font-mono" />
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
      <p class="text-[10px] text-deep-400 mt-2">
        Full MCXboxBroadcast config. Set <span class="font-mono text-deep-400">session.session-info.ip/port</span> to your
        Bedrock server, then restart. A restart of the panel process itself is not needed.
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
