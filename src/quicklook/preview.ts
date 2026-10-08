import katex from 'katex';
import type { TypographyOptions } from '../lib/typography/types';
import MarkdownIt from 'markdown-it';
import Token from 'markdown-it/lib/token.mjs';
import { transformCalloutTokens } from '../lib/editor-core/callout/calloutParser';
import { normalizeLinkHref } from '../lib/editor-core/link';
import { createMermaidDiagramRenderer } from '../lib/services/mermaidDiagramRenderer';
import {
  bindMermaidFullscreen,
  normalizeMermaidSvgSize,
  normalizeRenderedMermaidViewport,
  type MermaidFullscreenBinding,
} from '../lib/services/mermaidDiagramView';
import type { DiagramRenderer } from '../lib/services/render';
import type { AppearancePreferences, MermaidThemeDefinition } from '../lib/theme/types';
import { applyResolvedTheme, resolveTheme } from '../app/services/themeManager';

/** Quick Look 渲染器可选的文件上下文，用于标题展示和相对资源解析。 */
export interface QuickLookPreviewOptions {
  /** 原始文档文件名；缺失或仅含空白时使用通用的 Markdown Preview 标题。 */
  fileName?: string;
  /** 原始文档父目录的绝对路径；缺失时不解析相对图片路径。 */
  documentDirectory?: string;
}

/** 原生 Quick Look 扩展传给内嵌渲染器的完整数据。 */
export interface QuickLookPreviewPayload extends QuickLookPreviewOptions {
  /** 待渲染的完整 Markdown UTF-8 文本；允许空字符串。 */
  markdown: string;
  /** Nomo 原生配置中的外观偏好；配置不可读时缺失并回退到系统明暗模式。 */
  appearance?: Partial<AppearancePreferences>;
  typography?: TypographyOptions;
  fontSize?: number;
  lineHeight?: number;
}

const CALLOUT_LABELS: Record<string, string> = {
  note: '提醒',
  tip: '建议',
  important: '重要',
  warning: '警告',
  caution: '风险',
};

const ALLOWED_TAGS = new Set([
  'a',
  'blockquote',
  'br',
  'code',
  'del',
  'div',
  'em',
  'figcaption',
  'figure',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'hr',
  'img',
  'input',
  'li',
  'mark',
  'ol',
  'p',
  'pre',
  's',
  'span',
  'strong',
  'sub',
  'sup',
  'table',
  'tbody',
  'td',
  'th',
  'thead',
  'tr',
  'u',
  'ul',
]);

const DISCARD_TAGS = new Set(['script', 'style', 'iframe', 'object', 'embed']);
const GLOBAL_ATTRS = new Set(['class', 'title']);
const ATTRS_BY_TAG: Record<string, Set<string>> = {
  br: new Set(['data-nomo-break']),
  a: new Set(['href', 'title', 'target', 'rel']),
  div: new Set(['class', 'data-callout-type', 'data-nomo-tex']),
  img: new Set(['src', 'alt', 'title', 'width', 'height', 'class', 'loading']),
  input: new Set(['class', 'type', 'checked', 'disabled', 'aria-label']),
  span: new Set(['class', 'aria-hidden', 'data-nomo-tex']),
  td: new Set(['style']),
  th: new Set(['style']),
};

const markdownIt = createQuickLookMarkdownIt();

type QuickLookInlineState = {
  src: string;
  pos: number;
  tokens: Token[];
  push(type: string, tag: string, nesting: number): Token;
};

type QuickLookBlockState = {
  src: string;
  bMarks: number[];
  tShift: number[];
  eMarks: number[];
  line: number;
  push(type: string, tag: string, nesting: number): Token;
};

/**
 * 将主应用保存的外观偏好应用到 Quick Look 文档根节点。
 *
 * 颜色和样式 token 仍由 Nomo 的主题注册表解析，本函数不维护第二套 Quick Look 主题。
 * 当偏好缺失时按系统明暗模式解析默认主题，确保正文和 Mermaid 使用同一有效主题。
 *
 * @param appearance 主应用配置中的主题模式、颜色主题和文档样式；允许缺失。
 * @param root 接收主题 dataset 与 CSS 变量的文档根节点；默认使用当前页面根节点。
 * @returns 已应用到根节点的完整解析主题，包含 Mermaid 对应主题配置。
 */
