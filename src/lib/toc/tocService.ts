import { Fragment, type Node as ProseMirrorNode } from 'prosemirror-model';
import type { OutlineItem } from '../outline/outlineService';
import { analyzeInlineSourceText, escapeInlineSourceLiteral } from '../editor-core/InlineSourceCodec';
import { createMarkdownTokenizer, inlineTokenSourceRange } from '../editor-core/markdownTokenizer';
import { serializeMarkdown } from '../editor-core/markdownSerialization';
import { tokenizeMarkdownWithSourceLines } from '../editor-core/markdown';
import type Token from 'markdown-it/lib/token.mjs';
import { TOC_START_MARKER, TOC_END_MARKER } from './tocMarkers';
export { TOC_START_MARKER, TOC_END_MARKER } from './tocMarkers';

const headingTokenizer = createMarkdownTokenizer({ inlineSource: true });
const documentTocItems = new WeakMap<ProseMirrorNode, readonly DocumentTocItem[]>();
const headingTitles = new WeakMap<ProseMirrorNode, string>();

export interface DocumentTocItem {
  id: string;
  level: OutlineItem['level'];
  title: string;
  pos: number;
}

export interface TocBlockRange {
  start: number;
  end: number;
  contentStart: number;
  contentEnd: number;
}

interface MarkdownLine {
  text: string;
  start: number;
  contentEnd: number;
}

export function createTocBlock(markdown: string): string {
  const content = createTocList(markdown);
  return `${TOC_START_MARKER}\n${content}${content ? '\n' : ''}${TOC_END_MARKER}`;
}

export function updateTocBlocks(markdown: string): string {
  if (!markdown.includes(TOC_START_MARKER) || !markdown.includes(TOC_END_MARKER)) {
    return markdown;
  }

  const ranges = extractTocRanges(markdown);
  if (ranges.length === 0) {
    return markdown;
  }

  const content = createTocList(markdown);
  const replacement = `${TOC_START_MARKER}\n${content}${content ? '\n' : ''}${TOC_END_MARKER}`;
  let updated = markdown;
  for (let index = ranges.length - 1; index >= 0; index--) {
    const range = ranges[index];
    updated = `${updated.slice(0, range.start)}${replacement}${updated.slice(range.end)}`;
  }
  return updated;
}

export function hasTocBlock(markdown: string): boolean {
  return extractTocRanges(markdown).length > 0;
}

export function removeTocBlocks(markdown: string): string {
  const ranges = extractTocRanges(markdown);
  let updated = markdown;
  for (let index = ranges.length - 1; index >= 0; index--) {
    const range = ranges[index];
    updated = `${updated.slice(0, range.start)}${updated.slice(range.end)}`;
  }
  return updated;
}

