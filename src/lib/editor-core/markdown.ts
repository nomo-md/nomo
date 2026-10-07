import {
  createMarkdownTokenizer,
  parseHtmlImgAttrs,
  WRAPPED_IMAGE_HTML,
  STANDALONE_IMAGE_HTML,
} from './markdownTokenizer';
import { serializeMarkdown } from './markdownSerialization';
export { serializeMarkdown, serializeMarkdownSelection } from './markdownSerialization';
import { InputRule, textblockTypeInputRule, wrappingInputRule } from 'prosemirror-inputrules';
import Token from 'markdown-it/lib/token.mjs';
import { defaultMarkdownParser, MarkdownParser } from 'prosemirror-markdown';
import { type Node as ProseMirrorNode } from 'prosemirror-model';
import { schema, type TableColumnAlignment } from './schema';
import { classifyHtmlBlock } from './html/htmlClassifier';
import { parseHtmlContent } from './html/htmlToPmLogic';
import { createLinkAttrs, normalizeLinkHref } from './link';
import { calloutParserTokens } from './callout/calloutParser';
import { splitFrontMatterBlock } from '../markdown/frontMatter';
import { parseWithSyncAnchors, type MarkdownSyncAnchor } from './scrollSyncMapping';

const markdownIt = createMarkdownTokenizer();

const tableMarkdownParser = new MarkdownParser(schema, markdownIt, {
  ...defaultMarkdownParser.tokens,
  toc_block: { node: 'toc_block', getAttrs: (tok: Token) => ({ content: tok.content }) },
  link_open: {
    mark: 'link',
    getAttrs: (tok: Token) =>
      createLinkAttrs(tok.attrGet('href'), tok.attrGet('title')) ?? { href: '', title: null },
  },
  table: { block: 'table' },
  tr: { block: 'table_row' },
  th: { block: 'table_header', getAttrs: getTableCellAttrs },
  td: { block: 'table_cell', getAttrs: getTableCellAttrs },
  footnote_ref: {
    node: 'footnote_ref',
    getAttrs: (tok: Token) => ({ id: tok.meta?.id ?? tok.content }),
  },
  footnote_def: { block: 'footnote_def', getAttrs: (tok: Token) => ({ id: tok.meta?.id ?? '' }) },
  math_inline: { node: 'math_inline', getAttrs: (tok: Token) => ({ tex: tok.content }) },
  math_display: { node: 'math_block', getAttrs: (tok: Token) => ({ tex: tok.content }) },
  code_inline: { mark: 'code' },
  image: {
    node: 'image',
    getAttrs: (tok: Token) => ({
      src: tok.attrGet('src'),
      title: tok.attrGet('title') || null,
      alt: tok.children?.[0]?.content || null,
      align: tok.attrGet('align') || null,
      width: tok.attrGet('width') || null,
    }),
  },
  ...calloutParserTokens,
  s: { mark: 'strikethrough' },
  s_open: { mark: 'strikethrough' },
  s_close: { mark: 'strikethrough' },
  html_block: { ignore: true },
  html_inline: { ignore: true },
});

type HtmlMarkdownParseState = {
  openNode(type: unknown, attrs?: unknown): void;
  closeNode(): void;
  openMark(mark: unknown): void;
  closeMark(markType: unknown): void;
  addText(text: string): void;
};

type MarkdownParserWithTokenHandlers = MarkdownParser & {
  tokenHandlers: Record<string, (state: HtmlMarkdownParseState, tok: Token) => void>;
};

const tableMarkdownParserWithHandlers =
  tableMarkdownParser as unknown as MarkdownParserWithTokenHandlers;

const COMMENT_RE = /^<!--([\s\S]*?)-->$/;
const defaultFenceTokenHandler = tableMarkdownParserWithHandlers.tokenHandlers.fence;

