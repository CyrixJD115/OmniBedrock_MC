<script lang="ts">
  /** Theme, light/dark mode, custom accent, density and size (per browser). */
  import { Palette, Sun, Moon, Monitor, Check } from '@lucide/svelte';
  import { THEMES, THEME_MODES, theme, themeMode, customAccent, uiDensity, uiFontScale, isHexColor } from '$lib/theme';

  // The color input emits #rrggbb live while dragging; seeded from the saved accent.
  let customHexInput = $state($customAccent);
  function applyCustomAccent(hex: string) {
    customHexInput = hex;
    if (!isHexColor(hex)) return;
    customAccent.set(hex.toLowerCase());
    theme.set('custom');
  }
  function pickPreset() {
    theme.set('custom');
  }
</script>

  <div class="card">
    <h2 class="card-header"><Palette size={13} /> Appearance</h2>
    <div class="space-y-4">
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Theme</p>
        <div class="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
          {#each THEMES as t (t.id)}
            <button onclick={() => theme.set(t.id)}
                    aria-pressed={$theme === t.id}
                    class={"text-left p-2.5 border-2 transition-all " + ($theme === t.id
                      ? 'border-bedrock-400 bg-bedrock-500/10'
                      : 'border-deep-600/50 bg-deep-900/40 hover:border-deep-500')}>
              <span class="flex items-center gap-2">
                <span class="w-6 h-6 shrink-0 border border-black/20 grid grid-cols-2 overflow-hidden" aria-hidden="true">
                  <i class="block" style="background:{t.swatch[0]}"></i><i class="block" style="background:{t.swatch[1]}"></i>
                </span>
                {#if $theme === t.id}
                  <span class="text-xs font-bold uppercase tracking-wider text-bedrock-300 truncate">{t.label}</span>
                  <Check size={13} class="text-bedrock-400 ml-auto shrink-0" />
                {:else}
                  <span class="text-xs font-bold uppercase tracking-wider text-deep-200 truncate">{t.label}</span>
                {/if}
              </span>
              <span class="block text-[10px] text-deep-400 mt-1.5 truncate">{t.hint}</span>
            </button>
          {/each}
        </div>
      </div>
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Mode</p>
        <div class="inline-flex border-2 border-deep-600/50 bg-deep-900/40 p-0.5 gap-0.5" role="group" aria-label="Color mode">
          {#each THEME_MODES as m (m.id)}
            <button onclick={() => themeMode.set(m.id)} aria-pressed={$themeMode === m.id}
                    class={"flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold uppercase tracking-wider transition-colors " + ($themeMode === m.id
                      ? 'bg-bedrock-600 text-white'
                      : 'text-deep-300 hover:bg-deep-700/60 hover:text-deep-100')}>
              {#if m.id === 'dark'}<Moon size={12} />
              {:else if m.id === 'light'}<Sun size={12} />
              {:else}<Monitor size={12} />{/if}
              {m.label}
            </button>
          {/each}
        </div>
        <p class="text-[10px] text-deep-400 mt-2">
          Applies instantly across the panel and is remembered in this browser. “System” follows your OS setting.
        </p>
      </div>
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Custom accent</p>
        <div class="flex flex-wrap items-center gap-3">
          <input
            type="color"
            value={customHexInput}
            oninput={(e) => applyCustomAccent((e.currentTarget as HTMLInputElement).value)}
            class="w-10 h-10 cursor-pointer border-2 border-deep-600/50 bg-deep-900/40 rounded-sm p-0.5"
            aria-label="Pick a custom accent color"
          />
          <input
            class="input w-32 text-xs font-mono"
            value={customHexInput}
            onchange={(e) => applyCustomAccent((e.currentTarget as HTMLInputElement).value.trim())}
            placeholder="#38bdf8"
            aria-label="Custom accent hex"
          />
          <button onclick={pickPreset}
                  aria-pressed={$theme === 'custom'}
                  class={"text-xs uppercase tracking-wider font-semibold px-3 py-1.5 border-2 transition-colors " + ($theme === 'custom'
                    ? 'border-bedrock-400 bg-bedrock-500/10 text-bedrock-300'
                    : 'border-deep-600/50 text-deep-300 hover:border-deep-500')}>
            Use color
          </button>
          <span class="text-[10px] text-deep-400">The whole accent ramp (buttons, highlights, glows) is derived from this one color.</span>
        </div>
      </div>
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Density</p>
        <div class="inline-flex border-2 border-deep-600/50 bg-deep-900/40 p-0.5 gap-0.5" role="group" aria-label="UI density">
          <button onclick={() => uiDensity.set('comfortable')} aria-pressed={$uiDensity === 'comfortable'}
                  class={"px-3 py-1.5 text-xs font-semibold uppercase tracking-wider transition-colors " + ($uiDensity === 'comfortable'
                    ? 'bg-bedrock-600 text-white' : 'text-deep-300 hover:bg-deep-700/60 hover:text-deep-100')}>
            Comfortable
          </button>
          <button onclick={() => uiDensity.set('compact')} aria-pressed={$uiDensity === 'compact'}
                  class={"px-3 py-1.5 text-xs font-semibold uppercase tracking-wider transition-colors " + ($uiDensity === 'compact'
                    ? 'bg-bedrock-600 text-white' : 'text-deep-300 hover:bg-deep-700/60 hover:text-deep-100')}>
            Compact
          </button>
        </div>
      </div>
      <div>
        <p class="text-xs text-deep-300 uppercase tracking-wider mb-2">Interface size</p>
        <div class="flex items-center gap-3">
          <input
            type="range"
            min="90"
            max="120"
            step="5"
            value={Math.round($uiFontScale * 100)}
            oninput={(e) => uiFontScale.set(Number((e.currentTarget as HTMLInputElement).value) / 100)}
            class="w-56 accent-bedrock-500"
            aria-label="Interface font size"
          />
          <span class="text-xs font-mono text-deep-300 w-10">{Math.round($uiFontScale * 100)}%</span>
          <button onclick={() => uiFontScale.set(1)} class="btn-ghost text-xs px-2 py-1" title="Reset to 100%">Reset</button>
        </div>
        <p class="text-[10px] text-deep-400 mt-2">Scales every panel surface; combines with density.</p>
      </div>
    </div>
  </div>
