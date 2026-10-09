import { restoreMermaidExportSize } from '../../lib/services/mermaidDiagramView';
import {
  exportHtmlFile,
  exportPdfFromHtml,
  readFileAsBase64,
} from '../../lib/desktop/tauriStorage';
import { logDebug, logInfo, logWarn } from '../../lib/services/logger';
import exportCssContent from '../styles/export-document.css?inline';
import typographyRuntime from 'virtual:nomo-typography-runtime';
import katexCss from 'virtual:nomo-katex-css';
import { getTypographyOptions } from '../../lib/typography/options';

const REMOTE_IMAGE_FETCH_TIMEOUT_MS = 8_000;
const PDF_OUTLINE_MARKER_HOST = 'nomo-pdf-outline.invalid';

interface PdfOutlineEntry {
  marker_uri: string;
  title: string;
  level: number;
}

export interface ExportDocumentInput {
  /** 当前文档 Markdown 内容（用于 fallback 或元数据）。 */
  markdown: string;
  /** 编辑器渲染后的 HTML，从 editorHost.innerHTML 获取。 */
  renderedHtml: string;
  /** 当前文档磁盘路径，用于解析相对图片路径。 */
  documentPath: string | null;
  /** 导出文件名建议，不含扩展名。 */
  suggestedFileName: string;
  /** 文档标题，用于 HTML <title>。 */
  title: string;
}

export interface ExportResult {
  success: boolean;
  filePath?: string;
  cancelled?: boolean;
  error?: string;
  warnings?: string[];
}

/**
 * 导出当前文档为单文件 HTML。
 */
export async function exportHtml(input: ExportDocumentInput): Promise<ExportResult> {
  return perfAsync('exportService', 'exportHtml', async () => {
    logInfo('exportService', '开始导出 HTML', {
      title: input.title,
      documentPath: input.documentPath,
    });

    const { html, warnings } = await buildExportDocument(
      input.renderedHtml,
      input.documentPath,
      input.title,
    );

    const defaultName = input.suggestedFileName || 'Untitled';
    const filePath = await pickSavePath(`${defaultName}.html`, [
      { name: 'HTML', extensions: ['html', 'htm'] },
    ]);

    if (!filePath) {
      logInfo('exportService', '用户取消保存 HTML');
      return { success: false, cancelled: true };
    }

    const result = await exportHtmlFile({ html_content: html, file_path: filePath });
    logInfo('exportService', 'HTML 导出完成', { filePath, bytes: result.bytes_written });
    return { success: true, filePath, warnings };
  }).catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    logWarn('exportService', 'HTML 导出失败', { error: message });
    return { success: false, error: message };
  });
}

/**
 * 导出当前文档为 PDF（复用同一份内嵌图片的 HTML）。
 */
export async function exportPdf(input: ExportDocumentInput): Promise<ExportResult> {
  return perfAsync('exportService', 'exportPdf', async () => {
    logInfo('exportService', '开始导出 PDF', {
      title: input.title,
      documentPath: input.documentPath,
    });

    const { html, warnings } = await buildExportDocument(
      input.renderedHtml,
      input.documentPath,
      input.title,
    );

    const defaultName = input.suggestedFileName || 'Untitled';
    const filePath = await pickSavePath(`${defaultName}.pdf`, [
      { name: 'PDF', extensions: ['pdf'] },
    ]);

    if (!filePath) {
      logInfo('exportService', '用户取消保存 PDF');
      return { success: false, cancelled: true };
    }

    const pdfDocument = addPdfOutlineMarkers(html);
    const result = await exportPdfFromHtml({
      html_content: pdfDocument.html,
      file_path: filePath,
      paper_size: 'A4',
      orientation: 'portrait',
      margins: { top: 20, right: 20, bottom: 20, left: 20 },
      print_background: true,
      outline: pdfDocument.outline,
    });

    const mergedWarnings = [...warnings, ...(result.warnings ?? [])];
    logInfo('exportService', 'PDF 导出完成', {
      filePath,
      bytes: result.bytes_written,
      outlineCount: pdfDocument.outline.length,
      warningCount: mergedWarnings.length,
    });
    return { success: true, filePath, warnings: mergedWarnings };
  }).catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    logWarn('exportService', 'PDF 导出失败', { error: message });
    return { success: false, error: message };
  });
}