export function applyQuickLookAppearance(
  appearance: Partial<AppearancePreferences> | undefined,
  root: HTMLElement = document.documentElement,
) {
  const resolved = resolveTheme(appearance ?? {});
  return applyResolvedTheme(resolved, { root });
}

/**
 * 将 Quick Look 正文中的 Mermaid 占位块异步替换为 SVG 图表。
 *
 * 每个图表使用主应用的 Mermaid renderer 和当前颜色主题。单个图表语法错误不会阻断其他图表；
 * 失败块会显示错误及原始源码，避免静默丢失文档内容。
 *
 * @param root 已插入 Markdown HTML 的 Quick Look 根节点。
 * @param theme 当前解析主题携带的 Mermaid 配置。
 * @param renderer Mermaid 渲染器；默认复用主应用实现，测试可注入确定性实现。
 * @returns 所有成功图表的放大查看器绑定；调用方替换预览内容前必须逐个 `dispose()`。
 */
export async function renderQuickLookMermaidBlocks(
  root: HTMLElement,
  theme: MermaidThemeDefinition,
  renderer: DiagramRenderer = createMermaidDiagramRenderer(),
): Promise<MermaidFullscreenBinding[]> {
  const blocks = Array.from(root.querySelectorAll<HTMLElement>('.mermaid-block'));
  const bindings = await Promise.all(
    blocks.map(async (block): Promise<MermaidFullscreenBinding | null> => {
      const target = block.querySelector<HTMLElement>('.mermaid-block-rendered');
      const source = block.querySelector<HTMLElement>('.mermaid-block-source code');
      if (!target || !source) return null;

      const code = source.textContent?.replace(/\n$/, '') ?? '';
      if (!code.trim()) {
        showMermaidError(block, target, 'Mermaid 图表内容为空');
        return null;
      }

      try {
        const result = await renderer.renderMermaid(code, { theme });
        if (!result.svg) {
          showMermaidError(block, target, result.error || 'Mermaid 渲染失败');
          return null;
        }
        target.innerHTML = normalizeMermaidSvgSize(result.svg);
        block.classList.remove('is-pending', 'is-error');
        block.classList.add('is-rendered');
        normalizeRenderedMermaidViewport(target);
        return bindMermaidFullscreen(block, target, {
          mode: '图表查看模式',
          readable: '可读适配',
          fit: '适应全图',
          original: '原始尺寸',
          custom: '自定义',
          open: '放大查看图表',
          dialog: '图表放大预览',
          close: '关闭图表预览',
          zoomIn: '放大',
          zoomOut: '缩小',
          reset: '重置缩放',
        });
      } catch (error) {
        showMermaidError(
          block,
          target,
          error instanceof Error ? error.message : 'Mermaid 渲染失败',
        );
      }
      return null;
    }),
  );
  return bindings.filter((binding): binding is MermaidFullscreenBinding => binding !== null);
}

/**
 * 将 Mermaid 块切换到可诊断的失败状态，并保留源码供用户查看。
 *
 * @param block 当前 Mermaid 图表外壳。
 * @param target 原本承载 SVG 的状态区域。
 * @param message Mermaid 返回或捕获到的错误信息；通过 `textContent` 写入，不能注入 HTML。
 * @returns 无返回值；副作用是更新块状态类和错误文本。
 */
function showMermaidError(block: HTMLElement, target: HTMLElement, message: string): void {
  target.textContent = `Mermaid 渲染失败：${message}`;
  block.classList.remove('is-pending', 'is-rendered');
  block.classList.add('is-error');
}

/**
 * 将 Markdown 渲染为经过安全过滤的 Quick Look 正文外壳。
 *
 * @param markdown 完整 Markdown 源文；允许空字符串。
 * @param options 文件名和父目录上下文；缺失时使用通用标题且不解析相对资源。
 * @returns 可直接写入 Quick Look 根节点的 HTML 字符串。
 */
export function renderMarkdownPreview(markdown: string, options: QuickLookPreviewOptions = {}) {
  const title = escapeHtml(options.fileName?.trim() || 'Markdown Preview');
  const sanitizedBody = renderMarkdownPreviewBody(markdown, options);

  return `
    <article class="quicklook-document">
      <header class="quicklook-header">
        <div class="quicklook-kicker">Nomo Quick Look</div>
        <h1>${title}</h1>
      </header>
      <div class="quicklook-markdown rich-markdown">
        ${sanitizedBody}
      </div>
    </article>
  `;
}

