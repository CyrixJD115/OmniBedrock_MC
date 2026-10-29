<!-- Manifest preview drawer: structured/raw manifest view plus quick actions
     for the selected addon. -->
<script lang="ts">
  import {
    Package, X, Edit, Hash, FolderOpen, Shuffle, AlertTriangle, Trash2, Power, PowerOff,
  } from '@lucide/svelte';
  import { authsrc } from '$lib/api/client';
  import { structuredHtml, versionStr } from '$lib/addons/display';
  import ActivePill from './ActivePill.svelte';
  import SourceBadge from './SourceBadge.svelte';
  import type { Addon } from '$types/index';

  let {
    addon,
    canManage,
    active,
    hidden,
    icon,
    oniconbroken,
    viewMode = $bindable<'structured' | 'raw'>('structured'),
    wrap = $bindable(true),
    onclose,
    onactivate,
    ondeactivate,
    oneditmanifest,
    oneditversion,
    onrename,
    onchangeuuid,
    onrandomizeuuid,
    ontogglehidden,
    ondelete,
  }: {
    addon: Addon;
    canManage: boolean;
    active: boolean;
    hidden: boolean;
    icon: string;
    oniconbroken: () => void;
    viewMode?: 'structured' | 'raw';
    wrap?: boolean;
    onclose: () => void;
    onactivate: () => void;
    ondeactivate: () => void;
    oneditmanifest: () => void;
    oneditversion: () => void;
    onrename: () => void;
    onchangeuuid: () => void;
    onrandomizeuuid: () => void;
    ontogglehidden: () => void;
    ondelete: () => void;
  } = $props();
</script>

<!-- backdrop on mobile -->
<button type="button" class="fixed inset-0 z-40 bg-black/40 lg:hidden cursor-pointer" onclick={onclose} aria-label="Close preview"></button>
<!-- drawer -->
<aside class="fixed top-0 right-0 z-50 h-full w-full max-w-md bg-deep-900 border-l-2 border-deep-600/50 shadow-2xl flex flex-col animate-slide-in">
  <!-- drawer header -->
  <div class="flex items-center justify-between p-4 border-b border-deep-700/50 shrink-0">
    {#if icon}
      <img use:authsrc={{ url: icon, onerror: oniconbroken }} alt=""
           class="pack-icon w-10 h-10 rounded object-cover bg-deep-800 mr-3 shrink-0 border border-deep-700/60"
           onerror={oniconbroken} />
    {:else}
      <div class="w-10 h-10 rounded bg-deep-800/80 flex items-center justify-center mr-3 shrink-0">
        <Package size={18} class="text-deep-400" />
      </div>
    {/if}
    <div class="min-w-0">
      <p class="text-[10px] text-deep-400 font-mono uppercase tracking-wider">
        {addon.pack_type === 'behavior_packs' ? 'Behavior Pack' : 'Resource Pack'} · v{versionStr(addon.version)}
      </p>
      <div class="mt-0.5 flex items-center gap-1.5 flex-wrap">
        <SourceBadge addon={addon} />
        <ActivePill active={active} />
        {#if addon.source === 'curseforge' && addon.store?.project_name}
          <span class="text-[10px] text-deep-400 truncate" title={`CurseForge project ${String(addon.store.project_id)}`}>{String(addon.store.project_name)}</span>
        {/if}
      </div>
    </div>
    <button onclick={onclose} class="btn-ghost p-1.5 shrink-0" title="Close"><X size={16} /></button>
  </div>

  <!-- drawer toolbar -->
  <div class="flex items-center gap-2 p-3 border-b border-deep-700/30 shrink-0">
    <select bind:value={viewMode} class="input text-xs py-1 w-28 shrink-0">
      <option value="structured">Structured</option>
      <option value="raw">Raw JSON</option>
    </select>
    <label class="flex items-center gap-1 text-xs text-deep-400 cursor-pointer select-none shrink-0">
      <input type="checkbox" bind:checked={wrap} class="accent-bedrock-500" />
      Wrap
    </label>
    <div class="flex-1"></div>
    {#if canManage}
      {#if active}
        <button onclick={ondeactivate} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Remove from the world's pack order">
          <PowerOff size={12} /> Deactivate
        </button>
      {:else}
        <button onclick={onactivate} class="btn-ghost text-xs px-2 py-1 rounded border border-green-500/30 text-green-400 flex items-center gap-1" title="Add to the world's pack order">
          <Power size={12} /> Activate
        </button>
      {/if}
      <button onclick={oneditmanifest} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Edit manifest">
        <Edit size={12} /> Edit
      </button>
      <button onclick={oneditversion} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Edit version">
        <Hash size={12} /> Ver
      </button>
      <button onclick={onrename} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1" title="Rename">
        <FolderOpen size={12} /> Rename
      </button>
    {/if}
  </div>

  <!-- drawer body -->
  <div class="flex-1 overflow-y-auto p-4">
    {#if viewMode === 'structured'}
      <div class="bg-deep-950/50 rounded border border-deep-600/30 p-3">
        {@html structuredHtml(addon)}
      </div>
    {:else}
      <pre class="text-xs font-mono bg-deep-950/50 p-3 border border-deep-600/30 rounded
                 {wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre overflow-x-auto'}">{JSON.stringify(addon.manifest ?? {}, null, 2)}</pre>
    {/if}
  </div>

  <!-- drawer footer with quick actions -->
  {#if canManage}
    <div class="p-3 border-t border-deep-700/50 shrink-0 flex flex-wrap gap-1.5">
      <button onclick={onchangeuuid} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
        <Edit size={11} /> Change UUID
      </button>
      <button onclick={onrandomizeuuid} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1">
        <Shuffle size={11} /> Randomize
      </button>
      <button onclick={ontogglehidden} class="btn-ghost text-xs px-2 py-1 rounded border border-deep-600/30 flex items-center gap-1"
              title="Toggle hidden state">
        <AlertTriangle size={11} /> {hidden ? 'Unhide' : 'Hide'}
      </button>
      <button onclick={ondelete}
              class="btn-ghost text-xs px-2 py-1 rounded border border-red-500/30 text-red-400 flex items-center gap-1 ml-auto">
        <Trash2 size={11} /> Delete
      </button>
    </div>
  {/if}
</aside>

<style>
  @keyframes slide-in {
    from { transform: translateX(100%); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }
  .animate-slide-in {
    animation: slide-in 0.2s ease-out;
  }
  /* Minecraft pack icons are pixel art — keep them crisp when scaled up. */
  .pack-icon {
    image-rendering: pixelated;
  }
</style>
