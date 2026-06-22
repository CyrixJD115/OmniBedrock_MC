<script lang="ts">
  import { page } from '$app/stores';
  import { serverStatus } from '$stores/index';
  import { currentUser, userPermissions } from '$stores/auth';
  import {
    LayoutDashboard, Terminal, FileCode, HardDrive, Package, Globe,
    Users, UserCog, FileText, BarChart3, Settings, ChevronLeft, Server, Power, LogOut,
    UserCircle, Shield, ClipboardList
  } from '@lucide/svelte';

  let { sidebarOpen, onlogout }: { sidebarOpen: boolean; onlogout?: () => void } = $props();

  const navItems = [
    { href: '/', icon: LayoutDashboard, label: 'Dashboard', perm: '' },
    { href: '/console', icon: Terminal, label: 'Console', perm: 'CONSOLE_VIEW' },
    { href: '/properties', icon: FileCode, label: 'Properties', perm: 'PROPERTIES_VIEW' },
    { href: '/backups', icon: HardDrive, label: 'Backups', perm: 'BACKUPS_VIEW' },
    { href: '/addons', icon: Package, label: 'Addons', perm: 'ADDONS_VIEW' },
    { href: '/worlds', icon: Globe, label: 'Worlds', perm: '' },
    { href: '/players', icon: Users, label: 'Players', perm: 'PLAYERS_VIEW' },
    { href: '/users', icon: UserCog, label: 'Users', perm: 'USERS_VIEW' },
    { href: '/roles', icon: Shield, label: 'Roles', perm: 'USERS_VIEW' },
    { href: '/files', icon: FileText, label: 'Files', perm: 'FILES_VIEW' },
    { href: '/performance', icon: BarChart3, label: 'Performance', perm: '' },
    { href: '/audit', icon: ClipboardList, label: 'Audit', perm: 'AUDIT_VIEW' },
    { href: '/settings', icon: Settings, label: 'Settings', perm: 'SETTINGS_VIEW' },
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
    if (active) return `${base} bg-bedrock-500/10 border-bedrock-400 text-bedrock-300`;
    return `${base} border-transparent text-deep-300 hover:bg-deep-800/50 hover:border-deep-500 hover:text-deep-100`;
  }
</script>

<aside class={"h-full bg-deep-900 border-r border-deep-700/50 flex flex-col transition-all duration-200 " + (sidebarOpen ? 'w-56' : 'w-16')}>
  <div class="flex items-center justify-between p-4 border-b border-deep-700/50">
    {#if sidebarOpen}
      <div class="flex items-center gap-2.5">
        <div class="w-4 h-4 bg-bedrock-500" style="box-shadow: 0 0 12px rgba(6,182,212,0.5);"></div>
        <span class="font-bold text-white text-sm uppercase tracking-widest" style="font-family: 'Montserrat', sans-serif;">OmniBedrock</span>
      </div>
    {/if}
    <button onclick={() => sidebarOpen = !sidebarOpen}
            class="p-1.5 hover:bg-deep-800 text-deep-400 hover:text-white transition-colors rounded">
      <ChevronLeft size={16} class={!sidebarOpen ? 'rotate-180' : ''} />
    </button>
  </div>

  <!-- User info -->
  {#if sidebarOpen && $currentUser}
    <div class="px-4 py-3 border-b border-deep-700/30 flex items-center gap-2.5">
      <UserCircle size={18} class="text-bedrock-400 shrink-0" />
      <div class="min-w-0">
        <p class="text-xs font-medium text-white truncate">{$currentUser.display_name}</p>
        <p class="text-[10px] uppercase tracking-wider text-deep-400">{$currentUser.role}</p>
      </div>
    </div>
  {/if}

  <nav class="flex-1 overflow-y-auto p-2 space-y-0.5">
    {#each navItems as item}
      {#if canAccess(item.perm)}
        <a href={item.href} class={navClass(item.href)}>
          <item.icon size={16} class="shrink-0" />
          {#if sidebarOpen}
            <span>{item.label}</span>
          {/if}
        </a>
      {/if}
    {/each}
  </nav>

  <div class="p-3 border-t border-deep-700/50 space-y-2">
    {#if sidebarOpen}
      <div class="flex items-center gap-2">
        <span class="status-dot {$serverStatus.status}"></span>
        <span class="text-xs uppercase tracking-wider {$serverStatus.status === 'running' ? 'text-teal-400' : 'text-red-400'}">
          {$serverStatus.status}
        </span>
      </div>
    {/if}
    <button onclick={onlogout}
            class="flex items-center gap-3 px-3 py-2 w-full text-xs uppercase tracking-wider text-deep-400 hover:text-red-400 hover:bg-deep-800/50 transition-colors border-l-2 border-transparent hover:border-red-500/50">
      <LogOut size={14} class="shrink-0" />
      {#if sidebarOpen}
        <span>Disconnect</span>
      {/if}
    </button>
  </div>
</aside>