export function resolvePreviewAssetSrc(src: string, documentDirectory?: string): string | null {
  const value = src.trim();
  if (!value || /^(?:javascript|vbscript|data)\s*:/i.test(value)) {
    return null;
  }

  if (/^(?:https?|file):/i.test(value)) {
    return value;
  }

  if (value.startsWith('#') || value.startsWith('mailto:')) {
    return null;
  }

  if (isAbsoluteFilePath(value)) {
    return pathToFileUrl(value);
  }

  if (!documentDirectory?.trim()) {
    return value;
  }

  const baseUrl = pathToDirectoryFileUrl(documentDirectory);
  try {
    return new URL(value.replace(/\\/g, '/'), baseUrl).href;
  } catch {
    return null;
  }
}

function createQuickLookMarkdownIt() {
  const md = MarkdownIt('commonmark', {
    html: true,
    linkify: false,
    typographer: true,
  }).enable(['table', 'strikethrough']);
  md.renderer.rules.softbreak = () => '<br data-nomo-break="soft">';
  md.renderer.rules.hardbreak = () => '<br data-nomo-break="hard">';

  // 先让 markdown-it 识别链接/图片语法，再在 renderer 和 sanitizer 中按 Nomo 的安全边界过滤。
  md.validateLink = (url: string) => Boolean(url.trim());

  md.core.ruler.after('block', 'nomo_callout', (state) => {
    transformCalloutTokens(state.tokens);
  });

  md.inline.ruler.after('image', 'nomo_image_attrs', parseImageAttrs);
  md.inline.ruler.after('backticks', 'nomo_math_inline', parseMathInline);
  md.block.ruler.after('fence', 'nomo_math_display', parseMathDisplay, {
    alt: ['paragraph', 'reference', 'blockquote', 'list'],
  });

  md.renderer.rules.callout_open = (tokens, index) => {
    const token = tokens[index];
    const type = String(token.meta?.calloutType ?? 'note');
    const label = CALLOUT_LABELS[type] ?? CALLOUT_LABELS.note;
    return `<div class="callout-card" data-callout-type="${escapeHtml(type)}"><div class="callout-header"><span class="callout-icon" aria-hidden="true"></span><span class="callout-title">${label}</span></div><div class="callout-body">`;
  };
  md.renderer.rules.callout_close = () => '</div></div>';

  md.renderer.rules.math_inline = (tokens, index) => {
    return `<span class="math-inline" data-nomo-tex="${escapeHtml(tokens[index].content)}"></span>`;
  };
  md.renderer.rules.math_display = (tokens, index) => {
    return `<div class="math-block" data-nomo-tex="${escapeHtml(tokens[index].content)}"></div>`;
  };

  md.renderer.rules.image = (tokens, index, options, env, self) => {
    const token = tokens[index];
    const rawSrc = token.attrGet('src') ?? '';
    const src = resolvePreviewAssetSrc(rawSrc, env.documentDirectory);
    const alt = escapeHtml(token.content ?? token.attrGet('alt') ?? '');
    if (!src) {
      return `<span class="image-node-placeholder"><strong>${alt || '图片不可预览'}</strong><span>图片路径不可用</span></span>`;
    }

    token.attrSet('src', src);
    token.attrSet('alt', alt);
    token.attrSet('loading', 'lazy');
    if (isBadgeImageSrc(rawSrc)) {
      token.attrJoin('class', 'image-badge');
    }
    return self.renderToken(tokens, index, options);
  };

  md.renderer.rules.fence = (tokens, index) => {
    const token = tokens[index];
    const info = token.info.trim();
    const language = info.split(/\s+/)[0]?.toLowerCase() || 'text';
    const code = escapeHtml(token.content.replace(/\n$/, ''));

    if (language === 'mermaid') {
      return `<figure class="mermaid-block is-pending"><figcaption>Mermaid</figcaption><div class="mermaid-block-rendered">正在渲染图表…</div><pre class="mermaid-block-source"><code>${code}</code></pre></figure>`;
    }

    return `<figure class="code-card"><figcaption>${escapeHtml(language)}</figcaption><pre><code class="language-${escapeHtml(language)}">${code}</code></pre></figure>`;
  };

  md.renderer.rules.link_open = (tokens, index, options, _env, self) => {
    const token = tokens[index];
    const href = normalizeLinkHref(token.attrGet('href'));
    if (!href) {
      token.attrs = [];
      return '<span class="unsafe-link">';
    }
    token.attrSet('href', href);
    token.attrSet('target', '_blank');
    token.attrSet('rel', 'noreferrer noopener');
    return self.renderToken(tokens, index, options);
  };
  md.renderer.rules.link_close = (tokens, index, options, _env, self) => {
    const previousOpen = findPreviousOpenToken(tokens, index, 'link_open');
    if (previousOpen && (previousOpen.attrs?.length ?? 0) === 0) {
      return '</span>';
    }
    return self.renderToken(tokens, index, options);
  };

  return md;
}

