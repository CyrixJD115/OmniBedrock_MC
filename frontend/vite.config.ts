import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 17755,
    proxy: {
      '/api': {
        target: 'http://localhost:17754',
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
