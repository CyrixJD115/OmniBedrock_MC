<script lang="ts">
  /**
   * One instance in the instances grid: identity/status header, memory and
   * CPU usage bars, disk monitoring, shared-with line, and the lifecycle
   * action row. Capability flags are computed by the page; every action is
   * delegated back through callback props.
   */
  import {
    RotateCw, Square, Play, Terminal, Settings2, Trash2,
    Cpu, MemoryStick, ShieldAlert, ShieldCheck, Users2, HardDrive
  } from '@lucide/svelte';
  import type { Instance, RoleInfo } from '$types/index';
  import { deploymentBadge, enforcementNote, fmtMb, fmtUptime, memBar } from '$lib/instances/format';
  import { roleInfo } from '$lib/instances/access';

  let {
    instance, active, acting, canAct, canEdit, canDelete, roleCatalog,
    onact, onopen, onedit, ondelete,
  }: {
    instance: Instance;
    active: boolean;
    acting: string | null;
    canAct: boolean;
    canEdit: boolean;
    canDelete: boolean;
    roleCatalog: RoleInfo[];
    onact: (action: string) => void;
    onopen: () => void;
    onedit: () => void;
    ondelete: () => void;
  } = $props();
</script>

<div class="card flex flex-col gap-3">
  <div class="flex items-start justify-between gap-3">
    <div class="min-w-0">
      <div class="flex items-center gap-2">
        <span class="status-dot {instance.status}"></span>
        <h3 class="text-sm font-bold text-strong truncate">{instance.name}</h3>
        {#if active}
          <span class="px-1.5 py-0.5 border border-bedrock-500/40 bg-bedrock-500/10 text-bedrock-300 text-[10px] uppercase tracking-wider">active</span>
        {/if}
      </div>
      <p class="text-[10px] uppercase tracking-wider text-deep-400 mt-1">
        {instance.owner} · {instance.backend} · port {instance.port} · up {fmtUptime(instance.uptime)}
      </p>
    </div>
    <div class="flex flex-col items-end gap-1 shrink-0">
      <span class="px-1.5 py-0.5 border text-[10px] uppercase tracking-wider {deploymentBadge(instance.deployment)}" title={enforcementNote(instance)}>
        {instance.deployment}
      </span>
      {#if instance.enforcement === 'advisory'}
        <span class="flex items-center gap-1 text-[10px] text-amber-400" title={enforcementNote(instance)}>
          <ShieldAlert size={10} /> advisory limits
        </span>
      {:else}
        <span class="flex items-center gap-1 text-[10px] text-teal-400" title={enforcementNote(instance)}>
          <ShieldCheck size={10} /> enforced
        </span>
      {/if}
    </div>
  </div>

  <div class="grid grid-cols-2 gap-3 text-xs">
    <div>
      <div class="flex justify-between text-deep-400 mb-1">
        <span class="uppercase tracking-wider text-[10px] flex items-center gap-1"><MemoryStick size={10} /> Memory</span>
        <span class="font-mono">
          {#if instance.usage}{fmtMb(instance.usage.memory_mb)}{/if}
          {#if instance.resources.memory_mb}/ {fmtMb(instance.resources.memory_mb)}{:else}(no limit){/if}
        </span>
      </div>
      <div class="h-1.5 bg-deep-700">
        <div class="h-full bg-bedrock-500" style="width: {memBar(instance)}%"></div>
      </div>
    </div>
    <div>
      <div class="flex justify-between text-deep-400 mb-1">
        <span class="uppercase tracking-wider text-[10px] flex items-center gap-1"><Cpu size={10} /> CPU</span>
        <span class="font-mono">
          {#if instance.usage}{instance.usage.cpu_percent.toFixed(1)}%{/if}
          {#if instance.resources.cpus}/ {instance.resources.cpus} CPU{:else}(no limit){/if}
        </span>
      </div>
      <div class="h-1.5 bg-deep-700">
        <div class="h-full bg-teal-500" style="width: {instance.resources.cpus && instance.usage ? Math.min(100, (instance.usage.cpu_percent / (instance.resources.cpus * 100)) * 100) : Math.min(100, instance.usage?.cpu_percent ?? 0)}%"></div>
      </div>
    </div>
  </div>

  <div class="flex justify-between text-deep-400 text-[10px] uppercase tracking-wider">
    <span class="flex items-center gap-1"><HardDrive size={10} /> Disk (monitored, unenforced)</span>
    <span class="font-mono text-deep-300">{instance.usage ? fmtMb(instance.usage.disk_mb ?? 0) : '…'}</span>
  </div>
  {#if instance.access.length > 0}
    <p class="text-[10px] text-deep-400 flex items-center gap-1">
      <Users2 size={10} />
      Shared with
      {#each instance.access as a, ai}{#if ai > 0},{/if}
        {a.user} ({a.roles.map((r) => roleInfo(roleCatalog, r)?.display_name ?? r).join('/')}){/each}
    </p>
  {/if}

  <div class="flex flex-wrap gap-2 pt-1 border-t border-deep-700/40">
    {#if canAct}
      {#if instance.status === 'running'}
        <button onclick={() => onact('restart')} disabled={acting !== null}
                class="btn-secondary text-xs flex items-center gap-1.5">
          <RotateCw size={12} /> {acting === `${instance.id}:restart` ? 'Restarting…' : 'Restart'}
        </button>
        <button onclick={() => onact('stop')} disabled={acting !== null}
                class="btn-danger text-xs flex items-center gap-1.5">
          <Square size={12} /> Stop
        </button>
      {:else if instance.status !== 'starting' && instance.status !== 'stopping'}
        <button onclick={() => onact('start')} disabled={acting !== null}
                class="btn-success text-xs flex items-center gap-1.5">
          <Play size={12} /> {acting === `${instance.id}:start` ? 'Starting…' : 'Start'}
        </button>
      {/if}
    {/if}
    <button onclick={onopen} class="btn-secondary text-xs flex items-center gap-1.5">
      <Terminal size={12} /> Console
    </button>
    {#if canEdit}
      <button onclick={onedit} class="btn-secondary text-xs flex items-center gap-1.5">
        <Settings2 size={12} /> Edit
      </button>
    {/if}
    {#if canDelete}
      <button onclick={ondelete} class="btn-danger text-xs flex items-center gap-1.5 ml-auto">
        <Trash2 size={12} /> Delete
      </button>
    {/if}
  </div>
</div>
