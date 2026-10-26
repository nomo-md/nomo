import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { typographyRuntimePlugin } from './scripts/typography-runtime-plugin';

const projectRequire = createRequire(import.meta.url);
const markdownlintEntry = projectRequire.resolve('markdownlint');
const workerEntityDecoder = createRequire(markdownlintEntry).resolve(
  'decode-named-character-reference',
);

export default defineConfig({
  plugins: [svelte(), svelteTesting(), typographyRuntimePlugin()],
  base: './',
  clearScreen: false,
  resolve: {
    // markdownlint 的传递依赖默认命中 DOM 实现；主构建和开发 Worker 都改用无 DOM 入口。
    alias: [
      // Svelte 会保留组件顶层闭包；在模块解析时隔离测试实现，避免进入普通产物。
      ...(process.env.VITE_NOMO_AUTOMATION === '1' ? [] : [{
        find: './services/automationBridge',
        replacement: fileURLToPath(new URL('./src/app/services/automationDisabled.ts', import.meta.url)),
      }]),
      {
        find: /^decode-named-character-reference$/,
        replacement: workerEntityDecoder,
      },
    ],
  },
  optimizeDeps: {
    // 避免开发模式把 markdownlint 预打包成仍引用 document 的浏览器版本。
    exclude: ['markdownlint', 'markdownlint/sync'],
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/testSetup.ts'],
  },
});
