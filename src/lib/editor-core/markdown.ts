import {
  createMarkdownTokenizer,
  parseHtmlImgAttrs,
  parseHtmlAttributes,
  WRAPPED_IMAGE_HTML,
  STANDALONE_IMAGE_HTML,
  inlineTokenSourceRange,
} from './markdownTokenizer';
import {
  analyzeInlineSource,
  escapeInlineSourceLiteral,
  isInlineSourceBlock,
  isRetainedInlineToken,
  materializeInlineSource,
} from './InlineSourceCodec';
import { serializeMarkdown } from './markdownSerialization';
export { serializeMarkdown, serializeMarkdownSelection } from './markdownSerialization';
import { InputRule, textblockTypeInputRule } from 'prosemirror-inputrules';
import { createListInputRule } from './listCommands';
import Token from 'markdown-it/lib/token.mjs';
import { defaultMarkdownParser, MarkdownParser } from 'prosemirror-markdown';
import { DOMParser as PmDOMParser, Fragment, type Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import { schema, type TableColumnAlignment } from './schema';
import { classifyHtmlBlock, readHtmlInlineAtom, readHtmlTag } from './html/htmlClassifier';
import { TableMap } from 'prosemirror-tables';
import { parseHtmlContent } from './html/htmlToPmLogic';
import { parseHtmlAnchorAttrs, createHtmlAnchorAttrs } from './html/htmlAnchor';
import { createLinkAttrs, normalizeLinkHref, serializeMarkdownLinkDestination, serializeMarkdownLinkTitle } from './link';
import { calloutParserTokens } from './callout/calloutParser';
import { parseDetailsHtml } from './details/detailsSchema';
import { splitFrontMatterBlock } from '../markdown/frontMatter';
import { parseWithSyncAnchors, type MarkdownSyncAnchor } from './scrollSyncMapping';

const markdownIt = createMarkdownTokenizer({ inlineSource: true });

const tableMarkdownParser = new MarkdownParser(schema, markdownIt, {
  ...defaultMarkdownParser.tokens,
  ordered_list: {
    ...defaultMarkdownParser.tokens.ordered_list,
    getAttrs: (tok, tokens, index) => ({
      ...defaultMarkdownParser.tokens.ordered_list.getAttrs!(tok, tokens, index),
      order: Number(tok.attrGet('start') ?? 1),
    }),
  },
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
      alt: tok.children ? tok.children.map(readImageAltText).join('') : tok.attrGet('alt') ?? '',
      align: tok.attrGet('align') || null,
      width: tok.attrGet('width') || null,
    }),
  },
  ...calloutParserTokens,
  details: { block: 'details', getAttrs: tok => ({ defaultOpen: tok.attrGet('open') !== null }) },
  details_body: { block: 'details_body' },
  details_summary_html: { ignore: true },
  details_html_body: { ignore: true },
  superscript: { mark: 'superscript' },
  subscript: { mark: 'subscript' },
  s: { mark: 'strikethrough' },
  s_open: { mark: 'strikethrough' },
  s_close: { mark: 'strikethrough' },
  html_block: { ignore: true },
  html_inline: { ignore: true },
});

/** 图片替代文字保留全部显示内容，格式开闭token不计入文字。 */
function readImageAltText(token: Token): string {
  if (token.children) return token.children.map(readImageAltText).join('');
  if (token.type === 'softbreak' || token.type === 'hardbreak') return '\n';
  return token.nesting === 0 ? token.content : '';
}

