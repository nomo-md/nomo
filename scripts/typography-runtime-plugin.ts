import { resolve } from 'node:path';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname } from 'node:path';
import { build, type Plugin } from 'vite';

/** Build the offline export runtime once per Vite build/server instance. */
export function typographyRuntimePlugin(): Plugin {
  let source: Promise<string> | undefined;
  return {
    name: 'nomo-typography-runtime',
    resolveId(id) {
      if (id === 'virtual:nomo-typography-runtime' || id === 'virtual:nomo-katex-css')
        return '\0' + id;
    },
    async load(id) {
      if (id === '\0virtual:nomo-katex-css') {
        const path = createRequire(import.meta.url).resolve('katex/dist/katex.min.css');
        let css = await readFile(path, 'utf8');
        for (const match of [...css.matchAll(/url\((fonts\/[^)]+)\)/g)]) {
          const extension = match[1].split('.').at(-1);
          const bytes = await readFile(resolve(dirname(path), match[1]));
          css = css.replace(
            match[0],
            `url(data:font/${extension};base64,${bytes.toString('base64')})`,
          );
        }
        return `export default ${JSON.stringify(css)}`;
      }
      if (id !== '\0virtual:nomo-typography-runtime') return;
      source ??= (async () => {
        const result = await build({
          configFile: false,
          logLevel: 'error',
          build: {
            write: false,
            minify: true,
            lib: {
              entry: resolve('src/lib/typography/exportRuntime.ts'),
              formats: ['iife'],
              name: 'NomoTypography',
            },
          },
        });
        const output = Array.isArray(result) ? result[0] : result;
        if (!('output' in output)) throw new Error('Typography runtime build produced no output');
        const entry = output.output.find((chunk) => chunk.type === 'chunk' && chunk.isEntry);
        if (!entry || entry.type !== 'chunk') throw new Error('Typography runtime entry missing');
        return entry.code;
      })();
      return `export default ${JSON.stringify(await source)}`;
    },
    handleHotUpdate(context) {
      if (context.file.replace(/\\/g, '/').includes('/src/lib/typography/')) {
        source = undefined;
        const module = context.server.moduleGraph.getModuleById(
          '\0virtual:nomo-typography-runtime',
        );
        if (module) context.server.moduleGraph.invalidateModule(module);
      }
    },
  };
}