/** 只返回经过安全过滤的 Markdown 正文，供不需要 Quick Look 外壳的只读视图复用。 */
export function renderMarkdownPreviewBody(markdown: string, options: QuickLookPreviewOptions = {}) {
  const body = markdownIt.render(markdown, {
    documentDirectory: options.documentDirectory,
  });
  const clean = sanitizePreviewHtml(renderTaskListItems(body));
  if (typeof document === 'undefined') return clean;
  const template = document.createElement('template');
  template.innerHTML = clean;
  // Render trusted KaTeX output after sanitizing user HTML: its positioning styles are required.
  template.content.querySelectorAll<HTMLElement>('[data-nomo-tex]').forEach((node) => {
    node.innerHTML = renderKatex(node.dataset.nomoTex ?? '', node.classList.contains('math-block'));
    node.removeAttribute('data-nomo-tex');
  });
  return template.innerHTML;
}

function isBadgeImageSrc(src: string): boolean {
  try {
    const url = new URL(src);
    const host = url.hostname.toLowerCase();
    return host === 'img.shields.io' || host === 'badgen.net';
  } catch {
    return false;
  }
}

function parseImageAttrs(state: QuickLookInlineState, silent: boolean) {
  const pos = state.pos;
  if (state.src.charCodeAt(pos) !== 0x7b) return false;

  const prevToken = state.tokens[state.tokens.length - 1];
  if (!prevToken || prevToken.type !== 'image') return false;

  const closeBrace = state.src.indexOf('}', pos + 1);
  if (closeBrace === -1) return false;

  const attrsStr = state.src.slice(pos + 1, closeBrace).trim();
  if (!attrsStr) return false;

  if (!silent) {
    for (const part of attrsStr.split(/\s+/)) {
      const eq = part.indexOf('=');
      if (eq <= 0) continue;
      const key = part.slice(0, eq);
      const value = part.slice(eq + 1);
      if (key === 'width') prevToken.attrSet('width', value);
      if (key === 'height') prevToken.attrSet('height', value);
      if (key === 'align') prevToken.attrJoin('class', `image-align-${value}`);
    }
  }

  state.pos = closeBrace + 1;
  return true;
}

function parseMathInline(state: QuickLookInlineState, silent: boolean) {
  const src = state.src;
  const pos = state.pos;
  if (src.charCodeAt(pos) !== 0x24) return false;
  if (src.charCodeAt(pos + 1) === 0x24) return false;
  if (pos > 0 && src.charCodeAt(pos - 1) === 0x5c) return false;

  let end = pos + 1;
  while (end < src.length) {
    if (src.charCodeAt(end) === 0x24 && src.charCodeAt(end - 1) !== 0x5c) break;
    end++;
  }
  if (end >= src.length || end === pos + 1) return false;

  const tex = src
    .slice(pos + 1, end)
    .trim()
    .replace(/\\\$/g, '$');
  if (!tex) return false;

  if (!silent) {
    const token = state.push('math_inline', '', 0);
    token.content = tex;
    token.markup = '$';
  }
  state.pos = end + 1;
  return true;
}

