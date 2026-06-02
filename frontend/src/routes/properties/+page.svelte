<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api/client';
  import { Eye, RotateCw, Save } from '@lucide/svelte';

  let properties = $state<{ key: string; value: string; comment: string; inline_comment: string }[]>([]);
  let rawText = $state('');
  let showRaw = $state(false);
  let loading = $state(true);

  onMount(async () => {
    try {
      properties = await api.getProperties();
    } catch { /* ignore */ }
    loading = false;
  });

  async function updateValue(key: string, value: string) {
    await api.updateProperty(key, value);
  }

  async function loadRaw() {
    rawText = await api.getPropertiesRaw();
    showRaw = true;
  }

  async function saveRaw() {
    await api.savePropertiesRaw(rawText);
    properties = await api.getProperties();
    showRaw = false;
  }

  const knownProps = ['server-name', 'gamemode', 'difficulty', 'allow-cheats', 'max-players',
    'online-mode', 'white-list', 'server-port', 'view-distance', 'tick-distance', 'level-name',
    'level-seed', 'default-player-permission-level', 'texturepack-required', 'player-idle-timeout'];

  let knownEntries = $derived(properties.filter(p => p.key && knownProps.includes(p.key)));
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between">
    <h1 class="text-2xl font-bold text-white">Server Properties</h1>
    <div class="flex gap-2">
      <button onclick={loadRaw} class="btn-secondary flex items-center gap-2">
        <Eye size={16} /> Raw Edit
      </button>
      <button onclick={async () => { properties = await api.getProperties(); }}
              class="btn-ghost p-2">
        <RotateCw size={16} />
      </button>
    </div>
  </div>

  {#if showRaw}
    <div class="card">
      <h2 class="card-header">Raw Editor</h2>
      <textarea bind:value={rawText}
                class="input w-full h-96 font-mono text-sm" spellcheck="false"></textarea>
      <div class="flex gap-2 mt-3">
        <button onclick={saveRaw} class="btn-primary flex items-center gap-2">
          <Save size={16} /> Save
        </button>
        <button onclick={() => showRaw = false} class="btn-secondary">Cancel</button>
      </div>
    </div>
  {:else}
    <div class="card">
      <h2 class="card-header">Properties</h2>
      <div class="space-y-3">
        {#each knownEntries as entry}
          <div class="flex items-center gap-4 p-3 rounded-lg bg-surface-800/50">
            <span class="text-sm font-mono text-bedrock-400 w-48 shrink-0">{entry.key}</span>
            {#if ['gamemode', 'difficulty', 'default-player-permission-level'].includes(entry.key)}
              <select value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLSelectElement).value)}
                      class="input flex-1">
                {#if entry.key === 'gamemode'}
                  <option value="survival">Survival</option><option value="creative">Creative</option><option value="adventure">Adventure</option>
                {:else if entry.key === 'difficulty'}
                  <option value="peaceful">Peaceful</option><option value="easy">Easy</option><option value="normal">Normal</option><option value="hard">Hard</option>
                {:else}
                  <option value="visitor">Visitor</option><option value="member">Member</option><option value="operator">Operator</option>
                {/if}
              </select>
            {:else if ['allow-cheats', 'online-mode', 'white-list', 'texturepack-required'].includes(entry.key)}
              <select value={entry.value} onchange={(e) => updateValue(entry.key, (e.target as HTMLSelectElement).value)}
                      class="input flex-1">
                <option value="true">true</option><option value="false">false</option>
              </select>
            {:else if ['server-port', 'max-players', 'view-distance', 'tick-distance'].includes(entry.key)}
              <input type="number" value={entry.value}
                     onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)}
                     class="input flex-1" />
            {:else}
              <input type="text" value={entry.value}
                     onchange={(e) => updateValue(entry.key, (e.target as HTMLInputElement).value)}
                     class="input flex-1" />
            {/if}
          </div>
        {/each}
      </div>
    </div>

    <div class="card">
      <h2 class="card-header">All Entries</h2>
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <thead>
            <tr class="text-surface-400 border-b border-surface-700">
              <th class="text-left py-2 px-3 font-medium">Key</th>
              <th class="text-left py-2 px-3 font-medium">Value</th>
              <th class="text-left py-2 px-3 font-medium">Comment</th>
            </tr>
          </thead>
          <tbody>
            {#each properties.filter(p => p.key) as entry}
              <tr class="border-b border-surface-800 hover:bg-surface-800/50">
                <td class="py-2 px-3 font-mono text-bedrock-400">{entry.key}</td>
                <td class="py-2 px-3">{entry.value}</td>
                <td class="py-2 px-3 text-surface-500 text-xs">{entry.inline_comment}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}
</div>
