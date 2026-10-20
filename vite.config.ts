import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed port and no clearing of its own output.
export default defineConfig({
  root: 'ui',
  plugins: [svelte()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/target/**'] } },
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
});
