<script lang="ts">
  /**
   * Edit-instance modal: name/port/limits, deployment selector, and the
   * share-with-users editor (role chips + per-entry roles) bound to the
   * page's edit model. Saving, adding access, toggling entry roles and
   * dismissal are delegated to the page.
   */
  import { Plus, ShieldAlert } from '@lucide/svelte';
  import type { AccessEntry, HostInfo, Instance, RoleInfo } from '$types/index';
  import type { EditInstanceForm } from '$lib/instances/forms';

  let {
    instance, editForm = $bindable(), hostInfo, users, roleCatalog,
    accessUser = $bindable(''), accessRoles = $bindable(new Set<string>()),
    onaddaccess, ontoggleentryrole, onsave, onclose,
  }: {
    instance: Instance;
    editForm: EditInstanceForm;
    hostInfo: HostInfo | null;
    users: string[];
    roleCatalog: RoleInfo[];
    accessUser: string;
    accessRoles: Set<string>;
    onaddaccess: () => void;
    ontoggleentryrole: (entry: AccessEntry, role: string) => void;
    onsave: () => void;
    onclose: () => void;
  } = $props();
</script>

<div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
     onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
     onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}>
  <div class="card max-w-md w-full mx-4 max-h-[85vh] overflow-y-auto">
    <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-1">Edit {instance.name}</h2>
    {#if instance.status === 'running'}
      <p class="text-[10px] text-amber-400 mb-3 flex items-center gap-1"><ShieldAlert size={10} /> Stop the instance before changing deployment, resources, or port.</p>
    {:else}
      <p class="mb-3"></p>
    {/if}
    <div class="space-y-3">
      <div>
        <label class="text-[10px] uppercase tracking-wider text-deep-400">Name</label>
        <input bind:value={editForm.name} class="input w-full text-xs mt-1" />
      </div>
      <div class="grid grid-cols-3 gap-3">
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Port</label>
          <input bind:value={editForm.port} type="number" class="input w-full text-xs mt-1" />
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Memory MB</label>
          <input bind:value={editForm.memory} type="number" placeholder="∞" class="input w-full text-xs mt-1" />
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">CPUs</label>
          <input bind:value={editForm.cpus} type="number" step="0.1" placeholder="∞" class="input w-full text-xs mt-1" />
        </div>
      </div>
      <div>
        <label class="text-[10px] uppercase tracking-wider text-deep-400">Deployment</label>
        <select bind:value={editForm.deployment} class="input w-full text-xs mt-1">
          <option value="native">Native — advisory</option>
          {#if hostInfo?.capabilities.systemd}<option value="systemd">systemd service — cgroup enforced</option>{/if}
          {#if hostInfo?.capabilities.docker}<option value="docker">Docker — container enforced</option>{/if}
        </select>
        {#if editForm.deployment === 'docker'}
          <div class="mt-2 space-y-2 border border-deep-600/40 bg-deep-900/30 p-2">
            <div>
              <label class="text-[10px] uppercase tracking-wider text-deep-400">Image (blank = panel default for the runtime)</label>
              <input bind:value={editForm.dockerImage} placeholder="omnibedrock-vanilla:latest" class="input w-full text-xs mt-1" />
            </div>
            <div>
              <label class="text-[10px] uppercase tracking-wider text-deep-400">Extra bind mounts (one host:container[:opts] per line)</label>
              <textarea bind:value={editForm.dockerMounts} rows="2" placeholder="/mnt/media/worlds:/data/worlds" class="input w-full text-xs mt-1 font-mono"></textarea>
            </div>
          </div>
        {/if}
        {#if editForm.deployment === 'systemd'}
          <label class="flex items-center gap-2 text-xs text-deep-300 mt-2">
            <input type="checkbox" bind:checked={editForm.autostart} />
            Start at boot (enables the user unit)
          </label>
        {/if}
      </div>
      <div>
        <label class="text-[10px] uppercase tracking-wider text-deep-400">Share with users — roles grant what they may do here</label>
        <div class="flex gap-2 mt-1">
          <input list="instance-users" bind:value={accessUser} placeholder="username" class="input flex-1 text-xs" />
          <datalist id="instance-users">
            {#each users as u}<option value={u} />{/each}
          </datalist>
          <button onclick={onaddaccess} disabled={accessRoles.size === 0} class="btn-secondary text-xs"><Plus size={12} /></button>
        </div>
        <div class="flex flex-wrap gap-2 mt-1.5">
          {#each roleCatalog as r (r.name)}
            <label class="flex items-center gap-1 text-[10px] uppercase tracking-wider cursor-pointer px-1.5 py-0.5 border {accessRoles.has(r.name) ? 'border-bedrock-500/60 bg-bedrock-500/10' : 'border-deep-600/40 bg-deep-900/40'}">
              <input type="checkbox" class="accent-bedrock-500 w-3 h-3"
                     checked={accessRoles.has(r.name)}
                     onchange={() => { const next = new Set(accessRoles); next.has(r.name) ? next.delete(r.name) : next.add(r.name); accessRoles = next; }} />
              <span class="w-2 h-2 inline-block" style="background: #{r.color || '99AAB5'}"></span>
              {r.display_name}
            </label>
          {/each}
        </div>
        {#if editForm.access.length > 0}
          <div class="mt-2 space-y-1.5">
            {#each editForm.access as a (a.user)}
              <div class="border border-deep-600/50 bg-deep-700/30 p-2">
                <div class="flex items-center justify-between">
                  <span class="text-xs font-bold text-strong">{a.user}</span>
                  <button onclick={() => { editForm.access = editForm.access.filter((x) => x.user !== a.user); }}
                          class="text-red-400 hover:text-red-300 text-xs" aria-label="Remove {a.user}">remove</button>
                </div>
                <div class="flex flex-wrap gap-2 mt-1.5">
                  {#each roleCatalog as r (r.name)}
                    <label class="flex items-center gap-1 text-[10px] uppercase tracking-wider cursor-pointer">
                      <input type="checkbox" class="accent-bedrock-500 w-3 h-3"
                             checked={a.roles.includes(r.name)}
                             onchange={() => ontoggleentryrole(a, r.name)} />
                      <span class="w-2 h-2 inline-block" style="background: #{r.color || '99AAB5'}"></span>
                      {r.display_name}
                    </label>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
      <div class="flex justify-end gap-2 pt-2">
        <button onclick={onclose} class="btn-secondary text-xs">Cancel</button>
        <button onclick={onsave} class="btn-primary text-xs">Save</button>
      </div>
    </div>
  </div>
</div>
