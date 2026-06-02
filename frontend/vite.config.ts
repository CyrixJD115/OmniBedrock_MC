import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 17755,
    proxy: {
      '/api': 'http://localhost:17754',
      '/ws': {
        target: 'ws://localhost:17754',
        ws: true,
      },
    },
  },
});