/**
 * 为 PDF 打印副本加入标题定位标记。标记链接会在 Rust 后处理中转换为 PDF 书签并移除。
 */
export function addPdfOutlineMarkers(html: string): { html: string; outline: PdfOutlineEntry[] } {
  const parser = new DOMParser();
  const doc = parser.parseFromString(html, 'text/html');
  const headings = Array.from(
    doc.querySelectorAll(
      '.nomo-export h1, .nomo-export h2, .nomo-export h3, .nomo-export h4, .nomo-export h5, .nomo-export h6',
    ),
  );
  const outline: PdfOutlineEntry[] = [];
  const jobId = createPdfOutlineJobId();

  for (const heading of headings) {
    const title = (heading.textContent ?? '').replace(/\s+/g, ' ').trim();
    if (!title) {
      continue;
    }

    const level = Number.parseInt(heading.tagName.slice(1), 10);
    const markerUri = `https://${PDF_OUTLINE_MARKER_HOST}/${jobId}/${outline.length}`;
    const marker = doc.createElement('a');
    marker.className = 'nomo-pdf-outline-marker';
    marker.href = markerUri;
    marker.setAttribute('aria-hidden', 'true');
    marker.setAttribute('tabindex', '-1');
    heading.classList.add('nomo-pdf-outline-heading');
    heading.prepend(marker);
    outline.push({ marker_uri: markerUri, title, level });
  }

  if (outline.length > 0) {
    const style = doc.createElement('style');
    style.textContent = `
.nomo-pdf-outline-heading { position: relative; }
.nomo-pdf-outline-marker {
  position: absolute;
  top: 0;
  left: 0;
  display: block;
  width: 1px;
  height: 1px;
  overflow: hidden;
  color: transparent;
  font-size: 0;
  line-height: 0;
  text-decoration: none;
}
`;
    doc.head.append(style);
  }

  return {
    html: `<!DOCTYPE html>\n${doc.documentElement.outerHTML}`,
    outline,
  };
}

function createPdfOutlineJobId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/**
 * 构建完整导出 HTML 文档字符串。
 * - 清理编辑器 UI 痕迹
 * - 注入导出专用 CSS
 * - 图片路径保持原样（P1 阶段再内嵌 base64）
 */
export async function buildExportDocument(
  renderedHtml: string,
  documentPath: string | null,
  title: string,
): Promise<{ html: string; warnings: string[] }> {
  const bodyHtml = cleanEditorArtifacts(renderedHtml);
  const { html, warnings } = await inlineLocalImages(bodyHtml, documentPath);
  const documentHtml = createExportHtmlDocument(html, title, exportCssContent);
  return { html: documentHtml, warnings };
}

/**
 * 清理编辑器 UI 痕迹，只保留正文渲染结果。
 */
