<script lang="ts">
  import { page } from '$app/stores';
  import { isServerRunning, serverStatus } from '$stores/index';
  import { Server, Terminal, HardDrive, Package, Globe, Users, FileText, Settings, BarChart3, ChevronLeft, Power } from '@lucide/svelte';

  let { sidebarOpen }: { sidebarOpen: boolean } = $props();

  const navItems = [
    { href: '/', icon: Server, label: 'Dashboard' },
    { href: '/console', icon: Terminal, label: 'Console' },
    { href: '/properties', icon: FileText, label: 'Properties' },
    { href: '/backups', icon: HardDrive, label: 'Backups' },
    { href: '/addons', icon: Package, label: 'Addons' },
    { href: '/worlds', icon: Globe, label: 'Worlds' },
    { href: '/players', icon: Users, label: 'Players' },
    { href: '/files', icon: FileText, label: 'Files' },
    { href: '/performance', icon: BarChart3, label: 'Performance' },
    { href: '/settings', icon: Settings, label: 'Settings' },
  ];

  let currentPath = $state('/');
  $effect(() => {
    currentPath = $page.url.pathname;
  });

  let asideClass = $derived(`h-full bg-surface-900 border-r border-surface-800 flex flex-col transition-all duration-200 ${sidebarOpen ? 'w-60' : 'w-16'}`);

  function navClass(href: string): string {
    const isActive = currentPath === href;
    const base = 'flex items-center gap-3 px-3 py-2.5 rounded-lg transition-all duration-150';
    if (isActive) return `${base} bg-surface-800 text-surface-100`;
    return `${base} hover:bg-surface-800 hover:text-surface-100`;
  }
</script>

<aside class={asideClass}>
  <div class="flex items-center justify-between p-4 border-b border-surface-800">
    {#if sidebarOpen}
      <div class="flex items-center gap-2">
        <div class="w-3 h-3 rounded-full bg-neon-green shadow-[0_0_8px_#39ff14]"></div>
        <span class="font-bold text-white tracking-wide">OmniBedrock</span>
      </div>
    {/if}
    <button onclick={() => sidebarOpen = !sidebarOpen}
            class="p-1.5 rounded-lg hover:bg-surface-800 text-surface-400 hover:text-white transition-colors">
      <ChevronLeft size={18} class={!sidebarOpen ? 'rotate-180' : ''} />
    </button>
  </div>

  <nav class="flex-1 overflow-y-auto p-2 space-y-1">
    {#each navItems as item}
      <a href={item.href} class={navClass(item.href)}>
        {#if item.icon}
          <item.icon size={18} class="min-w-fit" />
        {/if}
        {#if sidebarOpen}
          <span class="text-sm font-medium">{item.label}</span>
        {/if}
      </a>
    {/each}
  </nav>

  <div class="p-3 border-t border-surface-800">
    {#if sidebarOpen}
      <div class="flex items-center gap-2">
        <Power size={14} class={$isServerRunning ? 'text-neon-green' : 'text-red-400'} />
        <span class="text-xs {$isServerRunning ? 'text-neon-green' : 'text-red-400'}">
          {$isServerRunning ? 'Server Running' : 'Server Stopped'}
        </span>
      </div>
    {/if}
  </div>
</aside>