// 覆盖 html_block token handler — 分类 HTML 后决定走可编辑节点还是 fallback paragraph
tableMarkdownParserWithHandlers.tokenHandlers = {
  ...tableMarkdownParserWithHandlers.tokenHandlers,
  softbreak: (state: HtmlMarkdownParseState) => {
    state.openNode(schema.nodes.hard_break, { soft: true });
    state.closeNode();
  },
  fence: (state: HtmlMarkdownParseState, tok: Token) => {
    const language = tok.info.trim().split(/\s+/)[0]?.toLowerCase();
    if (language === 'mermaid') {
      state.openNode(schema.nodes.mermaid_block, { code: tok.content.replace(/\n$/, '') });
      state.closeNode();
      return;
    }

    defaultFenceTokenHandler(state, tok);
  },
  html_block: (state: HtmlMarkdownParseState, tok: Token) => {
    if (isMarkdownComment(tok.content) && !isReservedTocComment(tok.content)) {
      const content = readMarkdownCommentContent(tok.content);
      if (isSingleLineMarkdownComment(tok.content)) {
        state.openNode(schema.nodes.paragraph);
        state.openNode(schema.nodes.comment_inline, { content });
        state.closeNode();
        state.closeNode();
      } else {
        state.openNode(schema.nodes.comment_block, { content });
        state.closeNode();
      }
      return;
    }

    const classification = classifyHtmlBlock(tok.content);
    if (classification.editable) {
      const attrs: Record<string, unknown> = {
        tag: classification.tag!,
        class: classification.attrs?.class ?? null,
        id: classification.attrs?.id ?? null,
      };
      state.openNode(schema.nodes.html_block, attrs);
      parseHtmlContent(state, classification.innerHTML!, schema);
      state.closeNode();
    } else {
      // 不可编辑 HTML：作为 paragraph 保留原始文本，供 tableHtmlPlugin 渲染 widget
      state.openNode(schema.nodes.paragraph);
      state.addText(tok.content.trimEnd());
      state.closeNode();
    }
  },
};

// 覆盖 html_inline token handler — 映射内联 HTML 到已有 mark
// html:true 后，段落内的 <strong> 等标签会产生 html_inline token，
// 不与 markdown 语法 ** 冲突（后者走 strong_open/close 通道）

/** 内联标签到 ProseMirror mark 类型的映射 */
const INLINE_MARK_MAP: Record<string, string> = {
  strong: 'strong',
  b: 'strong',
  em: 'em',
  i: 'em',
  code: 'code',
  a: 'link',
  s: 'strikethrough',
  del: 'strikethrough',
  strike: 'strikethrough',
  u: 'underline',
  mark: 'highlight',
};

const htmlInlineStack: Array<{ tag: string; markName: string }> = [];

function resetHtmlInlineStack(): void {
  htmlInlineStack.length = 0;
}

tableMarkdownParserWithHandlers.tokenHandlers.html_inline = (
  state: HtmlMarkdownParseState,
  tok: Token,
) => {
  const content = tok.content;

  if (isMarkdownComment(content) && !isReservedTocComment(content)) {
    state.openNode(schema.nodes.comment_inline, {
      content: readMarkdownCommentContent(content),
    });
    state.closeNode();
    return;
  }

  const tagMatch = /^<\/?([a-zA-Z][a-zA-Z0-9]*)/.exec(content);
  if (!tagMatch) {
    // 注释、PI 等 — 原样输出
    state.addText(content);
    return;
  }

  const tag = tagMatch[1].toLowerCase();
  const isClosing = content.startsWith('</');
  const isSelfClosing = /\/>$/.test(content);

  if (isSelfClosing) {
    if (tag === 'br') {
      state.addText('\n');
    } else {
      state.addText(content);
    }
    return;
  }

  const markName = INLINE_MARK_MAP[tag];

  if (isClosing) {
    // 在栈中查找匹配的开标签
    const idx = findLastIndex(htmlInlineStack, (e) => e.tag === tag);
    if (idx >= 0) {
      // 先关闭后面开的标签
      while (htmlInlineStack.length > idx) {
        const top = htmlInlineStack.pop()!;
        state.closeMark(schema.marks[top.markName]);
      }
    } else {
      // 无匹配开标签 — 原样输出
      state.addText(content);
    }
    return;
  }

  // 开标签
  if (markName) {
    const markType = schema.marks[markName];
    let attrs: Record<string, unknown> | null = null;
    if (markName === 'link') {
      const linkAttrs = createLinkAttrs(
        extractAttr(content, 'href'),
        extractAttr(content, 'title'),
      );
      attrs = linkAttrs ? { ...linkAttrs } : null;
      if (!attrs) {
        state.addText(content);
        return;
      }
    }
    const mark = markType.create(attrs);
    htmlInlineStack.push({ tag, markName });
    state.openMark(mark);
  } else {
    // 不支持的内联标签（如 span）— 保留原始 HTML 文本
    state.addText(content);
  }
};

// 覆盖 image token handler — 从 tok.attrs 读取 align/width 写入 node
// 需要在 tableMarkdownParserWithHandlers 定义之后执行
const defaultImageTokenHandler = tableMarkdownParserWithHandlers.tokenHandlers.image;
tableMarkdownParserWithHandlers.tokenHandlers.image = (state, tok) => {
  if (defaultImageTokenHandler) {
    // image_attrs 行内规则已将 align/width 写入 tok.attrs，
    // 这里确保它们作为 ProseMirror node attrs 传递
    defaultImageTokenHandler(state, tok);
  }
};