export function cleanEditorArtifacts(htmlFragment: string): string {
  const parser = new DOMParser();
  const doc = parser.parseFromString(htmlFragment, 'text/html');
  const root = doc.body;

  if (!root.children.length) {
    return htmlFragment;
  }

  restoreMermaidExportSize(root);
  // Remove visual layout while retaining the semantic soft/hard break distinction.
  root.querySelectorAll('[data-kp-owned]').forEach((node) => node.remove());
  root.querySelectorAll('[data-kp-reflow],[data-kp-geometry]').forEach((node) => {
    node.removeAttribute('data-kp-reflow');
    node.removeAttribute('data-kp-geometry');
  });
  root.querySelectorAll<HTMLElement>('[data-kp-original-style]').forEach((node) => {
    const style = node.getAttribute('data-kp-original-style');
    if (style) node.setAttribute('style', style);
    else node.removeAttribute('style');
    node.removeAttribute('data-kp-original-style');
    node.removeAttribute('data-kp-layout');
  });
  root.querySelectorAll<HTMLElement>('.kp-unit').forEach((node) => {
    if (node.hasAttribute('data-kp-unit-original-style')) {
      const style = node.getAttribute('data-kp-unit-original-style');
      if (style) node.setAttribute('style', style);
      else node.removeAttribute('style');
      node.classList.remove('kp-unit');
      node.removeAttribute('data-kp-unit-original-style');
    } else node.replaceWith(...node.childNodes);
  });
  root.querySelectorAll<HTMLElement>('span[data-nomo-break]').forEach((node) => {
    const br = doc.createElement('br');
    br.dataset.nomoBreak = node.dataset.nomoBreak;
    node.replaceWith(br);
  });

  // 移除 ProseMirror 选区高亮、挂件、光标等编辑器痕迹。
  const selectorsToRemove = [
    '.ProseMirror-selectednode',
    '.ProseMirror-widget',
    '.ProseMirror-cursor',
    '.image-node-fullscreen-button',
    '.image-size-editor-overlay',
    '.image-fullscreen-overlay',
    '.image-node-placeholder',
    '.code-copy-button',
    '.code-edit-area',
    '.inline-comment-editor',
    '.inline-comment-edit-hint',
    '.footnote-backref',
    '.table-resize-handle',
    '.mermaid-block-fullscreen-button',
    '.callout-type-picker',
    '[contenteditable="false"]:not(.image-node):not(.mermaid-block):not(.table-widget):not(.horizontal-rule-node):not(.math-inline):not(.math-block):not(.footnote-ref)',
  ];

  for (const selector of selectorsToRemove) {
    const elements = root.querySelectorAll(selector);
    elements.forEach((el) => el.remove());
  }

  // 移除所有 style 属性中可能包含光标/选区的痕迹。
  root.querySelectorAll('[class*="ProseMirror"]').forEach((el) => {
    const classes = Array.from(el.classList).filter(
      (cls) =>
        !cls.startsWith('ProseMirror') &&
        cls !== 'code-card' &&
        cls !== 'callout-card' &&
        cls !== 'table-scroll',
    );
    el.className = classes.join(' ');
  });

  // 图片节点只保留 img 本身，同时将对齐样式从 wrapper 迁移到 img。
  root.querySelectorAll('.image-node').forEach((node) => {
    const img = node.querySelector('img');
    if (img) {
      const wrapper = node as HTMLElement;
      // ImageNodeView 将对齐（居中/右对齐/左对齐）通过内联样式设置在 .image-node wrapper 上，
      // 导出 HTML 中没有 .image-node 包装，需要将这些样式迁移到 <img> 内联样式。
      if (wrapper.style.display === 'block') {
        img.style.display = 'block';
      }
      if (wrapper.style.marginLeft) {
        img.style.marginLeft = wrapper.style.marginLeft;
      }
      if (wrapper.style.marginRight) {
        img.style.marginRight = wrapper.style.marginRight;
      }
      node.replaceWith(img);
    } else {
      node.remove();
    }
  });

  // 水平分割线节点只保留 hr 本身。
  root.querySelectorAll('.horizontal-rule-node').forEach((node) => {
    const hr = node.querySelector('hr');
    if (hr) {
      node.replaceWith(hr);
    } else {
      node.remove();
    }
  });

  // 移除所有 contenteditable 属性——导出 HTML 是只读文档，
  // 若保留 contenteditable="true"，浏览器点击时会显示 focus outline 黑边框。
  root.querySelectorAll('[contenteditable]').forEach((el) => {
    el.removeAttribute('contenteditable');
  });

  return root.innerHTML;
}

/**
 * 创建完整 HTML 文档外壳。
 */
