import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import { viteSingleFile } from 'vite-plugin-singlefile';

/** An offline comparison artifact that embeds the actual current KP worker and adapter. */
export default defineConfig({
  base: './',
  plugins: [viteSingleFile()],
  build: {
    outDir: '.artifacts/kp-comparison-build',
    emptyOutDir: false,
    rollupOptions: { input: resolve('scripts/fixtures/kp-comparison.html') },
  },
});