type HtmlMarkdownParseState = {
  openNode(type: unknown, attrs?: unknown): void;
  addNode(type: unknown, attrs?: unknown, content?: Fragment): void;
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
  details_summary_html: (state: HtmlMarkdownParseState, tok: Token) => {
    const root = document.createElement('div');
    const paragraph = document.createElement('p');
    paragraph.innerHTML = tok.content;
    root.append(paragraph);
    const inline = PmDOMParser.fromSchema(schema).parse(root, { preserveWhitespace: true }).firstChild?.content ?? Fragment.empty;
    const summary = materializeInlineSource(schema.nodes.details_summary.create(null, inline));
    state.addNode(summary.type, summary.attrs, summary.content);
  },
  details_html_body: (state: HtmlMarkdownParseState, tok: Token) => {
    const root = document.createElement('div');
    root.innerHTML = tok.content;
    parseDetailsHtml(root, schema).forEach(node => {
      const content = materializeInlineSource(node);
      state.addNode(content.type, content.attrs, content.content);
    });
  },
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
    const table = parseSemanticTableHtml(tok.content);
    if (table) {
      state.addNode(table.type, table.attrs, table.content);
      return;
    }
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
      state.openMark(schema.marks.inline_source.create({ literal: true }));
      state.addText(tok.content.trimEnd());
      state.closeMark(schema.marks.inline_source);
      state.closeNode();
    }
  },
};

interface SemanticTableHtmlElement {
  tag: string;
  attrs: Record<string, string>;
  children: Array<string | SemanticTableHtmlElement>;
}

