import type { DiagramRenderer } from './render';
import type { MermaidThemeDefinition } from '../theme/types';
import { createRenderTaskQueue } from './renderTaskQueue';
import { MermaidSvgCache } from './mermaidSvgCache';

type MermaidApi = typeof import('mermaid').default;

let mermaidPromise: Promise<MermaidApi> | null = null;
let initializedThemeKey: string | null = null;
let renderSequence = 0;
// Mermaid 的主题配置是全局状态；初始化与渲染必须在同一串行任务内。
const renderTask = createRenderTaskQueue(1);
const svgCache = new MermaidSvgCache();
let fontGeneration = 0;
// 渲染服务与 WebView 同寿命；唯一监听只清理纯字符串缓存，不引用任何编辑器实例。
if (typeof document !== 'undefined') document.fonts?.addEventListener('loadingdone', () => {
  fontGeneration++;
  svgCache.clear();
});

export function createMermaidDiagramRenderer(): DiagramRenderer {
  return {
    async renderMermaid(code, options) {
      renderSequence += 1;
      const id = `nomo-${hashText(code)}-${renderSequence}`;
      try {
        return await renderTask(async () => {
          const key = JSON.stringify([code, options.theme, fontGeneration]);
          const saved = svgCache.get(key, id);
          if (saved !== undefined) return { svg: saved };
          const mermaid = await loadMermaid(options.theme);
          options.signal?.throwIfAborted();
          const fonts = fontGeneration;
          // 大图测量限制在独立布局容器内，避免逐节点测量牵动整个编辑区。
          const surface = document.createElement('div');
          surface.style.cssText = `position:fixed;left:0;top:0;width:${document.documentElement.clientWidth}px;height:0;overflow:hidden;contain:layout style paint;visibility:hidden;pointer-events:none`;
          document.body.append(surface);
          // 大图分批让出主线程后，关闭或切换标签可及时使下一批失效。
          const cancel = () => surface.remove();
          options.signal?.addEventListener('abort', cancel, { once: true });
          try {
            const result = await mermaid.render(id, code, surface);
            options.signal?.throwIfAborted();
            await yieldForLargeSvg(result.svg, options.signal);
            if (fonts === fontGeneration) svgCache.put(key, result.svg);
            await yieldForLargeSvg(result.svg, options.signal);
            return { svg: result.svg };
          } finally {
            options.signal?.removeEventListener('abort', cancel);
            if (options.signal?.aborted) svgCache.delete(key);
            surface.remove();
          }
        }, options.signal);
      } catch (error) {
        return {
          svg: '',
          error: error instanceof Error ? error.message : 'Mermaid 渲染失败',
        };
      } finally {
        // Mermaid 失败路径会留下请求专属的临时容器；精确清理，不影响其他图表。
        if (typeof document !== 'undefined') {
          document.getElementById(`d${id}`)?.remove();
          document.getElementById(`i${id}`)?.remove();
        }
      }
    },
  };
}

/** 将大图的库渲染、缓存整理和视图提交分开，避免串成同一个长任务。 */
async function yieldForLargeSvg(svg: string, signal?: AbortSignal): Promise<void> {
  if (svg.length < 128 * 1024) return;
  const scheduler = (globalThis as { scheduler?: { yield(): Promise<void> } }).scheduler;
  if (scheduler?.yield) await scheduler.yield();
  else await new Promise<void>(resolve => setTimeout(resolve, 0));
  signal?.throwIfAborted();
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
