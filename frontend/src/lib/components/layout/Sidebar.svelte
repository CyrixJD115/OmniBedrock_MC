<script lang="ts">
  import { page } from '$app/stores';
  import { serverStatus } from '$stores/index';
  import { currentUser, userPermissions } from '$stores/auth';
  import { instances, activeInstanceId, activeInstance, switchInstance } from '$stores/instances';
  import { themeMode, cycleMode } from '$lib/theme';
  import {
    LayoutDashboard, Terminal, FileCode, HardDrive, Package, Globe, Puzzle,
    Users, UserCog, FileText, BarChart3, Settings, ChevronLeft, Server, Power, LogOut,
    UserCircle, Shield, ClipboardList, Radio, Store, Sun, Moon, Monitor, Layers, ChevronsUpDown, Check
  } from '@lucide/svelte';
  import { goto } from '$app/navigation';

  let { sidebarOpen, onlogout }: { sidebarOpen: boolean; onlogout?: () => void } = $props();

  // Grouped nav: three labeled sections instead of a flat 15-item list.
  // Groups whose items are all permission-hidden collapse away entirely.
  interface NavItem {
    href: string;
    icon: typeof LayoutDashboard;
    label: string;
    perm: string;
  }
  const navGroups: { label: string; items: NavItem[] }[] = [
    {
      label: 'Instance',
      items: [
        { href: '/', icon: LayoutDashboard, label: 'Dashboard', perm: '' },
        { href: '/instances', icon: Layers, label: 'Instances', perm: '' },
        { href: '/console', icon: Terminal, label: 'Console', perm: 'CONSOLE_VIEW' },
        { href: '/properties', icon: FileCode, label: 'Properties', perm: 'PROPERTIES_VIEW' },
        { href: '/files', icon: FileText, label: 'Files', perm: 'FILES_VIEW' },
        { href: '/backups', icon: HardDrive, label: 'Backups', perm: 'BACKUPS_VIEW' },
        { href: '/players', icon: Users, label: 'Players', perm: 'PLAYERS_VIEW' },
        { href: '/performance', icon: BarChart3, label: 'Performance', perm: '' },
      ],
    },
    {
      label: 'Content',
      items: [
        { href: '/addons', icon: Puzzle, label: 'Mods', perm: 'ADDONS_VIEW' },
        { href: '/store', icon: Store, label: 'Store', perm: 'STORE_VIEW' },
        { href: '/worlds', icon: Globe, label: 'Worlds', perm: '' },
        { href: '/broadcaster', icon: Radio, label: 'Broadcaster', perm: 'BROADCASTER_VIEW' },
      ],
    },
    {
      label: 'Panel',
      items: [
        { href: '/users', icon: UserCog, label: 'Users & Roles', perm: 'USERS_VIEW' },
        { href: '/audit', icon: ClipboardList, label: 'Audit', perm: 'AUDIT_VIEW' },
        { href: '/settings', icon: Settings, label: 'Settings', perm: 'SETTINGS_VIEW' },
      ],
    },
  ];

  let currentPath = $state('/');
  $effect(() => { currentPath = $page.url.pathname; });

  function canAccess(perm: string): boolean {
    if (!perm) return true;
    return $userPermissions.includes(perm);
  }

  function navClass(href: string): string {
    const active = currentPath === href;
    const base = 'flex items-center gap-3 px-3 py-2.5 text-sm font-medium uppercase tracking-wider transition-all duration-150 border-l-2';
    if (active) return `${base} bg-bedrock-500/15 border-bedrock-400 text-bedrock-300`;
    return `${base} border-transparent text-deep-300 hover:bg-deep-700/60 hover:border-deep-500 hover:text-deep-100`;
  }

  // Icon reflecting the *effective* mode; the label shows the choice.
  const modeIcon = $derived($themeMode === 'light' ? Sun : $themeMode === 'dark' ? Moon : Monitor);
  const modeTitle = $derived(
    $themeMode === 'light' ? 'Light mode — click to switch'
    : $themeMode === 'dark' ? 'Dark mode — click to switch'
    : 'Following system — click to switch'
  );
</script>