export function extractTocRanges(markdown: string): TocBlockRange[] {
  const ranges: TocBlockRange[] = [];
  let startLine: MarkdownLine | null = null;
  let inFence = false;
  let fenceMarker = '';

  for (const line of readMarkdownLines(markdown)) {
    const fenceMatch = /^(\s*)(`{3,}|~{3,})/.exec(line.text);
    if (fenceMatch) {
      const marker = fenceMatch[2];
      if (!inFence) {
        inFence = true;
        fenceMarker = marker[0];
      } else if (marker[0] === fenceMarker) {
        inFence = false;
        fenceMarker = '';
      }
      continue;
    }

    if (inFence) {
      continue;
    }

    const trimmed = line.text.trim();
    if (trimmed === TOC_START_MARKER) {
      startLine = line;
      continue;
    }

    if (trimmed === TOC_END_MARKER && startLine) {
      ranges.push({
        start: startLine.start,
        end: line.contentEnd,
        contentStart: startLine.contentEnd,
        contentEnd: line.start,
      });
      startLine = null;
    }
  }

  return ranges;
}

export function createTocList(markdown: string): string {
  return serializeTocItems(extractTocItems(markdown));
}

export function createDocumentTocList(doc: ProseMirrorNode): string {
  return serializeTocItems(extractDocumentTocItems(doc));
}

export function serializeTocItems(items: readonly Pick<OutlineItem, 'id' | 'level' | 'title'>[]): string {
  return items
    .map((item) => `${'  '.repeat(item.level - 1)}- [${escapeLinkText(item.title)}](#${item.id})`)
    .join('\n');
}

/** 源码目录与大纲复用真实 Markdown 标题 token，不把 HTML 块内部标签当作 Markdown 标题。 */
export function extractTocItems(markdown: string): OutlineItem[] {
  const { tokens, sourceLine } = tokenizeMarkdownWithSourceLines(markdown);
  const allocateId = createHeadingIdAllocator();
  const items: OutlineItem[] = [];
  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if (token.type !== 'heading_open' || tokens[index + 1]?.type !== 'inline') continue;
    const rawTitle = tokens[index + 1].content;
    const title = normalizeHeadingTitle(rawTitle) || rawTitle.trim();
    items.push({
      id: allocateId(title, items.length + 1),
      level: Number(token.tag.slice(1)) as OutlineItem['level'],
      title,
      line: sourceLine(token.map?.[0] ?? 0),
    });
  }
  return items;
}

/** 目录同步、标题 DOM 属性和点击定位共享同一份文档标题位置与 ID。 */
export function extractDocumentTocItems(doc: ProseMirrorNode): readonly DocumentTocItem[] {
  const cached = documentTocItems.get(doc);
  if (cached) return cached;
  const allocateId = createHeadingIdAllocator();
  const items: DocumentTocItem[] = [];
  doc.descendants((node, pos) => {
    if (node.type.name === 'html_block') return false;
    if (node.type.name !== 'heading') return true;
    let title = headingTitles.get(node);
    if (title === undefined) {
      // 标题投影只保留图片替代文字和换行，不让图片块对齐包装污染标题文本。
      const inline: ProseMirrorNode[] = [];
      node.forEach((child) => {
        if (child.type.name === 'hard_break') {
          inline.push(node.type.schema.text('\n', [node.type.schema.marks.inline_source.create()]));
        } else if (child.type.name === 'image') {
          inline.push(child.type.create({ ...child.attrs, align: null, width: null }, child.content, child.marks));
        } else {
          inline.push(child);
        }
      });
      const rawTitle = serializeMarkdown(node.type.schema.nodes.doc.create(null, [node.copy(Fragment.fromArray(inline))]))
        .replace(/^#{1,6}[ \t]*/, '').trim();
      title = normalizeHeadingTitle(rawTitle) || rawTitle;
      headingTitles.set(node, title);
    }
    items.push({
      id: allocateId(title, items.length + 1),
      level: node.attrs.level as OutlineItem['level'],
      title,
      pos,
    });
    return false;
  });
  documentTocItems.set(doc, items);
  return items;
}

/** 最终 ID 全局唯一；空 slug 按同一标题序号兜底，显式后缀也参与去重。 */
export function createHeadingIdAllocator(): (title: string, ordinal: number) => string {
  const usedIds = new Set<string>();
  const nextSuffixes = new Map<string, number>();
  return (title, ordinal) => {
    const baseId = slugifyHeading(title) || `heading-${ordinal}`;
    let id = baseId;
    let suffix = nextSuffixes.get(baseId) ?? 2;
    while (usedIds.has(id)) id = `${baseId}-${suffix++}`;
    nextSuffixes.set(baseId, suffix);
    usedIds.add(id);
    return id;
  };
}

function readMarkdownLines(markdown: string): MarkdownLine[] {
  const lines: MarkdownLine[] = [];
  let start = 0;

  while (start < markdown.length) {
    const newlineIndex = markdown.indexOf('\n', start);
    if (newlineIndex === -1) {
      lines.push({
        text: markdown.slice(start),
        start,
        contentEnd: markdown.length,
      });
      break;
    }

    const contentEnd =
      newlineIndex > start && markdown[newlineIndex - 1] === '\r' ? newlineIndex - 1 : newlineIndex;
    lines.push({
      text: markdown.slice(start, contentEnd),
      start,
      contentEnd,
    });
    start = newlineIndex + 1;
  }

  return lines;
}

export function slugifyHeading(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}\s-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
}

function escapeLinkText(text: string): string {
  return escapeInlineSourceLiteral(text);
}

export function normalizeHeadingTitle(title: string): string {
  const analysis = analyzeInlineSourceText(title);
  const tokens = headingTokenizer.parseInline(title, {})[0]?.children ?? [];
  const hidden = new Set<number>();
  const images = new Map<number, { to: number; text: string }>();
  for (const token of tokens) {
    const range = inlineTokenSourceRange(token);
    if (!range) continue;
    // A link-looking string inside a code span is still literal code content.
    if (analysis.spans.some((span) => span.type === 'code'
      && range.from >= span.from && range.to <= span.to)) continue;
    if (token.type === 'html_inline' && /^<br(?:\s[^>]*)?\/?>$/i.test(token.content)) {
      for (let pos = range.from; pos < range.to; pos++) hidden.add(pos);
      images.set(range.from, { to: range.to, text: ' ' });
    }
    if (token.type === 'link_open' || token.type === 'link_close' || token.type === 'image') {
      for (let pos = range.from; pos < range.to; pos++) hidden.add(pos);
      if (token.type === 'image') {
        images.set(range.from, {
          to: range.to,
          text: token.children
            ? token.children.map(readImageTitleText).join('')
            : token.attrGet('alt') ?? normalizeHeadingTitle(token.content || ''),
        });
      }
    }
  }
  let plain = '';
  const emittedImages = new Set<number>();
  for (let index = 0; index < analysis.visibleText.length; index++) {
    const original = analysis.visiblePositions[index];
    for (const [from, image] of images) {
      if (!emittedImages.has(from) && original >= from) {
        plain += image.text;
        emittedImages.add(from);
      }
    }
    if (!hidden.has(original)) plain += analysis.visibleText[index];
  }
  return plain.replace(/\r\n|\r|\n/g, ' ').trim();
}

/** 图片替代文本采用 parser 的可见 token 内容，公式只包含 tex，代码内符号保持字面值。 */
function readImageTitleText(token: Token): string {
  if (token.children) return token.children.map(readImageTitleText).join('');
  if (token.type === 'softbreak' || token.type === 'hardbreak') return '\n';
  return token.nesting === 0 ? token.content : '';
}
