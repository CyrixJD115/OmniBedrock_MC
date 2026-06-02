import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://localhost:17755',
      '/ws': {
        target: 'ws://localhost:17755',
        ws: true,
      },
    },
  },
});