/** 只回读生成器的完整 HTML 表格；未知结构/属性保留原文，不静默扁平化。 */
function parseSemanticTableHtml(raw: string): ProseMirrorNode | null {
  const html = raw.trim();
  if (!/^<table\s/i.test(html)) return null;
  let offset = 0;
  const decode = (text: string) => markdownIt.utils.unescapeAll(text.replace(/\\/g, '\\\\'));
  const readElement = (): SemanticTableHtmlElement | null => {
    const opening = readHtmlTag(html, offset);
    if (!opening || opening.closing) return null;
    offset = opening.end;
    const element: SemanticTableHtmlElement = { tag: opening.tag, attrs: opening.attrs, children: [] };
    if (opening.tag === 'br' || opening.tag === 'img') return element;
    if (opening.selfClosing) return null;
    while (offset < html.length) {
      if (html[offset] !== '<') {
        const end = html.indexOf('<', offset);
        if (end < 0) return null;
        element.children.push(decode(html.slice(offset, end)));
        offset = end;
        continue;
      }
      const tag = readHtmlTag(html, offset);
      if (!tag) return null;
      if (tag.closing) {
        if (tag.tag !== opening.tag) return null;
        offset = tag.end;
        return element;
      }
      const child = readElement();
      if (!child) return null;
      element.children.push(child);
    }
    return null;
  };
  const onlyAttrs = (element: SemanticTableHtmlElement, names: string[]) =>
    Object.keys(element.attrs).every((name) => names.includes(name));
  const elements = (element: SemanticTableHtmlElement): SemanticTableHtmlElement[] | null => {
    if (element.children.some((child) => typeof child === 'string' && child.trim() !== '')) return null;
    return element.children.filter((child): child is SemanticTableHtmlElement => typeof child !== 'string');
  };
  const inline = (children: SemanticTableHtmlElement['children'], marks: readonly import('prosemirror-model').Mark[] = []): ProseMirrorNode[] | null => {
    const nodes: ProseMirrorNode[] = [];
    for (const child of children) {
      if (typeof child === 'string') {
        if (child) nodes.push(schema.text(child, marks));
        continue;
      }
      const atom = readHtmlInlineAtom(child.tag, child.attrs);
      if (atom) {
        const text = child.children.every((entry) => typeof entry === 'string') ? child.children.join('') : null;
        const expected = atom.type === 'math_inline' ? '$' + atom.attrs.tex + '$' :
          atom.type === 'comment_inline' ? atom.attrs.content : atom.type === 'footnote_ref' ? atom.attrs.id : '';
        if (text !== expected) return null;
        nodes.push(schema.nodes[atom.type].createChecked(atom.attrs, null, marks));
        continue;
      }
      if (child.tag === 'a') {
        if (!onlyAttrs(child, ['href', 'title', 'id', 'name'])) return null;
        const attrs = createHtmlAnchorAttrs(child.attrs.href ?? null, child.attrs.title ?? null, child.attrs.id ?? null, child.attrs.name ?? null);
        if (!attrs) return null;
        if (!child.children.length) {
          nodes.push(schema.nodes.html_anchor.createChecked(attrs, null, marks));
          continue;
        }
        const linkAttrs = child.attrs.id !== undefined || child.attrs.name !== undefined ? attrs :
          createLinkAttrs(child.attrs.href, child.attrs.title);
        if (!linkAttrs) return null;
        const nested = inline(child.children, schema.marks.link.create(linkAttrs).addToSet(marks));
        if (!nested) return null;
        nodes.push(...nested);
        continue;
      }
      const markType = schema.marks[INLINE_MARK_MAP[child.tag]];
      if (!markType || !onlyAttrs(child, [])) return null;
      const nested = inline(child.children, markType.create().addToSet(marks));
      if (!nested) return null;
      nodes.push(...nested);
    }
    return nodes;
  };
  try {
    const table = readElement();
    if (!table || offset !== html.length || table.tag !== 'table' ||
        table.attrs['data-nomo-table'] !== 'semantic' || !onlyAttrs(table, ['data-nomo-table'])) return null;
    const bodies = elements(table);
    if (!bodies || bodies.length !== 1 || bodies[0].tag !== 'tbody' || !onlyAttrs(bodies[0], [])) return null;
    const rows = elements(bodies[0]);
    if (!rows?.length) return null;
    const rowNodes: ProseMirrorNode[] = [];
    for (const row of rows) {
      if (row.tag !== 'tr' || !onlyAttrs(row, [])) return null;
      const cells = elements(row);
      if (!cells) return null;
      const cellNodes: ProseMirrorNode[] = [];
      for (const cell of cells) {
        if (!['td', 'th'].includes(cell.tag) || !onlyAttrs(cell, ['colspan', 'rowspan', 'data-colwidth', 'data-align'])) return null;
        const colspan = Number(cell.attrs.colspan ?? 1), rowspan = Number(cell.attrs.rowspan ?? 1);
        if (!Number.isSafeInteger(colspan) || colspan < 1 || !Number.isSafeInteger(rowspan) || rowspan < 1) return null;
        const colwidth = cell.attrs['data-colwidth'] === undefined ? null : cell.attrs['data-colwidth'].split(',').map(Number);
        if (colwidth && (colwidth.length !== colspan || colwidth.some((width) => !Number.isSafeInteger(width) || width < 0))) return null;
        const align = cell.attrs['data-align'] ?? null;
        if (align !== null && !['left', 'center', 'right'].includes(align)) return null;
        const paragraphs = elements(cell);
        if (!paragraphs?.length) return null;
        const paragraphNodes: ProseMirrorNode[] = [];
        for (const paragraph of paragraphs) {
          if (paragraph.tag !== 'p' || !onlyAttrs(paragraph, [])) return null;
          const content = inline(paragraph.children);
          if (!content) return null;
          paragraphNodes.push(schema.nodes.paragraph.createChecked(null, content));
        }
        cellNodes.push((cell.tag === 'th' ? schema.nodes.table_header : schema.nodes.table_cell)
          .createChecked({ colspan, rowspan, colwidth, align }, paragraphNodes));
      }
      rowNodes.push(schema.nodes.table_row.createChecked(null, cellNodes));
    }
    const node = schema.nodes.table.createChecked(null, rowNodes);
    node.check();
    if (TableMap.get(node).problems) return null;
    return materializeInlineSource(node);
  } catch {
    return null;
  }
}

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
  sup: 'superscript',
  sub: 'subscript',
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

  if (tag === 'br') {
    state.openNode(schema.nodes.hard_break, {
      soft: extractAttr(content, 'data-nomo-break') === 'soft',
    });
    state.closeNode();
    return;
  }

  if (isSelfClosing) {
    const attrs = tag === 'a' ? getHtmlAnchorAttrs(content) : null;
    if (attrs) {
      state.openNode(schema.nodes.html_anchor, attrs);
      state.closeNode();
    } else state.addText(content);
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
      const linkAttrs = getHtmlAnchorAttrs(content);
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

/**
 * 六类行内格式保留原文字符。只有链接、图片、公式等既有结构继续走原 handler，
 * 不从已经裁剪代码空白或解码转义的 token.content 重建原文。
 */
tableMarkdownParserWithHandlers.tokenHandlers.inline = (state, token) => {
  const source = token.content;
  let cursor = 0;
  const appendSource = (text: string) => {
    if (!text) return;
    state.openMark(schema.marks.inline_source.create());
    state.addText(text);
    state.closeMark(schema.marks.inline_source);
  };
  const events = (token.children ?? [])
    .filter((child, index, children) =>
      isRetainedInlineToken(child) ||
      (child.type === 'html_inline' && (
        (getHtmlInlineCommentContent(child.content) !== null &&
          children[index + 1]?.type === 'html_inline' && /^<\/span\s*>$/i.test(children[index + 1].content)) ||
        (/^<\/span\s*>$/i.test(child.content) &&
          children[index - 1]?.type === 'html_inline' && getHtmlInlineCommentContent(children[index - 1].content) !== null)
      )),
    )
    .map((child) => ({ token: child, range: inlineTokenSourceRange(child) }))
    .filter((event): event is { token: Token; range: { from: number; to: number } } =>
      event.range != null && event.range.from >= 0 && event.range.to <= source.length,
    )
    .sort((a, b) => a.range.from - b.range.from);
  for (let index = 0; index < events.length; index++) {
    const event = events[index];
    if (event.range.from < cursor) continue;
    appendSource(source.slice(cursor, event.range.from));
    const next = events[index + 1];
    const commentContent = event.token.type === 'html_inline'
      ? getHtmlInlineCommentContent(event.token.content) : null;
    if (
      commentContent !== null && next?.token.type === 'html_inline' &&
      /^<\/span\s*>$/i.test(next.token.content) && event.range.to === next.range.from
    ) {
      state.openNode(schema.nodes.comment_inline, { content: commentContent });
      state.closeNode();
      cursor = next.range.to;
      index++;
      continue;
    }
    // 空锚点无法用 mark 表示；相邻开闭标签合并为保留属性的行内节点。
    if (
      event.token.type === 'html_inline' && /^<a(?:\s|>)/i.test(event.token.content) &&
      next?.token.type === 'html_inline' && /^<\/a\s*>$/i.test(next.token.content) &&
      event.range.to === next.range.from
    ) {
      const attrs = getHtmlAnchorAttrs(event.token.content);
      if (attrs) {
        state.openNode(schema.nodes.html_anchor, attrs);
        state.closeNode();
        cursor = next.range.to;
        index++;
        continue;
      }
    }
    if (event.token.type === 'text_special' && event.token.info === 'entity') {
      // 实体保持原来的显示字符，六类语法字符则用真实反斜杠保护其字面含义。
      appendSource(escapeInlineSourceLiteral(event.token.content));
    } else {
      tableMarkdownParserWithHandlers.tokenHandlers[event.token.type](state, event.token);
    }
    cursor = event.range.to;
  }
  appendSource(source.slice(cursor));
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

interface MarkdownSourceRange {
  from: number;
  to: number;
}

/** 图片转换前按真实 token 保护字面内容，避免反引号或容器标记改变匹配边界。 */
function getImageHtmlLiteralRanges(markdown: string): MarkdownSourceRange[] {
  if (!/<img\s/i.test(markdown)) return [];
  const lineStarts = [0];
  for (let index = 0; index < markdown.length; index++) {
    if (markdown[index] === '\n') lineStarts.push(index + 1);
  }
  const ranges: MarkdownSourceRange[] = [];
  let rowMap: Token['map'] = null;
  let sourceCursor = 0;
  for (const token of markdownIt.parse(markdown, {})) {
    if (token.type === 'tr_open') rowMap = token.map;
    if (token.type === 'tr_close') rowMap = null;
    const map = token.map ?? rowMap;
    if (!map) continue;
    const from = lineStarts[map[0]] ?? markdown.length;
    const to = lineStarts[map[1]] ?? markdown.length;
    if (token.type === 'fence' || token.type === 'code_block' || (
      token.type === 'html_block' && (
        /^\s*<(?:pre|script|style|textarea)(?:\s|>|$)/i.test(token.content) ||
        (/^\s*<!--/.test(token.content) && !token.content.includes('-->'))
      )
    )) {
      ranges.push({ from, to });
      sourceCursor = to;
      continue;
    }
    if (token.type !== 'inline' && token.type !== 'html_block') continue;
    const children = token.children ?? markdownIt.parseInline(token.content, {})[0].children ?? [];
    const literals = children.filter((child) =>
      child.type === 'code_inline' || child.type === 'inline_source_code' ||
      (child.type === 'html_inline' && child.content.startsWith('<!--')),
    );
    if (!literals.length && !rowMap) {
      sourceCursor = to;
      continue;
    }

    // 行内 token 已去掉引用/列表前缀；保留原始偏移，同时还原 CRLF 和表格转义竖线。
    const sourceFrom = Math.max(sourceCursor, from + (token.meta?.statsContentColumn ?? 0));
    const sourceOffsets: number[] = [];
    let source = '';
    for (let index = sourceFrom; index < to; index++) {
      if (markdown[index] === '\r' && markdown[index + 1] === '\n') continue;
      if (rowMap && markdown[index] === '\\' && markdown[index + 1] === '|') continue;
      sourceOffsets.push(index);
      source += markdown[index] === '\0' ? '\uFFFD' : markdown[index];
    }
    const positions: number[] = [];
    const lines = token.content.split('\n');
    let cursor = 0;
    let mapped = true;
    for (let line = 0; line < lines.length; line++) {
      const offset = source.indexOf(lines[line], cursor);
      if (offset < 0) { mapped = false; break; }
      for (let index = 0; index < lines[line].length; index++) {
        positions.push(sourceOffsets[offset + index]);
      }
      cursor = offset + lines[line].length;
      if (line < lines.length - 1) {
        const newline = source.indexOf('\n', cursor);
        if (newline < 0) { mapped = false; break; }
        positions.push(sourceOffsets[newline]);
        cursor = newline + 1;
      }
    }
    sourceCursor = mapped ? (sourceOffsets[cursor - 1] ?? sourceFrom - 1) + 1 : to;
    for (const child of literals) {
      const range = inlineTokenSourceRange(child);
      const start = range && positions[range.from];
      const end = range && positions[range.to - 1];
      // 无法证明精确位置时保留整个源码块，不能冒险改写字面内容。
      ranges.push(mapped && start != null && end != null ? { from: start, to: end + 1 } : { from, to });
    }
  }
  return ranges;
}

/**
 * 图片 HTML 可能把多行源码折叠成一行 Markdown。解析器需要折叠后的文本，
 * 但块对齐必须继续使用原始源码行号，因此同时记录每一行的来源范围。
 */
function preprocessImageHtmlWithProvenance(markdown: string): PreprocessedImageHtml {
  const literalRanges = getImageHtmlLiteralRanges(markdown);
  const wrappedChanges: Array<{ end: number; delta: number }> = [];
  const collapsedRanges: Array<MarkdownLineProvenance & { transformedLine: number }> = [];
  let removedLineCount = 0;

  // 步骤1：<p align="..."><img ...></p>（单行或多行）
  let result = markdown.replace(
    WRAPPED_IMAGE_HTML,
    (_full: string, align: string, imgTag: string, offset: number) => {
      if (literalRanges.some((range) => range.from < offset + _full.length && range.to > offset)) return _full;
      const cleaned = imgTag.replace(/\/>$/, '').replace(/>$/, '').trim();
      const attrs = parseHtmlImgAttrs(cleaned);
      if (!attrs.src) return _full;
      const parts: string[] = [`align=${align.toLowerCase()}`];
      if (attrs.width) parts.push(`width=${attrs.width}`);
      const titleStr = attrs.title ? ` "${serializeMarkdownLinkTitle(attrs.title)}"` : '';
      const alt = escapeInlineSourceLiteral(attrs.alt ?? '').replace(/\r/g, '&#13;').replace(/\n/g, '&#10;');
      const replacement = `![${alt}](${serializeMarkdownLinkDestination(attrs.src)}${titleStr}){${parts.join(' ')}}`;
      wrappedChanges.push({ end: offset + _full.length, delta: replacement.length - _full.length });
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
  const wrappedLiteralRanges = literalRanges.map((range) => {
    const delta = wrappedChanges.reduce((total, change) => change.end <= range.from ? total + change.delta : total, 0);
    return { from: range.from + delta, to: range.to + delta };
  });

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
      const tagEnd = imgTag.lastIndexOf('>') + 1;
      if (wrappedLiteralRanges.some((range) => range.from < offset + tagEnd && range.to > offset)) return imgTag;
      // 末尾空白还承担后续字面块的分隔，不能折叠掉代码块所需的空行或缩进。
      const trailing = wrappedLiteralRanges.some((range) => range.to > offset + tagEnd && (
        range.from < offset + imgTag.length || /^\s*$/.test(wrappedMarkdown.slice(offset + imgTag.length, range.from))
      ))
        ? imgTag.slice(tagEnd) : '';
      const cleaned = imgTag.replace(/\/>$/, '').replace(/>$/, '').trim();
      if (/<[^>]+<[^>]+>/.test(cleaned)) return imgTag; // 含嵌套标签，不处理
      const attrs = parseHtmlImgAttrs(cleaned);
      if (!attrs.src) return imgTag;
      const parts: string[] = [];
      if (attrs.width) parts.push(`width=${attrs.width}`);
      const titleStr = attrs.title ? ` "${serializeMarkdownLinkTitle(attrs.title)}"` : '';
      const alt = escapeInlineSourceLiteral(attrs.alt ?? '').replace(/\r/g, '&#13;').replace(/\n/g, '&#10;');
      const attrsStr = parts.length > 0 ? `{${parts.join(' ')}}` : '';
      const from = getLineNumberAtOffset(wrappedMarkdown, offset);
      const to = getLineNumberAtOffset(wrappedMarkdown, offset + imgTag.length - trailing.length);
      standaloneRanges.push({ line: from - standaloneRemoved, from, to });
      standaloneRemoved += to - from;
      return `![${alt}](${serializeMarkdownLinkDestination(attrs.src)}${titleStr})${attrsStr}${trailing}`;
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

export function normalizeImageSrc(value: unknown): string {
  const src = String(value ?? '').trim();
  if (!src || /^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(src)) return src;
  let decoded = src;
  try {
    // 与资源后端一致：解码有效 %XX，保留其它百分号。
    decoded = decodeURIComponent(src.replace(/%(?![\da-f]{2})/gi, '%25'));
  } catch {
    // 无效 UTF-8 与资源后端一致，保留原值用于保守比较。
  }
  const slashPath = decoded.replace(/\\/g, '/');
  const absolute = slashPath.startsWith('/');
  const parts: string[] = [];
  for (const part of slashPath.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..' && parts.length && parts[parts.length - 1] !== '..') parts.pop();
    else if (part !== '..' || !absolute) parts.push(part);
  }
  return `${absolute ? '/' : ''}${parts.join('/')}`;
}

/** 复用真实 tokenizer 收集原始图片地址；null 表示无法证明无引用，禁止资源清理。 */
export function collectMarkdownImageSrcs(markdown: string): Set<string> | null {
  try {
    const { body } = splitMarkdownDocument(markdown);
    const tokens = markdownIt.parse(preprocessImageHtml(body), {});
    const srcs = new Set<string>();
    for (const token of tokens) {
      // 原始 HTML 使用另一解析边界，不能据此推断图片已无引用。
      if (token.type === 'html_block' || !tableMarkdownParserWithHandlers.tokenHandlers[token.type])
        return null;
      if (token.type !== 'inline') continue;
      const events = (token.children ?? [])
        .filter(isRetainedInlineToken)
        .map((child) => ({ token: child, range: inlineTokenSourceRange(child) }));
      if (
        events.some(
          ({ range }) =>
            !range || range.from < 0 || range.to > token.content.length || range.to < range.from,
        )
      )
        return null;
      const sorted = events.sort((a, b) => a.range!.from - b.range!.from);
      let cursor = 0;
      for (const event of sorted) {
        if (event.range!.from < cursor) continue;
        const child = event.token;
        if (child.type === 'html_inline') return null;
        if (
          !(child.type === 'text_special' && child.info === 'entity') &&
          !tableMarkdownParserWithHandlers.tokenHandlers[child.type]
        )
          return null;
        if (child.type === 'image') {
          const src = child.attrGet('src');
          if (!src || !normalizeImageSrc(src)) return null;
          srcs.add(src);
          // 图片的子 token 属于替代文本，不能算作额外的图片节点。
        }
        cursor = event.range!.to;
      }
    }
    return srcs;
  } catch {
    return null;
  }
}

/** 返回实际解析结果；失败明确返回 null，不能用字面回退结果执行删除。 */
export interface MarkdownParseContext { references?: Record<string, { href: string; title: string }> }
export function tryParseMarkdown(markdown: string, options: MarkdownParseContext & { exportImageSources?: boolean } = {}): ProseMirrorNode | null {
  const validateLink = markdownIt.validateLink;
  if (options.exportImageSources) {
    // Word 需要读取临时/内嵌图片。只在本次同步解析期间接受这些图片来源；
    // 超链接仍由 createLinkAttrs 独立过滤，默认编辑解析及序列化策略不变。
    markdownIt.validateLink = (href) => validateLink(href) ||
      /^(?:data:image\/(?:png|jpe?g|gif|webp|svg\+xml);|blob:|asset:|file:)/i.test(href);
  }
  resetHtmlInlineStack();
  try {
    const { frontMatterPrefix, body } = splitMarkdownDocument(markdown);
    const parsed = tableMarkdownParser.parse(preprocessImageHtml(body), { references: { ...options.references } });
    return parsed.type.create({ ...parsed.attrs, frontMatterPrefix }, parsed.content, parsed.marks);
  } catch {
    return null;
  } finally {
    markdownIt.validateLink = validateLink;
    resetHtmlInlineStack();
  }
}
/** 仅使用真实 tokenizer 与纯图片预处理建立源码行映射，可在无 DOM 的源码大纲环境调用。 */
export function tokenizeMarkdownWithSourceLines(markdown: string): {
  tokens: Token[];
  sourceLine: (line: number) => number;
} {
  const { body } = splitMarkdownDocument(markdown);
  const preprocessed = preprocessImageHtmlWithProvenance(body);
  const lineOffset = markdown.slice(0, markdown.length - body.length).split('\n').length - 1;
  return {
    tokens: markdownIt.parse(preprocessed.markdown, {}),
    sourceLine: (line) => (preprocessed.lineProvenance[line]?.fromLine ?? line + 1) + lineOffset,
  };
}

export function parseMarkdown(markdown: string, context: MarkdownParseContext = {}): ProseMirrorNode {
  const parsed = tryParseMarkdown(markdown, context);
  if (parsed) return parsed;
  const { frontMatterPrefix, body: rawBody } = splitMarkdownDocument(markdown);
  return schema.node('doc', { frontMatterPrefix }, [
    schema.node(
      'paragraph',
      null,
      rawBody
        ? [schema.text(rawBody, [schema.marks.inline_source.create({ literal: true })])]
        : undefined,
    ),
  ]);
}

export interface MarkdownBlockLineMap {
  fromLine: number;
  toLine: number;
  nodeIndex: number;
}

/** 返回与实际解析节点绑定的嵌套来源映射；解析失败时不提供猜测锚点。 */
export function parseMarkdownWithSyncAnchors(markdown: string, context: MarkdownParseContext = {}): {
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
    }, { references: { ...context.references } });
    return {
      doc: result.doc.type.create({ ...result.doc.attrs, frontMatterPrefix }, result.doc.content, result.doc.marks),
      anchors: result.anchors,
    };
  } catch {
    return { doc: parseMarkdown(markdown, context), anchors: [] };
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
    createListInputRule(),
    textblockTypeInputRule(/^```$/, schema.nodes.code_block),
    createHorizontalRuleInputRule(),
  ];
}

function createMathInlineInputRule(): InputRule {
  return new InputRule(/\$(?:\\.|[^\\$])+\$$/, (state, match, start, end) => {
    const $start = state.doc.resolve(start);
    const block = $start.parent;
    if (block.type.spec.code || block.type.name === 'details_summary') return null;

    // 美元边界与解析器一致，保留 TeX 内的转义，不消耗公式前的正文。
    const before = block.textBetween(0, $start.parentOffset, '\ufffc', '\ufffc');
    const slashCount = before.match(/\\+$/)?.[0].length ?? 0;
    if (slashCount % 2 === 1 || before.endsWith('$')) return null;
    const tex = match[0].slice(1, -1).trim();
    if (!tex) return null;

    // 原文式代码与旧 code mark 均不能通过输入规则转换为语义公式。
    if (
      state.doc.rangeHasMark(start, end, schema.marks.code) ||
      (state.storedMarks ?? state.selection.$from.marks()).some((mark) => mark.type === schema.marks.code)
    ) return null;
    const fromOffset = $start.parentOffset;
    const toOffset = end - $start.start();
    if (
      isInlineSourceBlock(block) &&
      analyzeInlineSource(block).spans.some(
        (span) => span.type === 'code' && fromOffset < span.closeTo && toOffset > span.openFrom,
      )
    ) return null;

    // 仅继承整个被替换范围共有的语义 marks；inline_source 只描述文本载荷。
    let marks = ($start.nodeAfter?.marks ?? $start.marks()).filter((mark) => mark.type !== schema.marks.inline_source);
    state.doc.nodesBetween(start, end, (node) => {
      if (node.isInline) marks = marks.filter((mark) => mark.isInSet(node.marks));
    });
    const node = schema.nodes.math_inline.create({ tex }, null, block.type.allowedMarks(marks));
    return state.tr.replaceWith(start, end, node);
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
    const $start = state.doc.resolve(start);
    if (
      !state.selection.empty ||
      $start.parent.type !== schema.nodes.paragraph ||
      !$start.sameParent(state.selection.$to) ||
      $start.parentOffset !== 0 ||
      end !== $start.end()
    ) return null;

    const hrNode = schema.nodes.horizontal_rule.create();
    const emptyParagraph = schema.nodes.paragraph.create();
    const replacement = Fragment.fromArray([hrNode, emptyParagraph]);
    const parentDepth = $start.depth - 1;
    const paragraphIndex = $start.index(parentDepth);
    // 只替换当前合法段落，不让 fitting 将非法块提升到表格或其他容器外。
    if (!$start.node(parentDepth).canReplace(paragraphIndex, paragraphIndex + 1, replacement)) {
      return null;
    }
    const blockStart = $start.before($start.depth);
    const tr = state.tr.replaceWith(blockStart, $start.after($start.depth), replacement);
    return tr.setSelection(TextSelection.create(tr.doc, blockStart + hrNode.nodeSize + 1));
  });
}

function findLastIndex<T>(arr: T[], predicate: (value: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) return i;
  }
  return -1;
}

function extractAttr(rawTag: string, name: string): string | null {
  return parseHtmlAttributes(rawTag).find(([key]) => key === name)?.[1] ?? null;
}

function getHtmlAnchorAttrs(rawTag: string) {
  return parseHtmlAnchorAttrs(rawTag);
}

function getHtmlInlineCommentContent(rawTag: string): string | null {
  if (!/^<span(?:\s|>)/i.test(rawTag)) return null;
  const classes = extractAttr(rawTag, 'class')?.split(/\s+/) ?? [];
  return classes.includes('comment-inline') ? extractAttr(rawTag, 'data-comment') : null;
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
  return /^<!--[^\S\r\n]*toc[^\S\r\n]*-->\s*$/i.test(trimmed) || /^<!--[^\S\r\n]*\/toc[^\S\r\n]*-->\s*$/i.test(trimmed);
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