export function createExportHtmlDocument(
  bodyHtml: string,
  title: string,
  cssContent: string,
): string {
  const escapedTitle = escapeHtml(title || 'Exported Document');
  const configuration = JSON.stringify(getTypographyOptions()).replace(/</g, '\\u003c');
  const rootStyle = typeof document === 'undefined' ? null : getComputedStyle(document.documentElement);
  const font = rootStyle?.getPropertyValue('--md-editor-font-body').trim() ?? '';
  const fontStyle = font && !/[<>]/.test(font) ? `:root{--nomo-export-font-body:${font};}` : '';
  const fontSize = parseFloat(rootStyle?.getPropertyValue('--md-editor-font-size') ?? '');
  const lineHeight = parseFloat(rootStyle?.getPropertyValue('--md-editor-line-height') ?? '');
  const metricsStyle = `.nomo-export{${Number.isFinite(fontSize) ? `font-size:${fontSize}px;` : ''}${Number.isFinite(lineHeight) ? `line-height:${lineHeight};` : ''}}`;
  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapedTitle}</title>
  <style>
${cssContent}
${bodyHtml.includes('katex') ? katexCss : ''}
${fontStyle}
${metricsStyle}
  </style>
</head>
<body>
  <article class="nomo-export markdown-body">
${bodyHtml}
  </article>
  <script id="nomo-typography-options" type="application/json">${configuration}</script>
  <script>${typographyRuntime.replace(/<\/script/gi, '<\\/script')}</script>
</body>
</html>
`;
}

/**
 * 弹出保存路径选择对话框。
 */
async function pickSavePath(
  defaultName: string,
  filters: Array<{ name: string; extensions: string[] }>,
): Promise<string | null> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  return save({
    defaultPath: defaultName,
    filters,
  });
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export async function inlineLocalImages(
  html: string,
  documentPath: string | null,
): Promise<{ html: string; warnings: string[] }> {
  const parser = new DOMParser();
  const doc = parser.parseFromString(html, 'text/html');
  const images = Array.from(doc.querySelectorAll('img[src]'));
  const warnings: string[] = [];

  for (const img of images) {
    const originalSrc = img.getAttribute('src') ?? '';
    const result = await inlineImageSrc(originalSrc, documentPath);

    if (result.dataUrl) {
      img.setAttribute('src', result.dataUrl);
    } else if (result.warning) {
      warnings.push(result.warning);
    }
  }

  return { html: doc.body.innerHTML, warnings };
}

async function inlineImageSrc(
  src: string,
  documentPath: string | null,
): Promise<{ dataUrl?: string; warning?: string }> {
  const trimmed = src.trim();

  // data: 和 blob: 已经内嵌，保持原样。
  if (/^(data:|blob:)/i.test(trimmed)) {
    return {};
  }

  // 远程图片 fetch 后转为 base64 内嵌；设置超时，避免导出被慢请求无限阻塞。
  if (/^https?:/i.test(trimmed)) {
    return fetchImageAsBase64(trimmed, REMOTE_IMAGE_FETCH_TIMEOUT_MS).catch((error) => ({
      warning: `远程图片未能内嵌（${error instanceof Error ? error.message : String(error)}）：${trimmed}`,
    }));
  }

  // asset:// 或 http://asset.localhost/ 是 Tauri 本地资源协议，尝试 fetch 转 base64。
  if (isAssetUrl(trimmed)) {
    return fetchImageAsBase64(trimmed, REMOTE_IMAGE_FETCH_TIMEOUT_MS).catch((error) => ({
      warning: `图片未能内嵌（${error instanceof Error ? error.message : String(error)}）：${trimmed}`,
    }));
  }

  // file:// 转换为本地路径。
  const localPath = fileUrlToLocalPath(trimmed) ?? trimmed;

  // 解析为绝对路径。
  const resolved = await resolveImagePath(localPath, documentPath);
  if (resolved.warning) {
    return { warning: resolved.warning };
  }

  const absolutePath = resolved.absolutePath;
  if (!absolutePath) {
    return { warning: `无法解析图片路径：${trimmed}` };
  }

  try {
    const { data_url } = await readFileAsBase64(absolutePath);
    return { dataUrl: data_url };
  } catch (error) {
    return {
      warning: `图片读取失败（${error instanceof Error ? error.message : String(error)}）：${trimmed}`,
    };
  }
}

function isAssetUrl(src: string): boolean {
  const lower = src.toLowerCase();
  return (
    lower.startsWith('asset://') ||
    lower.startsWith('http://asset.localhost/') ||
    lower.startsWith('https://asset.localhost/')
  );
}

async function fetchImageAsBase64(
  url: string,
  timeoutMs: number,
): Promise<{ dataUrl?: string; warning?: string }> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) {
      throw new Error(`fetch failed: ${response.status}`);
    }
    const blob = await response.blob();
    const dataUrl = await blobToDataUrl(blob);
    return { dataUrl };
  } catch (error) {
    if (controller.signal.aborted) {
      throw new Error(`fetch timeout after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    window.clearTimeout(timeout);
  }
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => {
      const result = reader.result;
      if (typeof result === 'string') {
        resolve(result);
      } else {
        reject(new Error('FileReader 结果不是字符串'));
      }
    };
    reader.onerror = () => reject(reader.error ?? new Error('读取 Blob 失败'));
    reader.readAsDataURL(blob);
  });
}

