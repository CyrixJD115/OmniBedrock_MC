<script lang="ts">
  /**
   * Create-instance modal: name/runtime/port, deployment and resource-limit
   * fields bound to the page's create model, docker/systemd hints driven by
   * host capabilities, and the install-after flag. Submission and dismissal
   * are delegated to the page.
   */
  import { ShieldAlert } from '@lucide/svelte';
  import type { HostInfo } from '$types/index';
  import type { CreateInstanceForm } from '$lib/instances/forms';

  let { form = $bindable(), hostInfo, creating, oncreate, onclose }: {
    form: CreateInstanceForm;
    hostInfo: HostInfo | null;
    creating: boolean;
    oncreate: () => void;
    onclose: () => void;
  } = $props();
</script>

<div class="fixed inset-0 flex items-center justify-center z-50" style="background: rgb(var(--color-overlay) / 0.85); backdrop-filter: blur(4px);" role="dialog" aria-modal="true" tabindex="-1"
     onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
     onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}>
  <div class="card max-w-md w-full mx-4">
    <h2 class="text-sm font-bold text-strong uppercase tracking-wider mb-4">New Instance</h2>
    <div class="space-y-3">
      <div>
        <label class="text-[10px] uppercase tracking-wider text-deep-400">Name</label>
        <input bind:value={form.name} placeholder="Dad's Survival Server" class="input w-full text-xs mt-1" />
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Runtime</label>
          <select bind:value={form.backend} class="input w-full text-xs mt-1">
            <option value="vanilla">Vanilla Bedrock</option>
            <option value="endstone">Endstone</option>
          </select>
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">UDP port (blank = auto)</label>
          <input bind:value={form.port} type="number" placeholder="19132" class="input w-full text-xs mt-1" />
        </div>
      </div>
      <div>
        <label class="text-[10px] uppercase tracking-wider text-deep-400">Deployment / limit enforcement</label>
        <select bind:value={form.deployment} class="input w-full text-xs mt-1">
          <option value="native">Native process — advisory limits only</option>
          {#if !hostInfo?.capabilities.systemd}
            <option value="systemd" disabled>systemd service — unavailable on this host</option>
          {:else}
            <option value="systemd">systemd service — cgroup enforced (Linux)</option>
          {/if}
          {#if !hostInfo?.capabilities.docker}
            <option value="docker" disabled>Docker container — unavailable on this host</option>
          {:else}
            <option value="docker">Docker container — runtime enforced</option>
          {/if}
        </select>
        <p class="text-[10px] text-deep-400 mt-1">
          {#if form.deployment === 'native'}Limits below are monitored but NOT enforced for native processes.{/if}
          {#if form.deployment === 'systemd'}Runs as a systemd user service (omnibedrock-&lt;id&gt;.service): MemoryMax/CPUQuota enforced, survives panel restarts.{/if}
          {#if form.deployment === 'docker'}--memory/--cpus are enforced by the container runtime; the container runs detached under Docker's unless-stopped restart policy, so it survives panel restarts and reboots (turn the instance's auto-restart off to keep it down).{/if}
        </p>
        {#if form.deployment === 'docker' && hostInfo?.capabilities?.platform === 'macos'}
          <p class="text-[10px] text-amber-400 mt-1 flex items-start gap-1">
            <ShieldAlert size={10} class="shrink-0 mt-0.5" />
            Docker is the default here because there is no official macOS bedrock_server binary. Apple Silicon Macs run linux/amd64 images under emulation, which is slower.
          </p>
        {/if}
        {#if form.deployment === 'docker' && hostInfo?.capabilities?.platform === 'windows'}
          <p class="text-[10px] text-amber-400 mt-1 flex items-start gap-1">
            <ShieldAlert size={10} class="shrink-0 mt-0.5" />
            Docker Desktop adds licensing and WSL2 overhead, and every UDP port must be mapped manually — native is recommended on Windows.
          </p>
        {/if}
        {#if form.deployment === 'docker'}
          <div class="mt-2 space-y-2 border border-deep-600/40 bg-deep-900/30 p-2">
            <div>
              <label class="text-[10px] uppercase tracking-wider text-deep-400">Image (blank = panel default for the runtime)</label>
              <input bind:value={form.dockerImage} placeholder="omnibedrock-vanilla:latest" class="input w-full text-xs mt-1" />
            </div>
            <div>
              <label class="text-[10px] uppercase tracking-wider text-deep-400">Extra bind mounts (one host:container[:opts] per line)</label>
              <textarea bind:value={form.dockerMounts} rows="2" placeholder="/mnt/media/worlds:/data/worlds" class="input w-full text-xs mt-1 font-mono"></textarea>
            </div>
            <p class="text-[10px] text-deep-400">
              {#if hostInfo?.capabilities?.platform === 'linux'}
                Runs with host networking: game ports and ICE ranges behave exactly like a native process.
              {:else}
                The instance's UDP port pair is mapped through to the container's stock 19132/19133.
              {/if}
            </p>
          </div>
        {/if}
        {#if form.deployment === 'systemd'}
          <label class="flex items-center gap-2 text-xs text-deep-300 mt-2">
            <input type="checkbox" bind:checked={form.autostart} />
            Start at boot (enables the user unit)
          </label>
          {#if hostInfo?.capabilities.systemd && !hostInfo?.capabilities.linger}
            <p class="text-[10px] text-amber-400 mt-1">
              Linger is off — run <code>loginctl enable-linger $USER</code> so units start at boot without a login session.
            </p>
          {/if}
        {/if}
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">Memory limit (MB, blank = ∞)</label>
          <input bind:value={form.memory} type="number" placeholder="1024" class="input w-full text-xs mt-1" />
        </div>
        <div>
          <label class="text-[10px] uppercase tracking-wider text-deep-400">CPU limit (cores, blank = ∞)</label>
          <input bind:value={form.cpus} type="number" step="0.1" placeholder="1.0" class="input w-full text-xs mt-1" />
        </div>
      </div>
      <label class="flex items-center gap-2 text-xs text-deep-300">
        <input type="checkbox" bind:checked={form.installAfter} />
        Install the runtime now and open its console
      </label>
      <div class="flex justify-end gap-2 pt-2">
        <button onclick={onclose} class="btn-secondary text-xs">Cancel</button>
        <button onclick={oncreate} disabled={creating} class="btn-primary text-xs">{creating ? 'Creating…' : 'Create'}</button>
      </div>
    </div>
  </div>
</div>