/**
 * 预处理：将 GitHub 兼容的 HTML 图片格式转换为旧 {align=center width=128} 格式，
 * 让现有 parser 统一处理。
 *   - <p align="left|center|right"><img ...></p>  → ![alt](src){align=X width=Y}
 *   - 独立一行的 <img src="..." ...>             → ![alt](src){width=Y}
 */
interface MarkdownLineProvenance {
  fromLine: number;
  toLine: number;
}

interface PreprocessedImageHtml {
  markdown: string;
  lineProvenance: MarkdownLineProvenance[];
}

/**
 * 图片 HTML 可能把多行源码折叠成一行 Markdown。解析器需要折叠后的文本，
 * 但块对齐必须继续使用原始源码行号，因此同时记录每一行的来源范围。
 */
function preprocessImageHtmlWithProvenance(markdown: string): PreprocessedImageHtml {
  const collapsedRanges: Array<MarkdownLineProvenance & { transformedLine: number }> = [];
  let removedLineCount = 0;

  // 步骤1：<p align="..."><img ...></p>（单行或多行）
  let result = markdown.replace(
    WRAPPED_IMAGE_HTML,
    (_full: string, align: string, imgTag: string, offset: number) => {
      const cleaned = imgTag.replace(/\/>$/, '').replace(/>$/, '').trim();
      const attrs = parseHtmlImgAttrs(cleaned);
      if (!attrs.src) return _full;
      const parts: string[] = [`align=${align.toLowerCase()}`];
      if (attrs.width) parts.push(`width=${attrs.width}`);
      const titleStr = attrs.title ? ` "${attrs.title}"` : '';
      const replacement = `![${attrs.alt || ''}](${attrs.src}${titleStr}){${parts.join(' ')}}`;
      const originalFromLine = getLineNumberAtOffset(markdown, offset);
      const originalToLine = getLineNumberAtOffset(markdown, offset + _full.length - 1);
      collapsedRanges.push({
        transformedLine: originalFromLine - removedLineCount,
        fromLine: originalFromLine,
        toLine: originalToLine,
      });
      removedLineCount += originalToLine - originalFromLine;
      return replacement;
    },
  );

  const wrappedProvenance = Array.from(
    { length: countMarkdownLines(result) },
    (_value, index): MarkdownLineProvenance => {
      const transformedLine = index + 1;
      const collapsedRange = collapsedRanges.find((range) => range.transformedLine === transformedLine);
      if (collapsedRange) return { fromLine: collapsedRange.fromLine, toLine: collapsedRange.toLine };
      const removedBeforeLine = collapsedRanges.reduce(
        (total, range) => range.transformedLine < transformedLine ? total + range.toLine - range.fromLine : total, 0,
      );
      const originalLine = transformedLine + removedBeforeLine;
      return { fromLine: originalLine, toLine: originalLine };
    },
  );
  const wrappedMarkdown = result;
  const standaloneRanges: Array<{ line: number; from: number; to: number }> = [];
  let standaloneRemoved = 0;
  // 步骤2：保留原有图片转换行为，同时记录被空白匹配吞并的行。
  result = result.replace(
    STANDALONE_IMAGE_HTML,
    (imgTag: string, offset: number) => {
      const cleaned = imgTag.replace(/\/>$/, '').replace(/>$/, '').trim();
      if (/<[^>]+<[^>]+>/.test(cleaned)) return imgTag; // 含嵌套标签，不处理
      const attrs = parseHtmlImgAttrs(cleaned);
      if (!attrs.src) return imgTag;
      const parts: string[] = [];
      if (attrs.width) parts.push(`width=${attrs.width}`);
      const titleStr = attrs.title ? ` "${attrs.title}"` : '';
      const attrsStr = parts.length > 0 ? `{${parts.join(' ')}}` : '';
      const from = getLineNumberAtOffset(wrappedMarkdown, offset);
      const to = getLineNumberAtOffset(wrappedMarkdown, offset + imgTag.length);
      standaloneRanges.push({ line: from - standaloneRemoved, from, to });
      standaloneRemoved += to - from;
      return `![${attrs.alt || ''}](${attrs.src}${titleStr})${attrsStr}`;
    },
  );

  let removedBeforeLine = 0;
  let rangeIndex = 0;
  const lineProvenance = Array.from(
    { length: countMarkdownLines(result) },
    (_value, index): MarkdownLineProvenance => {
      const transformedLine = index + 1;
      const range = standaloneRanges[rangeIndex];
      if (range?.line === transformedLine) {
        rangeIndex += 1;
        removedBeforeLine += range.to - range.from;
        return { fromLine: wrappedProvenance[range.from - 1].fromLine,
          toLine: wrappedProvenance[range.to - 1].toLine };
      }
      return wrappedProvenance[index + removedBeforeLine];
    },
  );

  return { markdown: result, lineProvenance };
}

