// `vitest/config` re-exports Vite's `defineConfig` with the `test` block typed.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Tauri drives the dev server on a fixed port and expects the build output in `dist/`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'es2022',
    sourcemap: true,
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./vitest.setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
