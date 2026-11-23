<script lang="ts">
  /**
   * Settings, grouped into sections with a side nav (top tabs on small
   * screens). The section is kept in the URL hash so links and reloads land
   * on the same place. Each section is its own component.
   */
  import { onMount } from 'svelte';
  import { userPermissions } from '$stores/auth';
  import { activeInstance } from '$stores/instances';
  import AppearanceCard from '$lib/components/settings/AppearanceCard.svelte';
  import BackendCard from '$lib/components/settings/BackendCard.svelte';
  import SupervisorCard from '$lib/components/settings/SupervisorCard.svelte';
  import AccessCard from '$lib/components/settings/AccessCard.svelte';
  import ApiKeysCard from '$lib/components/settings/ApiKeysCard.svelte';
  import CurseForgeCard from '$lib/components/settings/CurseForgeCard.svelte';
  import AppInfoCard from '$lib/components/settings/AppInfoCard.svelte';
  import { Palette, Server, ShieldCheck, Plug, Info } from '@lucide/svelte';

  const SECTIONS = [
    { id: 'appearance', label: 'Appearance', icon: Palette, hint: 'Theme, mode, density' },
    { id: 'server', label: 'Server', icon: Server, hint: 'Runtime, version, supervisor' },
    { id: 'security', label: 'Security', icon: ShieldCheck, hint: 'Login and API keys' },
    { id: 'integrations', label: 'Integrations', icon: Plug, hint: 'CurseForge store' },
    { id: 'about', label: 'About', icon: Info, hint: 'Panel configuration' },
  ] as const;
  type Section = (typeof SECTIONS)[number]['id'];

  let section = $state<Section>('appearance');
  const canManage = $derived($userPermissions.includes('SERVER_MANAGE'));
  const canEditSettings = $derived($userPermissions.includes('SETTINGS_EDIT'));

  onMount(() => {
    const fromHash = location.hash.slice(1) as Section;
    if (SECTIONS.some((s) => s.id === fromHash)) section = fromHash;
  });

  function go(id: Section) {
    section = id;
    history.replaceState(history.state, '', `#${id}`);
  }
</script>

<div class="space-y-4">
  <div>
    <h1 class="page-title">Settings</h1>
    <div class="pixel-divider mt-2 w-24"></div>
  </div>

  <div class="grid grid-cols-1 lg:grid-cols-[14rem_1fr] gap-4 items-start">
    <nav class="card !p-2 lg:sticky lg:top-4 flex lg:flex-col gap-1 overflow-x-auto" aria-label="Settings sections">
      {#each SECTIONS as s (s.id)}
        <button onclick={() => go(s.id)} aria-current={section === s.id ? 'page' : undefined}
                class="flex items-center gap-2.5 px-3 py-2 text-left whitespace-nowrap transition-colors border-l-2
                       {section === s.id ? 'bg-bedrock-500/15 border-bedrock-400 text-strong' : 'border-transparent text-deep-300 hover:bg-deep-800/50 hover:text-deep-100'}">
          <s.icon size={14} class="shrink-0" />
          <span class="min-w-0">
            <span class="block text-xs font-semibold uppercase tracking-wider">{s.label}</span>
            <span class="hidden lg:block text-[10px] text-deep-400 normal-case tracking-normal truncate">{s.hint}</span>
          </span>
        </button>
      {/each}
    </nav>

    <div class="space-y-4 min-w-0">
      {#if section === 'appearance'}
        <AppearanceCard />
      {:else if section === 'server'}
        {#if $activeInstance}
          <p class="text-[11px] text-deep-400">These apply to the active instance, <b class="text-strong">{$activeInstance.name}</b>. Switch instances in the sidebar.</p>
        {/if}
        <BackendCard {canManage} />
        <SupervisorCard {canEditSettings} />
      {:else if section === 'security'}
        <AccessCard {canEditSettings} />
        <ApiKeysCard />
      {:else if section === 'integrations'}
        <CurseForgeCard {canEditSettings} />
      {:else}
        <AppInfoCard />
      {/if}
    </div>
  </div>
</div>