function preprocessImageHtml(markdown: string): string {
  return preprocessImageHtmlWithProvenance(markdown).markdown;
}

function getLineNumberAtOffset(value: string, offset: number): number {
  let line = 1;
  const end = Math.max(0, Math.min(offset, value.length));
  for (let index = 0; index < end; index += 1) {
    if (value.charCodeAt(index) === 10) line += 1;
  }
  return line;
}

function countMarkdownLines(value: string): number {
  return value.split(/\r?\n/).length;
}

export function parseMarkdown(markdown: string): ProseMirrorNode {
  resetHtmlInlineStack();
  const { frontMatterPrefix, body: rawBody } = splitMarkdownDocument(markdown);
  const preprocessed = preprocessImageHtml(rawBody);
  try {
    const parsed = tableMarkdownParser.parse(preprocessed);
    return parsed.type.create({ ...parsed.attrs, frontMatterPrefix }, parsed.content, parsed.marks);
  } catch {
    resetHtmlInlineStack();
    return schema.node('doc', { frontMatterPrefix }, [
      schema.node('paragraph', null, rawBody ? [schema.text(rawBody)] : undefined),
    ]);
  }
}

export interface MarkdownBlockLineMap {
  fromLine: number;
  toLine: number;
  nodeIndex: number;
}

/** 返回与实际解析节点绑定的嵌套来源映射；解析失败时不提供猜测锚点。 */
export function parseMarkdownWithSyncAnchors(markdown: string): {
  doc: ProseMirrorNode;
  anchors: MarkdownSyncAnchor[];
} {
  resetHtmlInlineStack();
  const { frontMatterPrefix, body } = splitMarkdownDocument(markdown);
  const preprocessed = preprocessImageHtmlWithProvenance(body);
  const bodyOffset = markdown.length - body.length;
  const lineOffset = markdown.slice(0, bodyOffset).split('\n').length - 1;
  try {
    const result = parseWithSyncAnchors(tableMarkdownParser, preprocessed.markdown, (line, end) => {
      const provenance = preprocessed.lineProvenance[line];
      return provenance ? (end ? provenance.toLine : provenance.fromLine) + lineOffset : undefined;
    });
    return {
      doc: result.doc.type.create({ ...result.doc.attrs, frontMatterPrefix }, result.doc.content, result.doc.marks),
      anchors: result.anchors,
    };
  } catch {
    return { doc: parseMarkdown(markdown), anchors: [] };
  }
}

/**
 * 使用与语义编辑器相同的 markdown-it 配置建立源码行到顶层文档块的临时映射。
 * 该映射只用于导航，不参与文档解析或持久化。
 */
export function getMarkdownBlockLineMap(markdown: string): MarkdownBlockLineMap[] {
  const { frontMatter, body } = splitFrontMatter(markdown);
  const bodyOffset = body ? markdown.indexOf(body, frontMatter.length) : markdown.length;
  const bodyStartLineOffset =
    (bodyOffset >= 0 ? markdown.slice(0, bodyOffset) : '').split('\n').length - 1;
  const preprocessed = preprocessImageHtmlWithProvenance(body);
  const tokens = markdownIt.parse(preprocessed.markdown, {});

  return tokens
    .filter((token) => token.level === 0 && token.nesting >= 0 && token.map)
    .map((token, nodeIndex): MarkdownBlockLineMap | null => {
      const [fromLineIndex, toLineIndex] = token.map!;
      const fromProvenance = preprocessed.lineProvenance[fromLineIndex];
      const toProvenance = preprocessed.lineProvenance[toLineIndex - 1];
      if (!fromProvenance || !toProvenance) return null;
      return {
        fromLine: fromProvenance.fromLine + bodyStartLineOffset,
        toLine: toProvenance.toLine + bodyStartLineOffset,
        nodeIndex,
      };
    })
    .filter((mapping): mapping is MarkdownBlockLineMap => mapping !== null);
}

export function splitFrontMatter(markdown: string): { frontMatter: string; body: string } {
  return splitFrontMatterBlock(markdown);
}

