<script lang="ts">
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import type { BackendInfo } from '$types/index';
  import { Eye, Plus, RotateCw, Save, Search } from '@lucide/svelte';

  type Entry = { key: string; value: string; comment: string; inline_comment: string };

  let properties = $state<Entry[]>([]);
  let rawText = $state('');
  let showRaw = $state(false);
  let loading = $state(true);
  let query = $state('');
  let newKey = $state('');
  let newValue = $state('');
  let backendInfo = $state<BackendInfo | null>(null);
  let canEdit = $derived($userPermissions.includes('PROPERTIES_EDIT'));
  // Bumps on every (re)load so a slow response for a previously selected
  // instance can never overwrite the freshly loaded one.
  let epoch = 0;

  // The Endstone toolchain is panel-wide, so one fetch serves every
  // instance; the version only feeds the port-forward advisor.
  $effect(() => {
    void api
      .getBackendInfo()
      .then((info) => (backendInfo = info))
      .catch(() => {});
  });

  async function load() {
    const mine = ++epoch;
    loading = true;
    showRaw = false;
    rawText = '';
    try {
      const list = await api.getProperties();
      if (mine !== epoch) return;
      properties = list;
    } catch (e: any) { addToast(`Failed to load properties: ${e.message}`, 'error'); }
    if (mine === epoch) loading = false;
  }

  // Properties are instance-scoped: reload whenever the active instance
  // changes. Keyed on the instance id — the layout's periodic list refresh
  // re-emits activeInstance with a fresh object even when the selection is
  // unchanged, and reloading on every emission would flicker the page.
  let loadedInstanceId: string | null = null;
  $effect(() => {
    const id = $activeInstance?.id ?? 'default';
    if (id === loadedInstanceId) return;
    loadedInstanceId = id;
    void load();
  });

  // Bumped whenever a save is refused so the table rows rebuild from disk
  // state: a plain reload assigns the same values Svelte already rendered,
  // and the user's typed (rejected) text would stay in the input.
  let rowEpoch = $state(0);

  async function updateValue(key: string, value: string) {
    const before = properties.find(p => p.key === key);
    if (!before || before.value === value) return;
    // server-udp-ports is validated against what the backend would actually
    // bind (same matrix the API enforces) so a broken forward rule never
    // saves silently and surfaces as "clients can't join" later.
    if (key === 'server-udp-ports') {
      const error = udpPortsError(value);
      if (error) {
        addToast(error, 'error');
        rowEpoch++;
        void load();
        return;
      }
    }
    try {
      await api.updateProperty(key, value);
      // Sync every row carrying this key (hand-edited files can hold dupes);
      // no reload needed — the backend write is the file's new truth.
      for (const p of properties) if (p.key === key) p.value = value;
      addToast(`Updated ${key}`, 'success');
    } catch (e: any) {
      addToast(`Update failed: ${e.message}`, 'error');
      rowEpoch++;
      void load();
    }
  }

  async function addProperty() {
    const key = newKey.trim();
    if (!key) return;
    try {
      await api.updateProperty(key, newValue);
      if (!properties.some(p => p.key === key)) {
        properties.push({ key, value: newValue, comment: '', inline_comment: '' });
      } else {
        for (const p of properties) if (p.key === key) p.value = newValue;
      }
      newKey = '';
      newValue = '';
      addToast(`Saved ${key}`, 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  async function loadRaw() {
    try {
      rawText = await api.getPropertiesRaw();
      showRaw = true;
    } catch (e: any) { addToast(`Failed to load: ${e.message}`, 'error'); }
  }

  async function saveRaw() {
    try {
      await api.savePropertiesRaw(rawText);
      properties = await api.getProperties();
      showRaw = false;
      addToast('Properties saved', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  // Widget schema. Known keys get predefined inputs; every other entry stays
  // fully editable as free text, and any entry holding true/false offers a
  // toggle — so every parsed line is configurable, whatever it is.
  const CHOICES: Record<string, string[]> = {
    gamemode: ['survival', 'creative', 'adventure'],
    difficulty: ['peaceful', 'easy', 'normal', 'hard'],
    'default-player-permission-level': ['visitor', 'member', 'operator'],
    'chat-restriction': ['None', 'Dropped', 'Disabled'],
    transport: ['raknet', 'nethernet'],
  };
  const BOOL_KEYS = new Set([
    'allow-cheats', 'online-mode', 'white-list', 'texturepack-required',
    'content-log-file-enabled', 'server-authoritative-movement',
    'enable-lan-visibility', 'disable-player-interaction',
  ]);
  const NUMBERS: Record<string, [number, number | null]> = {
    'max-players': [1, 1000],
    'server-port': [1, 65535],
    'server-portv6': [1, 65535],
    'view-distance': [2, 96],
    'tick-distance': [4, 12],
    'player-idle-timeout': [0, null],
    'compression-threshold': [0, 65535],
  };
  // Common keys float to the top when unfiltered; everything else keeps
  // file order beneath them.
  const COMMON = ['server-name', 'motd', 'gamemode', 'difficulty', 'allow-cheats', 'max-players',
    'online-mode', 'white-list', 'server-port', 'server-portv6', 'transport', 'server-udp-ports',
    'view-distance', 'tick-distance', 'level-name', 'level-seed',
    'default-player-permission-level', 'texturepack-required',
    'player-idle-timeout', 'content-log-file-enabled', 'compression-threshold',
    'server-authoritative-movement', 'enable-lan-visibility', 'chat-restriction',
    'disable-player-interaction'];

  type Widget =
    | { kind: 'enum'; options: string[] }
    | { kind: 'number'; min: number; max: number | null }
    | { kind: 'text' };

  function widgetFor(entry: Entry): Widget {
    if (BOOL_KEYS.has(entry.key)) return { kind: 'enum', options: ['true', 'false'] };
    if (CHOICES[entry.key]) return { kind: 'enum', options: CHOICES[entry.key] };
    if (NUMBERS[entry.key]) {
      const [min, max] = NUMBERS[entry.key];
      return { kind: 'number', min, max };
    }
    if (entry.value === 'true' || entry.value === 'false') {
      return { kind: 'enum', options: ['true', 'false'] };
    }
    return { kind: 'text' };
  }

  // ─── port-forward advisor ────────────────────────────────────────────────
  // What the instance's backend/transport combination actually binds, so the
  // router rule shown here is the one that works. Replaces the old blanket
  // "nethernet ignores server-port" warning, which has been wrong since
  // Endstone 0.11.15 multiplexed NetherNet onto server-port.
  const MULTIPLEXER_VERSION = '0.11.15';

  function compareVersions(a: string, b: string): number {
    const pa = a.split(/[-.+]/).map((p) => parseInt(p, 10) || 0);
    const pb = b.split(/[-.+]/).map((p) => parseInt(p, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const d = (pa[i] ?? 0) - (pb[i] ?? 0);
      if (d) return d;
    }
    return 0;
  }

  function propValue(key: string): string {
    return properties.find((p) => p.key === key)?.value?.trim() ?? '';
  }

  let backend = $derived(
    $activeInstance?.backend === 'vanilla' ? 'vanilla' : ('endstone' as 'endstone' | 'vanilla')
  );
  let transport = $derived(propValue('transport').toLowerCase() || 'nethernet');
  let serverPort = $derived(Number(propValue('server-port')) || 19132);
  let maxPlayers = $derived(Number(propValue('max-players')) || 10);
  let endstoneVersion = $derived(backendInfo?.endstone.installed?.endstone_version ?? null);
  let hasMultiplexer = $derived(
    !!endstoneVersion && compareVersions(endstoneVersion, MULTIPLEXER_VERSION) >= 0
  );

  type Advisor = { level: 'ok' | 'warn'; title: string; lines: string[] };

  let advisor = $derived.by<Advisor | null>(() => {
    if (loading || !properties.length) return null;
    if (transport === 'raknet') {
      return {
        level: 'warn',
        title: `RakNet transport — forward UDP ${serverPort} only`,
        lines: [
          'Fully updated Bedrock clients (1.26.60+) removed RakNet and CANNOT join this server.',
          'Switch transport to nethernet below for updated clients.',
        ],
      };
    }
    if (transport !== 'nethernet') {
      return {
        level: 'warn',
        title: `Unknown transport "${transport}"`,
        lines: ['Use raknet or nethernet — anything else falls back to the server default.'],
      };
    }
    if (backend === 'endstone' && hasMultiplexer) {
      return {
        level: 'ok',
        title: `NetherNet single-port mode — forward ONE rule: TCP+UDP ${serverPort}`,
        lines: [
          `Forward exactly one port on the router: TCP and UDP on ${serverPort} (server-port).`,
          'No per-player ports needed — Endstone multiplexes every player onto server-port.',
          'Start order matters: start the server FIRST. The Endstone launcher claims server-port at boot and exits with code 1 if this instance\'s broadcaster holds it.',
        ],
      };
    }
    if (backend === 'endstone') {
      return {
        level: 'warn',
        title: `Endstone ${endstoneVersion ?? '(version unknown)'} has no NetherNet multiplexer`,
        lines: [
          'Game traffic needs one UDP port per player slot from server-udp-ports — server-port is not the game port on this toolchain.',
          `Upgrade the Endstone toolchain to ${MULTIPLEXER_VERSION}+ (Settings page) to multiplex NetherNet onto one TCP+UDP port.`,
        ],
      };
    }
    return {
      level: 'warn',
      title: `Vanilla NetherNet — TCP ${serverPort} (signaling) + one UDP port per player slot`,
      lines: [
        `server-udp-ports must cover ${maxPlayers} UDP ports (one per player slot) on top of TCP ${serverPort}.`,
        'Home routers usually cannot forward port ranges as a single rule — the Endstone backend needs just one TCP+UDP port instead.',
      ],
    };
  });

  // Client-side mirror of the backend's server-udp-ports matrix (identical
  // rules: the API still rejects authoritatively).
  function badPort(entry: string, text: string): string | null {
    const t = text.trim();
    if (!/^\d+$/.test(t) || Number(t) < 1 || Number(t) > 65535) {
      return `server-udp-ports entry '${entry}': '${t || ''}' is not a UDP port (1-65535)`;
    }
    return null;
  }

  function udpPortsError(value: string): string | null {
    if (!value.trim()) {
      return 'server-udp-ports cannot be blank; delete the key instead of emptying it';
    }
    const nethernet = transport === 'nethernet';
    for (const raw of value.split(',')) {
      const entry = raw.trim();
      if (!entry) return 'server-udp-ports has an empty entry (check for stray commas)';
      // [ip:]external:internal — read from the right so IPv6 hosts survive.
      const groups = entry.split(':').reverse();
      if (groups.length >= 2) {
        const internal = groups[0];
        const external = groups[1];
        const err = badPort(entry, internal) ?? badPort(entry, external);
        if (err) return err;
        if (!nethernet) continue;
        if (backend === 'endstone') {
          if (Number(internal) !== serverPort) {
            return `Endstone multiplexes NetherNet onto server-port ${serverPort}: the mapping's internal port (the last number) must equal it`;
          }
        } else {
          return 'The [ip:]external:internal mapping form is Endstone-only syntax — vanilla BDS binds the listed ports directly (and ip:a-b:a-b ranges bind nothing)';
        }
        continue;
      }
      if (entry.includes('-')) {
        const [lo, hi] = entry.split('-');
        const err = badPort(entry, lo) ?? badPort(entry, hi);
        if (err) return err;
        if (Number(lo.trim()) > Number(hi.trim())) {
          return `server-udp-ports range ${entry} starts above its end`;
        }
        if (nethernet && backend === 'endstone') {
          return 'Ranges do nothing on Endstone: its NetherNet multiplexer needs ONE port (TCP+UDP on server-port), not a range';
        }
        continue;
      }
      const err = badPort(entry, entry);
      if (err) return err;
    }
    return null;
  }

  let entries = $derived(properties.filter(p => p.key));
  // One-line guidance under the transport and server-udp-ports inputs, so
  // picking a transport explains itself at the point of the pick.
  function hintFor(key: string): string | null {
    const nethernet = transport === 'nethernet';
    if (key === 'transport') {
      if (!nethernet) return 'Updated clients (1.26.60+) cannot join RakNet — nethernet recommended';
      if (backend === 'endstone' && hasMultiplexer) return 'Single port: TCP+UDP on server-port';
      return 'Per-player UDP ports on this toolchain — see the advisor above';
    }
    if (key === 'server-udp-ports') {
      if (!nethernet) return 'Inert on raknet (syntax-checked only)';
      if (backend === 'endstone') return 'Form: [ip:]external:internal with internal = server-port';
      return 'Ports or a-b ranges — one UDP port per player slot';
    }
    return null;
  }
  let visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const rows = q
      ? entries.filter(p => p.key.toLowerCase().includes(q) || p.value.toLowerCase().includes(q))
      : [...entries];
    const rank = (key: string) => {
      const i = COMMON.indexOf(key);
      return i === -1 ? COMMON.length : i;
    };
    return rows.sort((a, b) => rank(a.key) - rank(b.key));
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-strong uppercase tracking-widest">Server Properties</h1>
      <div class="pixel-divider mt-2 w-48"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadRaw} class="btn-secondary flex items-center gap-2 text-xs">
        <Eye size={14} /> Raw
      </button>
      <button onclick={load} class="btn-ghost p-2" title="Reload from disk"
              disabled={loading}><RotateCw size={14} /></button>
    </div>
  </div>

  {#if loading}
    <div class="card py-3 text-xs text-deep-400 uppercase tracking-wider">Loading properties…</div>
  {/if}

  {#if advisor}
    <div class="card {advisor.level === 'warn' ? 'border-yellow-500/40' : 'border-green-500/30'}">
      <h2 class="card-header">Port Forwarding</h2>
      <p class="text-xs font-medium {advisor.level === 'warn' ? 'text-yellow-400' : 'text-green-400'}">
        {advisor.title}
      </p>
      <ul class="mt-1 space-y-0.5 text-xs text-deep-300 list-disc list-inside">
        {#each advisor.lines as line (line)}
          <li>{line}</li>
        {/each}
      </ul>
      <p class="mt-2 text-[10px] text-deep-400">
        Based on this instance's backend{#if backend === 'endstone' && endstoneVersion}&nbsp;(Endstone {endstoneVersion}){/if},
        transport, server-port, max-players, and server-udp-ports below. Changing any of them updates this advice.
      </p>
    </div>
  {/if}

  {#if showRaw}
    <div class="card">
      <h2 class="card-header">Raw Editor</h2>
      <textarea bind:value={rawText} class="input w-full h-96 font-mono text-sm" spellcheck="false"></textarea>
      <div class="flex gap-2 mt-3">
        {#if canEdit}
          <button onclick={saveRaw} class="btn-primary flex items-center gap-2 text-xs"><Save size={14} /> Save</button>
        {/if}
        <button onclick={() => showRaw = false} class="btn-secondary text-xs">Cancel</button>
      </div>
    </div>
  {:else}
    {#if canEdit}
      <div class="card">
        <h2 class="card-header">Add Property</h2>
        <form class="flex flex-wrap items-center gap-2"
              onsubmit={(e) => { e.preventDefault(); void addProperty(); }}>
          <input type="text" placeholder="key (e.g. education-feature)" bind:value={newKey}
                 class="input flex-1 min-w-44 text-xs py-1.5 font-mono" />
          <input type="text" placeholder="value" bind:value={newValue}
                 class="input flex-1 min-w-32 text-xs py-1.5" />
          <button type="submit" class="btn-primary flex items-center gap-2 text-xs"
                  disabled={!newKey.trim()}>
            <Plus size={14} /> Add
          </button>
        </form>
        <p class="mt-2 text-xs text-deep-400">
          New keys are appended to server.properties; existing keys are updated in place.
        </p>
      </div>
    {/if}

    <div class="card">
      <div class="flex items-center justify-between flex-wrap gap-2">
        <h2 class="card-header">Properties</h2>
        <div class="flex items-center gap-2">
          <span class="text-xs text-deep-400">{visible.length} of {entries.length}</span>
          <div class="relative">
            <Search size={12} class="absolute left-2 top-1/2 -translate-y-1/2 text-deep-400" />
            <input type="text" placeholder="Filter key or value…" bind:value={query}
                   class="input pl-7 w-56 text-xs py-1.5" />
          </div>
        </div>
      </div>
      <div class="overflow-x-auto mt-2">
        <table class="w-full text-xs">
          <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
            <th class="text-left py-2 px-3 font-medium w-72">Key</th>
            <th class="text-left py-2 px-3 font-medium">Value</th>
            <th class="text-left py-2 px-3 font-medium">Comment</th>
          </tr></thead>
          <tbody>
            {#each visible as entry (entry.key + '#' + rowEpoch)}
              {@const widget = widgetFor(entry)}
              <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
                <td class="py-1.5 px-3 font-mono text-bedrock-400 whitespace-nowrap">{entry.key}</td>
                <td class="py-1.5 px-3 w-1/2">
                  {#if !canEdit}
                    <span>{entry.value}</span>
                  {:else if widget.kind === 'enum'}
                    <select value={entry.value}
                            onchange={(e) => updateValue(entry.key, (e.target as HTMLSelectElement).value)}
                            class="input w-full text-xs py-1">
                      {#if !widget.options.includes(entry.value)}
                        <option value={entry.value}>{entry.value} (current)</option>
                      {/if}
                      {#each widget.options as o}
                        <option value={o}>{o}</option>
                      {/each}
                    </select>
                  {:else if widget.kind === 'number'}
                    <input type="number" value={entry.value}
                           min={widget.min} max={widget.max ?? undefined}
                           onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)}
                           class="input w-full text-xs py-1" />
                  {:else}
                    <input type="text" value={entry.value}
                           onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)}
                           class="input w-full text-xs py-1" />
                  {/if}
                  {#if canEdit && hintFor(entry.key)}
                    <p class="text-[10px] text-deep-400 mt-0.5">{hintFor(entry.key)}</p>
                  {/if}
                </td>
                <td class="py-1.5 px-3 text-deep-400">{entry.inline_comment}</td>
              </tr>
            {:else}
              <tr><td colspan="3" class="py-3 px-3 text-deep-400 text-center">
                {query.trim() ? 'No entries match the filter.' : 'No properties found.'}
              </td></tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="mt-2 text-xs text-deep-400">
        Every entry saves to the instance's server.properties on change. Full-line comments and
        blank lines are preserved — edit them via Raw.
      </p>
    </div>
  {/if}
</div>