function parseMathDisplay(
  state: QuickLookBlockState,
  startLine: number,
  endLine: number,
  silent: boolean,
) {
  const startPos = state.bMarks[startLine] + state.tShift[startLine];
  const lineText = state.src.slice(startPos, state.eMarks[startLine]).trim();
  if (!lineText.startsWith('$$')) return false;

  const singleLineContent = lineText.slice(2);
  if (singleLineContent.endsWith('$$') && singleLineContent.length > 2) {
    const tex = singleLineContent.slice(0, -2).trim();
    if (!tex) return false;
    if (!silent) {
      const token = state.push('math_display', 'math', 0);
      token.content = tex;
      token.markup = '$$';
      token.map = [startLine, startLine + 1];
    }
    state.line = startLine + 1;
    return true;
  }

  const texLines: string[] = [];
  let closeLine = -1;
  for (let line = startLine + 1; line < endLine; line++) {
    const lineStart = state.bMarks[line] + state.tShift[line];
    const text = state.src.slice(lineStart, state.eMarks[line]).trim();
    if (text === '$$') {
      closeLine = line;
      break;
    }
    texLines.push(state.src.slice(state.bMarks[line], state.eMarks[line]));
  }

  if (closeLine === -1) return false;
  if (!silent) {
    const token = state.push('math_display', 'math', 0);
    token.content = texLines.join('\n').trim();
    token.markup = '$$';
    token.map = [startLine, closeLine + 1];
  }
  state.line = closeLine + 1;
  return true;
}

function renderKatex(tex: string, displayMode: boolean) {
  try {
    return katex.renderToString(tex, {
      displayMode,
      output: 'html',
      throwOnError: false,
      trust: false,
    });
  } catch {
    return `<code>${escapeHtml(tex)}</code>`;
  }
}

function renderTaskListItems(html: string) {
  return html.replace(
    /<li>\s*\[([ xX])\]\s*/g,
    (_match, checked: string) =>
      `<li class="task-list-item"><input class="task-checkbox" type="checkbox" disabled${checked.toLowerCase() === 'x' ? ' checked' : ''} aria-label="任务状态"> `,
  );
}

function sanitizePreviewHtml(html: string) {
  if (typeof document === 'undefined') {
    return html;
  }

  const template = document.createElement('template');
  template.innerHTML = html;
  sanitizeNode(template.content);
  return template.innerHTML;
}

function sanitizeNode(node: Node) {
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType !== Node.ELEMENT_NODE) {
      continue;
    }

    const element = child as HTMLElement;
    const tagName = element.tagName.toLowerCase();
    if (DISCARD_TAGS.has(tagName)) {
      element.remove();
      continue;
    }

    if (!ALLOWED_TAGS.has(tagName)) {
      element.replaceWith(document.createTextNode(element.textContent ?? ''));
      continue;
    }

    sanitizeElementAttributes(element, tagName);
    sanitizeNode(element);
  }
}

function sanitizeElementAttributes(element: HTMLElement, tagName: string) {
  const allowed = ATTRS_BY_TAG[tagName] ?? new Set<string>();
  for (const attr of Array.from(element.attributes)) {
    const name = attr.name.toLowerCase();
    if (!GLOBAL_ATTRS.has(name) && !allowed.has(name)) {
      element.removeAttribute(attr.name);
      continue;
    }

    if (name.startsWith('on')) {
      element.removeAttribute(attr.name);
      continue;
    }

    if (tagName === 'a' && name === 'href') {
      const href = normalizeLinkHref(attr.value);
      if (!href) {
        element.removeAttribute('href');
      } else {
        element.setAttribute('href', href);
      }
    }

    if (tagName === 'img' && name === 'src' && !resolvePreviewAssetSrc(attr.value)) {
      element.removeAttribute('src');
    }

    if ((tagName === 'td' || tagName === 'th') && name === 'style') {
      const align = /text-align\s*:\s*(left|center|right)/i.exec(attr.value)?.[1];
      if (align) {
        element.setAttribute('style', `text-align: ${align.toLowerCase()}`);
      } else {
        element.removeAttribute('style');
      }
    }
  }
}

function findPreviousOpenToken(tokens: Token[], closeIndex: number, type: string) {
  for (let index = closeIndex - 1; index >= 0; index--) {
    if (tokens[index].type === type) return tokens[index];
  }
  return null;
}

function isAbsoluteFilePath(value: string) {
  return value.startsWith('/') || /^[a-zA-Z]:[\\/]/.test(value);
}

function pathToDirectoryFileUrl(path: string) {
  const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '');
  return `${pathToFileUrl(normalized)}/`;
}

function pathToFileUrl(path: string) {
  const normalized = path.replace(/\\/g, '/');
  const withLeadingSlash = /^[a-zA-Z]:\//.test(normalized) ? `/${normalized}` : normalized;
  return encodeURI(`file://${withLeadingSlash.startsWith('/') ? '' : '/'}${withLeadingSlash}`);
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