function splitMarkdownDocument(markdown: string): { frontMatterPrefix: string; body: string } {
  const { frontMatter, body } = splitFrontMatterBlock(markdown);
  if (!frontMatter) {
    return { frontMatterPrefix: '', body };
  }

  const suffix = markdown.slice(frontMatter.length);
  const bodyOffset = body ? suffix.indexOf(body) : suffix.length;
  const separator = bodyOffset >= 0 ? suffix.slice(0, bodyOffset) : '';
  return { frontMatterPrefix: `${frontMatter}${separator}`, body };
}


function createTableMarkdown(rows: number, columns: number): string {
  const columnCount = Math.max(2, Math.min(columns, 6));
  const rowCount = Math.max(1, Math.min(rows, 8));
  const headers = Array.from({ length: columnCount }, () => '');
  const separator = Array.from({ length: columnCount }, () => '---');
  const body = Array.from({ length: rowCount }, () => headers.map(() => ''));
  const lines = [headers, separator, ...body].map((cells) => `| ${cells.join(' | ')} |`);
  return `${lines.join('\n')}\n`;
}

function getTableCellAttrs(token: { attrGet(name: string): string | null }): {
  align: TableColumnAlignment | null;
} {
  const style = token.attrGet('style') ?? '';
  const match = /text-align\s*:\s*(left|center|right)/i.exec(style);
  return { align: match ? (match[1].toLowerCase() as TableColumnAlignment) : null };
}

export function createMarkdownInputRules() {
  return [
    createMathInlineInputRule(),
    textblockTypeInputRule(/^(#{1,6})\s$/, schema.nodes.heading, (match) => ({
      level: match[1].length,
    })),
    wrappingInputRule(/^\s*([-+*])\s$/, schema.nodes.bullet_list),
    wrappingInputRule(/^(\d+)\.\s$/, schema.nodes.ordered_list, (match) => ({
      order: Number(match[1]),
    })),
    textblockTypeInputRule(/^```$/, schema.nodes.code_block),
    createHorizontalRuleInputRule(),
  ];
}

function createMathInlineInputRule(): InputRule {
  return new InputRule(/(?:^|[^\\$])\$\s*([^$]*?\S[^$]*?)\s*\$$/, (state, match, start, end) => {
    const fullMatch = match[0];
    const tex = match[1]?.trim().replace(/\\\$/g, '$') ?? '';
    if (!tex.trim()) {
      return null;
    }

    // 步骤1：保留正则前导字符（如果有），只替换用户刚闭合的 $tex$ 片段。
    const hasLeadingChar = !fullMatch.startsWith('$');
    const mathStart = hasLeadingChar ? start + 1 : start;
    const node = schema.nodes.math_inline.create({ tex });

    // 步骤2：用语义公式节点替换源码标记，并把光标放到公式后面继续写正文。
    return state.tr.replaceWith(mathStart, end, node);
  });
}

/**
 * 输入 `---` 或 `___` 时转为水平分割线。
 *
 * 不用 `***`：它和加粗/斜体语法冲突，打 `****` 准备包一层加粗时，
 * 第三个 `*` 就会被吃成分割线。分割线请用短横线，或从菜单插入。
 */
function createHorizontalRuleInputRule(): InputRule {
  return new InputRule(/^(-{3,}|_{3,})$/, (state, _match, start, end) => {
    const hrNode = schema.nodes.horizontal_rule.create();
    const emptyParagraph = schema.nodes.paragraph.create();
    return state.tr.replaceWith(start, end, [hrNode, emptyParagraph]);
  });
}

function findLastIndex<T>(arr: T[], predicate: (value: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) return i;
  }
  return -1;
}

function extractAttr(rawTag: string, name: string): string | null {
  const regex = new RegExp(`${name}="([^"]*)"`, 'i');
  const match = regex.exec(rawTag);
  return match ? match[1] : null;
}

function isMarkdownComment(content: string): boolean {
  const match = COMMENT_RE.exec(content.trim());
  if (!match) {
    return false;
  }

  const commentContent = match[1];
  return !commentContent.includes('<!--') && !commentContent.includes('-->');
}

function isReservedTocComment(content: string): boolean {
  const trimmed = content.trim();
  return /^<!--\s*toc\s*-->\s*$/i.test(trimmed) || /^<!--\s*\/toc\s*-->\s*$/i.test(trimmed);
}

function isSingleLineMarkdownComment(content: string): boolean {
  return !content.trim().includes('\n');
}

function readMarkdownCommentContent(rawComment: string): string {
  const match = COMMENT_RE.exec(rawComment.trim());
  if (!match) return rawComment;

  const content = match[1].replace(/\r\n/g, '\n');
  if (/^\n[\s\S]*\n$/.test(content)) {
    return content.slice(1, -1);
  }

  return content.replace(/^[ \t]?/, '').replace(/[ \t]?$/, '');
}
