import type { DiagramRenderer } from './render';
import type { MermaidThemeDefinition } from '../theme/types';

type MermaidApi = typeof import('mermaid').default;

let mermaidPromise: Promise<MermaidApi> | null = null;
let initializedThemeKey: string | null = null;
let renderSequence = 0;

export function createMermaidDiagramRenderer(): DiagramRenderer {
  return {
    async renderMermaid(code, options) {
      try {
        const mermaid = await loadMermaid(options.theme);
        renderSequence += 1;
        const id = `nomo-${hashText(code)}-${renderSequence}`;
        const result = await mermaid.render(id, code);
        return { svg: result.svg };
      } catch (error) {
        return {
          svg: '',
          error: error instanceof Error ? error.message : 'Mermaid 渲染失败',
        };
      }
    },
  };
}

async function loadMermaid(theme: MermaidThemeDefinition): Promise<MermaidApi> {
  mermaidPromise ??= import('mermaid').then(({ default: mermaid }) => {
    // 图标数据随应用打包，使用时才加载，保证桌面与 Quick Look 离线可用。
    mermaid.registerIconPacks([
      {
        name: 'logos',
        loader: () => import('@iconify-json/logos').then((module) => module.icons),
      },
    ]);
    return mermaid;
  });
  const mermaid = await mermaidPromise;
  const themeKey = JSON.stringify(theme);
  if (initializedThemeKey !== themeKey) {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      theme: theme.theme,
      themeVariables: theme.themeVariables,
    });
    initializedThemeKey = themeKey;
  }
  return mermaid;
}

function hashText(text: string): string {
  let hash = 0;
  for (let index = 0; index < text.length; index += 1) {
    hash = (hash * 31 + text.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16);
}
