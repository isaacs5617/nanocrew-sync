import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const host = process.env['TAURI_DEV_HOST'];

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  define: {
    'import.meta.env.VITE_SENTRY_DSN': JSON.stringify(process.env['VITE_SENTRY_DSN'] ?? ''),
    'import.meta.env.VITE_APP_VERSION': JSON.stringify(process.env['npm_package_version'] ?? ''),
  },
  server: {
    port: 5173,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    target: 'chrome105',
    minify: !process.env['TAURI_ENV_DEBUG'] ? 'esbuild' : false,
    sourcemap: !!process.env['TAURI_ENV_DEBUG'],
  },
});
