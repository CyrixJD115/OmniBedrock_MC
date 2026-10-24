import adapter_node from '@sveltejs/adapter-node';
import adapter_static from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// The default build is a static SPA: the Rust API binary embeds frontend/build
// whole and serves it, so one executable ships the entire panel. Split Docker
// deployments set ADAPTER=node to get the SvelteKit node server instead.
const useNodeAdapter = process.env.ADAPTER === 'node';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: useNodeAdapter
      ? adapter_node()
      : adapter_static({ fallback: 'index.html', strict: false }),
    alias: {
      '$lib': './src/lib',
      '$components': './src/lib/components',
      '$stores': './src/stores',
      '$types': './src/types',
    },
  },
};

export default config;
