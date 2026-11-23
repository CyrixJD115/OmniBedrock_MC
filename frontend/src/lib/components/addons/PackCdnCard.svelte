<script lang="ts">
  /**
   * Optional pack CDN. BDS can point joining clients at an HTTP(S) URL for
   * each resource pack (cdn_config.json) instead of streaming the packs
   * through the game connection; the panel builds the zips, serves them on
   * its own port under /packs/, and writes the file. It speeds the download
   * up (consoles especially); it does not change how long a client keeps
   * the packs. Undocumented by Mojang, so strictly opt-in and reversible.
   */
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import type { PackCdnStatus, PackCdnTest } from '$types/index';
  import { Zap, ChevronDown, ChevronRight, ShieldAlert, Gamepad2, CircleCheck, CircleX, CircleDashed, HelpCircle } from '@lucide/svelte';

  let { canManage, instanceId }: { canManage: boolean; instanceId: string } = $props();

  let status = $state<PackCdnStatus | null>(null);
  let open = $state(false);
  let enabled = $state(false);
  let baseUrl = $state('');
  let busy = $state('');
  let test = $state<PackCdnTest | null>(null);

  function mb(bytes: number): string {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  async function load() {
    const forId = instanceId;
    try {
      const s = await api.getPackCdn();
      if (forId !== instanceId) return;
      status = s;
      enabled = s.state.settings.enabled;
      baseUrl = s.state.settings.base_url;
    } catch { /* optional feature */ }
  }

  $effect(() => {
    void instanceId;
    status = null;
    test = null;
    void load();
  });

  async function save() {
    if (busy) return;
    busy = 'save';
    test = null;
    try {
      const s = await api.savePackCdn(enabled, baseUrl.trim());
      status = s;
      addToast(s.message ?? 'Saved', 'success', 6000);
    } catch (e: any) {
      addToast(`Pack CDN: ${e.message}`, 'error', 7000);
    }
    busy = '';
  }

  async function runTest() {
    if (busy) return;
    busy = 'test';
    try {
      test = await api.testPackCdn();
    } catch (e: any) {
      addToast(`Test failed: ${e.message}`, 'error');
    }
    busy = '';
  }

  let explain = $state(false);

  /** Client-side check of the public base URL (the API validates too). */
  const urlProblem = $derived.by(() => {
    const v = baseUrl.trim();
    if (!v) return enabled ? 'Enter the public URL players reach the panel at' : '';
    let u: URL;
    try { u = new URL(v); } catch { return 'Not a valid URL'; }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return 'Use http:// or https://';
    if (/^(localhost|127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(u.hostname)) return 'This address is not reachable from the internet';
    if (u.pathname.replace(/\/$/, '').endsWith('/packs')) return 'Leave out /packs; the panel adds it';
    return '';
  });

  type Step = { label: string; state: 'done' | 'todo' | 'fail'; detail?: string };
  const steps = $derived.by<Step[]>(() => {
    if (!status) return [];
    const st = status.state;
    const on = st.settings.enabled;
    const loaded = status.bds?.status === 'loaded' || status.bds?.status === 'accepted';
    const bdsFail = status.bds?.status === 'mismatch' || status.bds?.status === 'parse_error';
    return [
      { label: 'Public URL set', state: st.settings.base_url ? 'done' : 'todo', detail: st.settings.base_url || undefined },
      { label: 'Pack zips built', state: on && st.files > 0 ? 'done' : 'todo', detail: st.files ? `${st.files} · ${mb(st.total_bytes)}` : undefined },
      { label: 'cdn_config.json written', state: st.config_present && st.config_owned ? 'done' : st.config_present ? 'fail' : 'todo',
        detail: st.config_present && !st.config_owned ? 'hand-made file present; the panel will not overwrite it' : undefined },
      { label: 'Every active resource pack covered', state: !on ? 'todo' : st.uncovered.length ? 'fail' : 'done',
        detail: on && st.uncovered.length ? `${st.uncovered.length} missing: Bedrock ignores the whole file` : undefined },
      { label: 'Bedrock loaded it (restart after changes)', state: loaded ? 'done' : bdsFail ? 'fail' : 'todo', detail: status.bds?.line },
      { label: 'Reachable from the internet', state: test ? (test.ok ? 'done' : 'fail') : 'todo', detail: test ? test.detail : 'Run "Test URL"' },
    ];
  });

  const badge = $derived.by(() => {
    if (!status) return null;
    const st = status.state;
    if (!st.settings.enabled) return { cls: 'badge-info', text: 'off' };
    if (status.bds?.status === 'loaded' || status.bds?.status === 'accepted') return { cls: 'badge-running', text: 'active' };
    if (status.bds?.status === 'mismatch') return { cls: 'badge-stopped', text: 'BDS: mismatch' };
    if (status.bds?.status === 'parse_error') return { cls: 'badge-stopped', text: 'BDS: parse error' };
    if (st.config_present && st.uncovered.length === 0) return { cls: 'badge-warn', text: 'ready — restart server' };
    return { cls: 'badge-warn', text: 'incomplete' };
  });
</script>

{#if status}
  <div class="card !py-3">
    <button class="w-full flex items-center justify-between gap-3 text-left" onclick={() => (open = !open)}>
      <span class="flex items-center gap-2 text-xs flex-wrap">
        <Zap size={14} class="text-bedrock-400" />
        <span class="uppercase tracking-wider text-strong font-semibold">Pack CDN</span>
        <span class="text-deep-400">optional · faster pack downloads for console players</span>
        {#if badge}<span class="{badge.cls} !text-[9px] !py-0.5">{badge.text}</span>{/if}
      </span>
      {#if open}<ChevronDown size={14} class="text-deep-400" />{:else}<ChevronRight size={14} class="text-deep-400" />{/if}
    </button>

    {#if open}
      <div class="mt-3 space-y-3 text-xs">
        <ol class="mc-slot p-3 space-y-1.5">
          {#each steps as step, i (i)}
            <li class="flex items-start gap-2 text-[11px]">
              {#if step.state === 'done'}<CircleCheck size={13} class="text-teal-400 shrink-0 mt-px" />
              {:else if step.state === 'fail'}<CircleX size={13} class="text-red-400 shrink-0 mt-px" />
              {:else}<CircleDashed size={13} class="text-deep-400 shrink-0 mt-px" />{/if}
              <span class="min-w-0">
                <span class={step.state === 'done' ? 'text-strong' : step.state === 'fail' ? 'text-red-200' : 'text-deep-300'}>{step.label}</span>
                {#if step.detail}<span class="block font-mono text-[10px] text-deep-400 break-all">{step.detail}</span>{/if}
              </span>
            </li>
          {/each}
        </ol>

        <button class="flex items-center gap-1.5 text-[11px] text-bedrock-300 hover:text-bedrock-200" onclick={() => (explain = !explain)} aria-expanded={explain}>
          <HelpCircle size={12} /> What is this?
        </button>
        {#if explain}
        <p class="text-deep-300 leading-snug">
          Bedrock servers can hand joining players a web link for each resource pack instead of sending the packs
          through the game connection, which is slow on consoles. With this on, the panel zips the world's resource
          packs, serves them at <span class="font-mono">/packs/…</span> on this same port, and writes
          <span class="font-mono">cdn_config.json</span>. It makes the download fast; it does not stop a console that
          cleared its cache from downloading again. Bedrock reads the file at boot, so restart the server after
          changes. This uses an undocumented Bedrock feature — if anything misbehaves, turn it off and packs go back
          to in-game transfer.
        </p>
        {/if}

        <div class="flex items-start gap-2 border-2 border-yellow-500/30 bg-yellow-500/5 p-2 text-[11px] text-yellow-200/90 leading-snug">
          <ShieldAlert size={14} class="shrink-0 mt-0.5" />
          <span>
            The URL must be reachable by your players from the internet. Forwarding the panel port
            ({status.panel_port}) also exposes the panel's login page — prefer a reverse proxy or tunnel
            (Cloudflare Tunnel, Caddy, nginx) that forwards only <span class="font-mono">/packs/</span> to this panel, with HTTPS.
          </span>
        </div>

        <div class="flex flex-wrap items-end gap-3">
          <label class="flex items-center gap-2 text-xs text-deep-300">
            <input type="checkbox" bind:checked={enabled} disabled={!canManage || !!busy} />
            Enable for this server
          </label>
          <label class="space-y-1 block flex-1 min-w-64">
            <span class="text-[10px] text-deep-400 uppercase tracking-wider">Public base URL</span>
            <input bind:value={baseUrl} disabled={!canManage || !!busy} placeholder="https://packs.example.com"
                   class="input w-full text-xs font-mono {urlProblem ? '!border-yellow-500/60' : ''}" aria-invalid={!!urlProblem} />
            {#if urlProblem}<span class="block text-[10px] text-yellow-300">{urlProblem}</span>{/if}
          </label>
          {#if canManage}
            <button onclick={save} disabled={!!busy || (enabled && (!baseUrl.trim() || urlProblem === 'Not a valid URL'))} class="btn-primary text-xs px-3 py-2">
              {busy === 'save' ? 'Building zips…' : 'Save'}
            </button>
            <button onclick={runTest} disabled={!!busy || !status.state.settings.enabled} class="btn-secondary text-xs px-3 py-2">
              {busy === 'test' ? 'Testing…' : 'Test URL'}
            </button>
          {/if}
        </div>
        {#if status.suggestions.length && canManage}
          <div class="flex flex-wrap items-center gap-2 text-[10px] text-deep-400">
            Suggested (your server-udp-ports address):
            {#each status.suggestions as s (s)}
              <button class="font-mono text-bedrock-300 hover:underline" onclick={() => (baseUrl = s)}>{s}</button>
            {/each}
          </div>
        {/if}

        {#if test}
          <p class="text-[11px] {test.ok ? 'text-teal-300' : 'text-yellow-300'} break-all">
            {test.ok ? '✔' : '✖'} {test.detail}
            <span class="block font-mono text-[10px] text-deep-400">
              {test.url} · HTTP {test.status ?? '—'} · {test.content_type ?? 'no type'} · {test.content_length ?? '?'}/{test.expected_length} bytes
            </span>
          </p>
        {/if}

        <div class="flex items-start gap-2 text-[11px] text-deep-300 leading-snug">
          <Gamepad2 size={14} class="shrink-0 mt-0.5 text-bedrock-400" />
          <span>
            Tips for console players: leaving and rejoining <b>without fully closing Minecraft</b> keeps the downloaded
            packs; on PlayStation, server packs live under Settings → Storage → Cached Data — don't clear them; and a
            smaller pack stack (see Client download above) helps every join.
          </span>
        </div>
      </div>
    {/if}
  </div>
{/if}
