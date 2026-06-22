<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { addToast } from '$stores/toast';
  import { userPermissions } from '$stores/auth';
  import { Eye, RotateCw, Save } from '@lucide/svelte';

  let properties = $state<{ key: string; value: string; comment: string; inline_comment: string }[]>([]);
  let rawText = $state('');
  let showRaw = $state(false);
  let loading = $state(true);

  onMount(async () => {
    try {
      properties = await api.getProperties();
    } catch (e: any) { addToast(`Failed to load properties: ${e.message}`, 'error'); }
    loading = false;
  });

  async function updateValue(key: string, value: string) {
    try {
      await api.updateProperty(key, value);
      addToast(`Updated ${key}`, 'success');
    } catch (e: any) { addToast(`Update failed: ${e.message}`, 'error'); }
  }

  async function loadRaw() {
    try {
      rawText = await api.getPropertiesRaw();
      showRaw = true;
    } catch (e: any) { addToast(`Failed to load: ${e.message}`, 'error'); }
  }

  async function saveRaw() {
    try {
      await api.savePropertiesRaw(rawText);
      properties = await api.getProperties();
      showRaw = false;
      addToast('Properties saved', 'success');
    } catch (e: any) { addToast(`Save failed: ${e.message}`, 'error'); }
  }

  const knownProps = ['server-name', 'gamemode', 'difficulty', 'allow-cheats', 'max-players',
    'online-mode', 'white-list', 'server-port', 'server-portv6', 'view-distance', 'tick-distance',
    'level-name', 'level-seed', 'default-player-permission-level', 'texturepack-required',
    'player-idle-timeout', 'content-log-file-enabled', 'compression-threshold',
    'server-authoritative-movement', 'enable-lan-visibility', 'chat-restriction',
    'disable-player-interaction'];

  let known = $derived(properties.filter(p => p.key && knownProps.includes(p.key)));
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-bold text-white uppercase tracking-widest">Server Properties</h1>
      <div class="pixel-divider mt-2 w-48"></div>
    </div>
    <div class="flex gap-2">
      <button onclick={loadRaw} class="btn-secondary flex items-center gap-2 text-xs">
        <Eye size={14} /> Raw
      </button>
      <button onclick={async () => { try { properties = await api.getProperties(); addToast('Reloaded', 'success'); } catch {} }}
              class="btn-ghost p-2"><RotateCw size={14} /></button>
    </div>
  </div>

  {#if showRaw}
    <div class="card">
      <h2 class="card-header">Raw Editor</h2>
      <textarea bind:value={rawText} class="input w-full h-96 font-mono text-sm" spellcheck="false"></textarea>
      <div class="flex gap-2 mt-3">
        {#if $userPermissions.includes('PROPERTIES_EDIT')}
          <button onclick={saveRaw} class="btn-primary flex items-center gap-2 text-xs"><Save size={14} /> Save</button>
        {/if}
        <button onclick={() => showRaw = false} class="btn-secondary text-xs">Cancel</button>
      </div>
    </div>
  {:else}
    <div class="card">
      <h2 class="card-header">Quick Edit</h2>
      <div class="space-y-2">
        {#each known as entry}
          <div class="flex items-center gap-3 p-2.5 border border-deep-600/20 bg-deep-900/50">
            <span class="text-xs font-mono text-bedrock-400 w-44 shrink-0 uppercase tracking-wider">{entry.key}</span>
            {#if ['gamemode', 'difficulty', 'default-player-permission-level'].includes(entry.key)}
              <select value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLSelectElement).value)} class="input flex-1 text-xs py-1.5">
                {#if entry.key === 'gamemode'}
                  <option value="survival">Survival</option><option value="creative">Creative</option><option value="adventure">Adventure</option>
                {:else if entry.key === 'difficulty'}
                  <option value="peaceful">Peaceful</option><option value="easy">Easy</option><option value="normal">Normal</option><option value="hard">Hard</option>
                {:else}
                  <option value="visitor">Visitor</option><option value="member">Member</option><option value="operator">Operator</option>
                {/if}
              </select>
            {:else if ['allow-cheats', 'online-mode', 'white-list', 'texturepack-required'].includes(entry.key)}
              <select value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLSelectElement).value)} class="input flex-1 text-xs py-1.5">
                <option value="true">true</option><option value="false">false</option>
              </select>
            {:else if ['server-port', 'max-players', 'view-distance', 'tick-distance'].includes(entry.key)}
              <input type="number" value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)} class="input flex-1 text-xs py-1.5" />
            {:else}
              <input type="text" value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)} class="input flex-1 text-xs py-1.5" />
            {/if}
          </div>
        {/each}
      </div>
    </div>

    <div class="card">
      <h2 class="card-header">All Entries</h2>
      <div class="overflow-x-auto">
        <table class="w-full text-xs">
          <thead><tr class="text-deep-400 border-b border-deep-600/30 uppercase tracking-wider">
            <th class="text-left py-2 px-3 font-medium">Key</th>
            <th class="text-left py-2 px-3 font-medium">Value</th>
            <th class="text-left py-2 px-3 font-medium">Comment</th>
          </tr></thead>
          <tbody>
            {#each properties.filter(p => p.key) as entry}
              <tr class="border-b border-deep-700/20 hover:bg-deep-800/30">
                <td class="py-1.5 px-3 font-mono text-bedrock-400">{entry.key}</td>
                <td class="py-1.5 px-3">{entry.value}</td>
                <td class="py-1.5 px-3 text-deep-500">{entry.inline_comment}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}
</div>
