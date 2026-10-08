import 'katex/dist/katex.min.css';
import './preview.css';
import { createDomTypography } from '../lib/typography/dom';
import { setTypographyOptions } from '../lib/typography/options';
import { DEFAULT_TYPOGRAPHY, type TypographyController } from '../lib/typography/types';
import type { MermaidFullscreenBinding } from '../lib/services/mermaidDiagramView';
import {
  applyQuickLookAppearance,
  renderMarkdownPreview,
  renderQuickLookMermaidBlocks,
  type QuickLookPreviewPayload,
} from './preview';

let mermaidBindings: MermaidFullscreenBinding[] = [];
let renderGeneration = 0;
let typography: TypographyController | undefined;

declare global {
  interface Window {
    /** 当前 Quick Look 文档数据；由原生扩展在静态渲染页加载完成后设置。 */
    __NOMO_QUICKLOOK_PAYLOAD__?: QuickLookPreviewPayload | null;
    /** 接收原生扩展安全传入的数据并刷新页面，不通过 HTML/JavaScript 字符串拼接传输正文。 */
    __NOMO_RENDER_QUICKLOOK__?: (payload: QuickLookPreviewPayload) => void;
  }
}

/**
 * 把一份原生 Quick Look 数据挂载到当前静态渲染页。
 *
 * @param payload 完整 Markdown、文件上下文与可选外观偏好；`null` 表示原生侧尚未传入数据。
 * @returns 无返回值；副作用是更新主题和 `#quicklook-root` 的预览内容。
 */
function mountQuickLookPreview(payload: QuickLookPreviewPayload | null | undefined) {
  const root = document.getElementById('quicklook-root');
  if (!root || !payload) return;

  renderGeneration += 1;
  typography?.destroy();
  setTypographyOptions(payload.typography ?? DEFAULT_TYPOGRAPHY);
  const generation = renderGeneration;
  for (const binding of mermaidBindings) binding.dispose();
  mermaidBindings = [];

  if (typeof payload.markdown !== 'string') {
    root.innerHTML = `
      <section class="quicklook-empty">
        <strong>无法生成预览</strong>
        <span>Quick Look 没有收到可渲染的 Markdown 内容。</span>
      </section>
    `;
    return;
  }

  const resolvedTheme = applyQuickLookAppearance(payload.appearance);
  const fontSize = Number.isFinite(payload.fontSize) ? payload.fontSize! : 16;
  const lineHeight = Number.isFinite(payload.lineHeight) ? payload.lineHeight! : 1.75;
  document.documentElement.style.setProperty(
    '--md-editor-font-size', `${Math.min(22, Math.max(14, fontSize))}px`,
  );
  document.documentElement.style.setProperty(
    '--md-editor-line-height', String(Math.min(2.1, Math.max(1.4, lineHeight))),
  );
  root.innerHTML = renderMarkdownPreview(payload.markdown, {
    fileName: payload.fileName,
    documentDirectory: payload.documentDirectory,
  });
  typography = createDomTypography(root);
  void renderQuickLookMermaidBlocks(root, resolvedTheme.editorTheme.mermaid).then((bindings) => {
    if (generation !== renderGeneration) {
      for (const binding of bindings) binding.dispose();
      return;
    }
    mermaidBindings = bindings;
  });
}

window.__NOMO_RENDER_QUICKLOOK__ = (payload) => {
  window.__NOMO_QUICKLOOK_PAYLOAD__ = payload;
  mountQuickLookPreview(payload);
};

mountQuickLookPreview(window.__NOMO_QUICKLOOK_PAYLOAD__);
