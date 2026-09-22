import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', outDir: 'dist' },
  test: {
    // The end-to-end specs run under WebdriverIO with mocha globals, which Vitest does not
    // provide. Without this, Vitest collects them by its default glob and reports a dozen
    // load failures that have nothing to do with the tests themselves.
    include: ['tests/unit/**/*.test.ts', 'tests/perf/**/*.spec.ts'],
    exclude: ['tests/e2e/**', 'node_modules/**', 'dist/**'],
  },
});
