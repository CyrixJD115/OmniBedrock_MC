import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    host: '0.0.0.0',
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