function fileUrlToLocalPath(src: string): string | null {
  if (!src.toLowerCase().startsWith('file://')) {
    return null;
  }
  const withoutProtocol = src.slice('file://'.length);
  // Windows file:///C:/path -> /C:/path -> C:/path
  if (withoutProtocol.startsWith('/')) {
    const path = withoutProtocol.slice(1);
    if (/^[a-zA-Z]:[\/]/.test(path)) {
      return path;
    }
    // Unix file:///home/user/path -> /home/user/path
    return withoutProtocol;
  }
  return withoutProtocol;
}

export async function resolveImagePath(
  src: string,
  documentPath: string | null,
): Promise<{ absolutePath?: string; warning?: string }> {
  const trimmed = src.trim();
  if (!trimmed) {
    return { warning: '图片 src 为空' };
  }

  // 已经是本地绝对路径。
  if (isAbsoluteLocalPath(trimmed)) {
    return { absolutePath: trimmed };
  }

  // 相对路径需要基于文档目录解析。
  if (!documentPath) {
    return { warning: `当前文档未保存，无法解析相对图片路径：${trimmed}` };
  }

  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const result = await invoke<{
      src: string;
      display_src: string;
      exists: boolean;
      absolute_path?: string | null;
      error?: string | null;
    }>('resolve_image_asset', {
      input: {
        document_path: documentPath,
        src: trimmed,
      },
    });

    if (!result.exists) {
      return {
        warning: result.error ?? `图片文件不存在：${trimmed}`,
      };
    }

    const absolutePath = result.absolute_path ?? result.display_src;
    return { absolutePath };
  } catch (error) {
    return {
      warning: `解析图片路径失败（${error instanceof Error ? error.message : String(error)}）：${trimmed}`,
    };
  }
}

function isAbsoluteLocalPath(src: string): boolean {
  // Windows: C:\path 或 C:/path
  if (/^[a-zA-Z]:[\/]/.test(src)) {
    return true;
  }
  // Unix: /path
  if (src.startsWith('/')) {
    return true;
  }
  return false;
}

// 简化 perfAsync 本地实现，避免循环依赖。
function perfAsync<T>(namespace: string, name: string, fn: () => Promise<T>): Promise<T> {
  const start = performance.now();
  return fn().finally(() => {
    logDebug(namespace, `${name} 耗时`, { ms: Math.round(performance.now() - start) });
  });
}
