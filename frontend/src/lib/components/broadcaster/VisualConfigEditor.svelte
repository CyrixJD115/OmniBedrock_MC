<script lang="ts">
  /**
   * Visual (form) editor for the broadcaster's config.yml: session, friend
   * sync, notifications, and debug fields bound onto the shared form model.
   * Every edit is reported through `onmarkdirty`; the raw YAML document and
   * its round-tripping live in the page, so unknown keys never pass through.
   */
  import type { BcVisualConfig } from '$lib/broadcaster/configModel';

  let { cfg = $bindable(), visualDirty, canManage, onmarkdirty }: {
    cfg: BcVisualConfig;
    visualDirty: boolean;
    canManage: boolean;
    onmarkdirty: () => void;
  } = $props();
</script>

<div class="space-y-4 max-h-[28rem] overflow-y-auto pr-1">
  <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
    <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Session</legend>
    <div class="border border-bedrock-500/30 bg-bedrock-950/20 p-3 space-y-2">
      <p class="text-[10px] uppercase tracking-widest text-bedrock-300">Relay target (operational)</p>
      <div class="grid grid-cols-2 gap-3">
        <label class="space-y-1 block">
          <span class="text-[10px] text-deep-400 uppercase tracking-wider">Relay address (session.remote-address)</span>
          <input type="text" bind:value={cfg.session['remote-address']} onchange={onmarkdirty}
                 class="input w-full text-xs font-mono" />
        </label>
        <label class="space-y-1 block">
          <span class="text-[10px] text-deep-400 uppercase tracking-wider">Relay port (session.remote-port)</span>
          <input type="text" inputmode="numeric" bind:value={cfg.session['remote-port']} onchange={onmarkdirty}
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
        <input type="number" min="20" bind:value={cfg.session['update-interval']} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Game mode</span>
        <select bind:value={cfg.session['session-info']['game-mode']} onchange={onmarkdirty}
                class="input w-full text-xs">
          {#each ['Survival', 'Creative', 'Adventure'] as mode (mode)}
            <option value={mode}>{mode}</option>
          {/each}
        </select>
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">ICE port min</span>
        <input type="number" min="0" max="65535" bind:value={cfg.session['ice-port-range'].min} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">ICE port max</span>
        <input type="number" min="0" max="65535" bind:value={cfg.session['ice-port-range'].max} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
    </div>
    <div class="flex flex-wrap gap-x-6 gap-y-2">
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg.session['query-server']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Query server
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg.session['web-query-fallback']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Web query fallback
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg.session['config-fallback']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Config fallback
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg.session['hide-old-sessions']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Hide old sessions
      </label>
    </div>
    <div class="grid grid-cols-2 md:grid-cols-4 gap-3 border-t border-deep-600/20 pt-3">
      <label class="space-y-1 block md:col-span-2">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Host name</span>
        <input type="text" bind:value={cfg.session['session-info']['host-name']} onchange={onmarkdirty}
               class="input w-full text-xs" />
      </label>
      <label class="space-y-1 block md:col-span-2">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">World name</span>
        <input type="text" bind:value={cfg.session['session-info']['world-name']} onchange={onmarkdirty}
               class="input w-full text-xs" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Players (fallback)</span>
        <input type="number" min="0" bind:value={cfg.session['session-info'].players} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Max players</span>
        <input type="number" min="1" bind:value={cfg.session['session-info']['max-players']} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
    </div>
  </fieldset>

  <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
    <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Friend sync</legend>
    <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Update interval (s)</span>
        <input type="number" min="20" bind:value={cfg['friend-sync']['update-interval']} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Expiry days</span>
        <input type="number" min="1" bind:value={cfg['friend-sync'].expiry.days} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Expiry check (s)</span>
        <input type="number" min="60" bind:value={cfg['friend-sync'].expiry.check} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
    </div>
    <div class="flex flex-wrap gap-x-6 gap-y-2">
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg['friend-sync']['auto-friend']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Auto-accept friends
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg['friend-sync']['initial-invite']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Initial invite
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg['friend-sync'].expiry.enabled} onchange={onmarkdirty} class="accent-bedrock-500" />
        Unfriend expired players
      </label>
    </div>
  </fieldset>

  <fieldset class="border border-deep-600/30 p-3 space-y-3" disabled={!canManage}>
    <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Notifications</legend>
    <label class="flex items-center gap-2 text-xs text-deep-300">
      <input type="checkbox" bind:checked={cfg.notifications.enabled} onchange={onmarkdirty} class="accent-bedrock-500" />
      Send webhook notifications (Discord/Slack)
    </label>
    {#if cfg.notifications.enabled}
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Webhook URL <span class="text-deep-400">(add /slack for Discord)</span></span>
        <input type="text" bind:value={cfg.notifications['webhook-url']} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" placeholder="https://discord.com/api/webhooks/…/slack" />
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Session expired message <span class="text-deep-400">(%s link, %s code)</span></span>
        <textarea bind:value={cfg.notifications['session-expired-message']} oninput={onmarkdirty}
                  class="input w-full h-20 text-xs font-mono"></textarea>
      </label>
      <label class="space-y-1 block">
        <span class="text-[10px] text-deep-400 uppercase tracking-wider">Friend restriction message</span>
        <input type="text" bind:value={cfg.notifications['friend-restriction-message']} onchange={onmarkdirty}
               class="input w-full text-xs font-mono" />
      </label>
    {/if}
  </fieldset>

  <fieldset class="border border-deep-600/30 p-3" disabled={!canManage}>
    <legend class="px-2 text-[10px] uppercase tracking-widest text-bedrock-300">Debug</legend>
    <div class="flex flex-wrap gap-x-6 gap-y-2">
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg['debug-mode']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Debug logging
      </label>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={cfg['suppress-session-update-message']} onchange={onmarkdirty} class="accent-bedrock-500" />
        Suppress "Updated session!" spam
      </label>
    </div>
  </fieldset>

  <p class="text-[10px] text-deep-400">
    Visual edits rewrite the document without YAML comments; unknown keys are preserved.
    {#if visualDirty}<span class="text-yellow-400">Pending visual changes will replace the raw text on save.</span>{/if}
  </p>
</div>