<aside class={"h-full bg-deep-900 border-r-2 border-deep-700/60 flex flex-col transition-all duration-200 " + (sidebarOpen ? 'w-56' : 'w-16')}>
  <div class="flex items-center justify-between p-4 border-b-2 border-deep-700/60">
    {#if sidebarOpen}
      <div class="flex items-center gap-2.5 min-w-0">
        <div class="w-4 h-4 shrink-0 bg-bedrock-500 shadow-glow-accent"></div>
        <span class="mc-heading font-bold text-strong text-sm uppercase tracking-widest truncate">OmniBedrock</span>
      </div>
    {/if}
    <button onclick={() => sidebarOpen = !sidebarOpen}
            class="p-1.5 hover:bg-deep-700/60 text-deep-400 hover:text-strong transition-colors rounded-sm shrink-0"
            aria-label={sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'}>
      <ChevronLeft size={16} class={!sidebarOpen ? 'rotate-180' : ''} />
    </button>
  </div>

  <!-- User info -->
  {#if sidebarOpen && $currentUser}
    <div class="px-4 py-3 border-b border-deep-700/40 flex items-center gap-2.5">
      <UserCircle size={18} class="text-bedrock-400 shrink-0" />
      <div class="min-w-0">
        <p class="text-xs font-medium text-strong truncate">{$currentUser.display_name}</p>
        <p class="text-[10px] uppercase tracking-wider text-deep-400">{$currentUser.role}</p>
      </div>
    </div>
  {/if}

  <!-- Instance switcher -->
  {#if sidebarOpen && $instances.length > 0}
    <div class="px-3 py-3 border-b border-deep-700/40">
      <p class="text-[10px] uppercase tracking-wider text-deep-400 mb-1.5 px-1">Instance</p>
      <div class="relative" >
        <select
          class="input w-full text-xs appearance-none pr-7 cursor-pointer"
          value={$activeInstanceId}
          onchange={(e) => switchInstance((e.currentTarget as HTMLSelectElement).value)}
          aria-label="Active instance">
          {#each $instances as i}
            <option value={i.id}>{i.name}</option>
          {/each}
        </select>
        <ChevronsUpDown size={12} class="absolute right-2.5 top-1/2 -translate-y-1/2 text-deep-400 pointer-events-none" />
      </div>
      {#if $activeInstance}
        <div class="flex items-center gap-2 mt-2 px-1">
          <span class="status-dot {$activeInstance.status}"></span>
          <span class="text-[10px] uppercase tracking-wider text-deep-300 truncate">
            {$activeInstance.status} · {$activeInstance.backend}
          </span>
        </div>
      {/if}
      <button onclick={() => goto('/instances')}
              class="mt-2 w-full text-left text-[10px] uppercase tracking-wider text-bedrock-400 hover:text-bedrock-300 transition-colors px-1 flex items-center gap-1">
        <Layers size={10} /> Manage instances
      </button>
    </div>
  {/if}

  <nav class="flex-1 overflow-y-auto p-2 space-y-0.5">
    {#each navGroups as group}
      {@const visible = group.items.filter((item) => canAccess(item.perm))}
      {#if visible.length > 0}
        {#if sidebarOpen}
          <p class="px-3 pt-2 pb-1 text-[9px] uppercase tracking-[0.2em] text-deep-500 font-bold select-none">{group.label}</p>
        {:else}
          <div class="my-1 border-t border-deep-700/40"></div>
        {/if}
        {#each visible as item}
          <a href={item.href} class={navClass(item.href)} title={sidebarOpen ? undefined : item.label}>
            <item.icon size={16} class="shrink-0" />
            {#if sidebarOpen}
              <span>{item.label}</span>
            {/if}
          </a>
        {/each}
      {/if}
    {/each}
  </nav>

  <div class="p-3 border-t-2 border-deep-700/60 space-y-2">
    {#if sidebarOpen}
      <div class="flex items-center gap-2">
        <span class="status-dot {$serverStatus.status}"></span>
        <span class="text-xs uppercase tracking-wider {$serverStatus.status === 'running' ? 'text-teal-400' : 'text-red-400'}">
          {$serverStatus.status}
        </span>
      </div>
    {:else}
      <div class="flex justify-center" title="Server {$serverStatus.status}">
        <span class="status-dot {$serverStatus.status}"></span>
      </div>
    {/if}
    <div class="flex items-center {sidebarOpen ? 'gap-2' : 'justify-center'}">
      <button onclick={cycleMode} title={modeTitle} aria-label={modeTitle}
              class="flex items-center {sidebarOpen ? 'gap-3 px-3 py-2 flex-1' : 'p-2'} text-xs uppercase tracking-wider text-deep-400 hover:text-bedrock-300 hover:bg-deep-700/60 transition-colors border-l-2 border-transparent hover:border-bedrock-500/50 rounded-none">
        {#key $themeMode}
          <modeIcon size={14} class="shrink-0" />
        {/key}
        {#if sidebarOpen}
          <span class="capitalize">{$themeMode}</span>
        {/if}
      </button>
    </div>
    <button onclick={onlogout}
            class="flex items-center gap-3 px-3 py-2 w-full text-xs uppercase tracking-wider text-deep-400 hover:text-red-400 hover:bg-deep-700/60 transition-colors border-l-2 border-transparent hover:border-red-500/50 {sidebarOpen ? '' : 'justify-center'}">
      <LogOut size={14} class="shrink-0" />
      {#if sidebarOpen}
        <span>Disconnect</span>
      {/if}
    </button>
  </div>
</aside>
