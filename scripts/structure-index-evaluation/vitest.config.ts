import { defineConfig, mergeConfig } from 'vitest/config';
import project from '../../vite.config';

export default mergeConfig(project, defineConfig({
  test: { include: ['scripts/structure-index-evaluation/*.test.ts'] },
}));
