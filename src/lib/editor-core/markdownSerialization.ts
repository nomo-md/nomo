import {
  defaultMarkdownSerializer,
  MarkdownSerializer,
  MarkdownSerializerState,
} from 'prosemirror-markdown';
import {
  Slice,
  Fragment,
  type Mark,
  type Node as ProseMirrorNode,
  type ResolvedPos,
} from 'prosemirror-model';
import { schema, type TableColumnAlignment } from './schema';
import { serializeHtmlBlock } from './html/pmToHtml';
import { serializeCallout } from './callout/calloutSerializer';
import { TOC_START_MARKER, TOC_END_MARKER } from '../toc/tocMarkers';
import { escapeHtmlAttr } from './markdownTokenizer';
import { serializeMarkdownLinkDestination, serializeMarkdownLinkTitle } from './link';

const tableMarkdownSerializer = new MarkdownSerializer(
  {
    ...defaultMarkdownSerializer.nodes,
    paragraph(state, node) {
      const taskParagraph = splitTaskParagraph(node);
      if (taskParagraph) {
        state.write(taskParagraph.marker);
        state.renderInline(taskParagraph.content);
        state.closeBlock(node);
        return;
      }
      if (node.content.size === 0) {
        // 空段落只需要触发前一个块落盘；不写入当前列表缩进，避免保存成带空格的“空行”。
        flushPendingClosedBlock(state);
      } else {
        state.renderInline(node);
      }
      state.closeBlock(node);
    },
    hard_break(state, node, parent, index) {
      if (node.attrs.soft === true) {
        if (hasFollowingInlineContent(parent, index)) {
          state.write('\n');
        }
        return;
      }

      defaultMarkdownSerializer.nodes.hard_break(state, node, parent, index);
    },
    bullet_list(state, node) {
      state.renderList(node, '  ', () => '- ');
    },
    table(state, node) {
      state.ensureNewLine();
      state.write(serializeTable(node));
      state.closeBlock(node);
    },
    table_row() {
      return;
    },
    table_cell() {
      return;
    },
    table_header() {
      return;
    },
    html_block(state, node) {
      const html = serializeHtmlBlock(node);
      state.write(html);
      state.closeBlock(node);
    },
    comment_block(state, node) {
      state.ensureNewLine();
      state.write(serializeMarkdownComment(String(node.attrs.content ?? ''), true));
      state.closeBlock(node);
    },
    toc_block(state, node) {
      state.ensureNewLine();
      const content = String(node.attrs.content ?? '').trim();
      state.write(`${TOC_START_MARKER}\n`);
      if (content) {
        state.write(`${content}\n`);
      }
      state.write(`${TOC_END_MARKER}\n`);
      state.closeBlock(node);
    },
    image(state, node) {
      const src = String(node.attrs.src ?? '');
      const alt = String(node.attrs.alt ?? '');
      const title = node.attrs.title as string | null;
      const align = node.attrs.align as string | null;
      const width = node.attrs.width as string | null;

      if (align === 'left' || align === 'center' || align === 'right') {
        // 有对齐 → 块级 HTML：<p align="...">\n  <img ...>\n</p>
        state.ensureNewLine();
        let imgTag = `<img src="${escapeHtmlAttr(src)}" alt="${escapeHtmlAttr(alt)}"`;
        if (title) imgTag += ` title="${escapeHtmlAttr(title)}"`;
        if (width) imgTag += ` width="${escapeHtmlAttr(width)}"`;
        imgTag += '>';
        state.write(`<p align="${align}">\n  ${imgTag}\n</p>`);
        state.closeBlock(node);
      } else if (width) {
        // 只有宽度 → 内联 <img ...>（不换行，保留在段落内）
        let imgTag = `<img src="${escapeHtmlAttr(src)}" alt="${escapeHtmlAttr(alt)}"`;
        if (title) imgTag += ` title="${escapeHtmlAttr(title)}"`;
        imgTag += ` width="${escapeHtmlAttr(width)}"`;
        imgTag += '>';
        state.write(imgTag);
      } else {
        // 无样式 → 标准 Markdown 图片
        const escapedAlt = state.esc(alt, false);
        const escapedSrc = state.esc(src);
        const titleStr = title ? ` "${state.esc(title, false)}"` : '';
        state.write(`![${escapedAlt}](${escapedSrc}${titleStr})`);
      }
    },
    math_inline(state, node) {
      state.write(`$${node.attrs.tex.replace(/\$/g, '\\$')}$`);
    },
    comment_inline(state, node) {
      state.write(serializeMarkdownComment(String(node.attrs.content ?? ''), false));
    },
    footnote_ref(state, node) {
      state.write(`[^${node.attrs.id}]`);
    },
    footnote_def(state, node) {
      state.ensureNewLine();
      state.write(`[^${node.attrs.id}]: `);
      state.renderInline(node);
      state.closeBlock(node);
    },
    text(state, node, parent) {
      let escaped = escapeMarkdownTextWithoutManualInlineMarkers(node.text ?? '');
      if (containsLiteralDisplayMathSyntax(parent)) {
        escaped = escaped.replace(/(?<!\\)\$/g, '\\$');
      }
      const atBlank = (state as unknown as { atBlank(): boolean }).atBlank();
      state.text(atBlank ? escapeMarkdownBlockStart(escaped) : escaped, false);
    },
    math_block(state, node) {
      state.ensureNewLine();
      state.write('$$\n');
      state.write(node.attrs.tex as string);
      state.write('\n$$\n');
      state.closeBlock(node);
    },
    mermaid_block(state, node) {
      state.ensureNewLine();
      state.write('```mermaid\n');
      state.write(node.attrs.code as string);
      state.write('\n```\n');
      state.closeBlock(node);
    },
    callout(state, node) {
      serializeCallout(state, node);
    },
  },
  {
    ...defaultMarkdownSerializer.marks,
    em: {
      ...defaultMarkdownSerializer.marks.em,
      open(_state, mark, parent, index) {
        return emphasisNeedsHtml(mark, parent, index) ? '<em>' : '*';
      },
      close(_state, mark, parent, index) {
        return emphasisNeedsHtml(mark, parent, index) ? '</em>' : '*';
      },
    },
    strong: {
      ...defaultMarkdownSerializer.marks.strong,
      open(_state, mark, parent, index) {
        return emphasisNeedsHtml(mark, parent, index) ? '<strong>' : '**';
      },
      close(_state, mark, parent, index) {
        return emphasisNeedsHtml(mark, parent, index) ? '</strong>' : '**';
      },
    },
    strikethrough: {
      open: '~~',
      close: '~~',
      mixable: true,
      expelEnclosingWhitespace: true,
    },
    underline: {
      open: '<u>',
      close: '</u>',
      mixable: true,
      expelEnclosingWhitespace: true,
    },
    highlight: {
      open: '<mark>',
      close: '</mark>',
      mixable: true,
      expelEnclosingWhitespace: true,
    },
    link: {
      open: '[',
      close(_state, mark) {
        const href = serializeMarkdownLinkDestination(String(mark.attrs.href ?? ''));
        const title = mark.attrs.title
          ? ` "${serializeMarkdownLinkTitle(String(mark.attrs.title))}"`
          : '';
        return `](${href}${title})`;
      },
      mixable: true,
    },
  },
);

function emphasisNeedsHtml(mark: Mark, parent: ProseMirrorNode, index: number): boolean {
  let from = Math.min(index, parent.childCount - 1);
  if (from >= 0 && !mark.isInSet(parent.child(from).marks)) from -= 1;
  if (from < 0) return false;

  let to = from + 1;
  while (from > 0 && mark.isInSet(parent.child(from - 1).marks)) from -= 1;
  while (to < parent.childCount && mark.isInSet(parent.child(to).marks)) to += 1;

  const first = inlineBoundaryText(parent.child(from)).match(/^[\s\S]/u)?.[0];
  const last = inlineBoundaryText(parent.child(to - 1)).match(/[\s\S]$/u)?.[0];
  const before = from > 0 ? inlineBoundaryText(parent.child(from - 1)).match(/[\s\S]$/u)?.[0] : '';
  const after = to < parent.childCount ? inlineBoundaryText(parent.child(to)).match(/^[\s\S]/u)?.[0] : '';

  // * / ** 内侧是标点、外侧紧贴文字时不满足 CommonMark 边界规则；用 HTML 保留真实格式。
  if (
    (isInlinePunctuation(first) && isInlineWordCharacter(before)) ||
    (isInlinePunctuation(last) && isInlineWordCharacter(after))
  ) {
    return true;
  }

  // 加粗回退为 HTML 时，外层斜体的星号也可能紧贴标签，需一起保持兼容。
  if (mark.type === schema.marks.em) {
    for (let i = from; i < to; i++) {
      const strong = schema.marks.strong.isInSet(parent.child(i).marks);
      if (
        strong &&
        (i === from || !strong.isInSet(parent.child(i - 1).marks)) &&
        emphasisNeedsHtml(strong, parent, i)
      ) {
        return true;
      }
    }
  }
  return false;
}

function inlineBoundaryText(node: ProseMirrorNode): string {
  if (node.type.name === 'hard_break') return '\n';
  return node.textContent || '\ufffc';
}

function isInlinePunctuation(character: string | undefined): boolean {
  return Boolean(character && /[\p{P}\p{S}]/u.test(character));
}

function isInlineWordCharacter(character: string | undefined): boolean {
  return Boolean(character && !/\s/u.test(character) && !isInlinePunctuation(character));
}

function hasFollowingInlineContent(parent: ProseMirrorNode, index: number): boolean {
  for (let i = index + 1; i < parent.childCount; i++) {
    if (parent.child(i).type.name !== 'hard_break') {
      return true;
    }
  }
  return false;
}

export function serializeMarkdown(doc: ProseMirrorNode): string {
  return `${String(doc.attrs.frontMatterPrefix ?? '')}${tableMarkdownSerializer.serialize(doc)}`;
}

interface MarkdownSelectionExtraction {
  nodes: ProseMirrorNode[];
  fallbackToPlainText: boolean;
}
const topBlocksByDoc = new WeakMap<
  ProseMirrorNode,
  Array<{ node: ProseMirrorNode; offset: number }>
>();

/** 内容修订建立索引时预热；选区查询只查找命中的顶层块。 */
export function prepareMarkdownSelection(doc: ProseMirrorNode): void {
  if (topBlocksByDoc.has(doc)) return;
  const blocks: Array<{ node: ProseMirrorNode; offset: number }> = [];
  doc.forEach((node, offset) => blocks.push({ node, offset }));
  topBlocksByDoc.set(doc, blocks);
}

/**
 * 将语义编辑器选区转换为可独立粘贴的 Markdown。
 *
 * 完整覆盖块的可见内容时保留块结构；只覆盖块的一部分时去掉未完整选择的外层语法，
 * 但继续保留选中文字上的行内 marks。局部表格无法稳定合成合法表头，交由调用方降级为纯文本。
 */
export function serializeMarkdownSelection(
  doc: ProseMirrorNode,
  from: number,
  to: number,
): string | null {
  const selected = extractMarkdownSelection(doc, from, to);
  return selected ? serializeMarkdown(selected) : null;
}

/** 使用相同序列化规则计量，不构建大选区的整段 Markdown 字符串。 */
export function measureMarkdownSelection(
  doc: ProseMirrorNode,
  from: number,
  to: number,
): { chars: number; lines: number } | null {
  const selected = extractMarkdownSelection(doc, from, to);
  if (!selected) return null;
  type OutputState = MarkdownSerializerState & { out: string };
  // prosemirror-markdown 的状态构造器与 out 是内部 API；现有规则已使用内部 flushClose。
  const State = MarkdownSerializerState as unknown as new (
    nodes: MarkdownSerializer['nodes'],
    marks: MarkdownSerializer['marks'],
    options: MarkdownSerializerState['options'],
  ) => OutputState;
  const state = new State(tableMarkdownSerializer.nodes, tableMarkdownSerializer.marks, {
    ...tableMarkdownSerializer.options,
  });
  let chars = 0,
    breaks = 0,
    previousCR = false;
  const consume = (text: string) => {
    if (!text) return;
    chars += text.length;
    breaks +=
      (text.match(/\r\n|\r|\n/g) ?? []).length - (previousCR && text.startsWith('\n') ? 1 : 0);
    previousCR = text.endsWith('\r');
  };
  consume(String(selected.attrs.frontMatterPrefix ?? ''));
  selected.forEach((node, _offset, index) => {
    state.render(node, selected, index);
    // 规则只检查输出尾部的换行，以及链接前的 !／\\!。保留两个字符和 closed 状态，
    // 后续块的分隔语义不变；atBlank 等规则不会反复扫描已经输出的所有块。
    if (state.out.length > 2) {
      consume(state.out.slice(0, -2));
      state.out = state.out.slice(-2);
    }
  });
  consume(state.out);
  return { chars, lines: breaks + 1 };
}

function extractMarkdownSelection(
  doc: ProseMirrorNode,
  from: number,
  to: number,
): ProseMirrorNode | null {
  if (from < 0 || to > doc.content.size || from >= to) {
    return null;
  }

  const listTextblock = extractSingleListTextblockSelection(doc, from, to);
  if (listTextblock) {
    return schema.nodes.doc.create(null, listTextblock);
  }

  const extraction: MarkdownSelectionExtraction = {
    nodes: [],
    fallbackToPlainText: false,
  };
  prepareMarkdownSelection(doc);
  const blocks = topBlocksByDoc.get(doc)!;
  let lo = 0,
    hi = blocks.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (blocks[mid].offset + blocks[mid].node.nodeSize <= from) lo = mid + 1;
    else hi = mid;
  }
  for (let i = lo; i < blocks.length && blocks[i].offset < to; i++) {
    appendSelectedMarkdownNodes(extraction, blocks[i].node, blocks[i].offset, from, to);
  }

  if (extraction.fallbackToPlainText || extraction.nodes.length === 0) {
    return null;
  }

  return schema.nodes.doc.create(null, extraction.nodes);
}

/**
 * 列表序号和项目符号属于外层列表结构，不是正文里的可选字符。
 * 当选区始终位于同一个列表正文块时，只复制该正文块，避免完整选中文字后补回未选中的列表标记。
 */
function extractSingleListTextblockSelection(
  doc: ProseMirrorNode,
  from: number,
  to: number,
): ProseMirrorNode | null {
  const $from = doc.resolve(from);
  const $to = doc.resolve(to);
  if (!$from.sameParent($to) || !$from.parent.isTextblock || !isInsideListItem($from)) {
    return null;
  }

  const textblock = $from.parent;
  const contentFrom = $from.start($from.depth);
  const contentTo = $from.end($from.depth);
  if (from <= contentFrom && to >= contentTo) {
    return textblock;
  }

  const localFrom = Math.max(0, from - contentFrom);
  const localTo = Math.min(textblock.content.size, to - contentFrom);
  if (localFrom >= localTo) {
    return null;
  }

  return schema.nodes.paragraph.create(null, textblock.content.cut(localFrom, localTo));
}

function isInsideListItem($pos: ResolvedPos): boolean {
  for (let depth = $pos.depth - 1; depth > 0; depth -= 1) {
    if ($pos.node(depth).type === schema.nodes.list_item) {
      return true;
    }
  }
  return false;
}

function appendSelectedMarkdownNodes(
  extraction: MarkdownSelectionExtraction,
  node: ProseMirrorNode,
  pos: number,
  from: number,
  to: number,
) {
  if (!selectionIntersectsNode(node, pos, from, to)) {
    return;
  }

  const fullySelected = selectionCoversVisibleNodeContent(node, pos, from, to);
  if (node.type === schema.nodes.table) {
    if (fullySelected) {
      extraction.nodes.push(node);
    } else {
      extraction.fallbackToPlainText = true;
    }
    return;
  }

  if (fullySelected) {
    extraction.nodes.push(node);
    return;
  }

  if (node.isTextblock) {
    appendPartialTextblock(extraction, node, pos, from, to);
    return;
  }

  if (node.type === schema.nodes.bullet_list || node.type === schema.nodes.ordered_list) {
    appendPartialList(extraction, node, pos, from, to);
    return;
  }

  node.forEach((child, offset) => {
    appendSelectedMarkdownNodes(extraction, child, pos + 1 + offset, from, to);
  });
}

function appendPartialTextblock(
  extraction: MarkdownSelectionExtraction,
  node: ProseMirrorNode,
  pos: number,
  from: number,
  to: number,
) {
  const contentFrom = pos + 1;
  const localFrom = Math.max(0, from - contentFrom);
  const localTo = Math.min(node.content.size, to - contentFrom);
  if (localFrom >= localTo) {
    return;
  }

  extraction.nodes.push(schema.nodes.paragraph.create(null, node.content.cut(localFrom, localTo)));
}

function appendPartialList(
  extraction: MarkdownSelectionExtraction,
  list: ProseMirrorNode,
  pos: number,
  from: number,
  to: number,
) {
  let selectedItems: ProseMirrorNode[] = [];
  let selectedItemsStartIndex = 0;

  const flushSelectedItems = () => {
    if (selectedItems.length === 0) {
      return;
    }

    const attrs =
      list.type === schema.nodes.ordered_list
        ? {
            ...list.attrs,
            order: Number(list.attrs.order ?? 1) + selectedItemsStartIndex,
          }
        : list.attrs;
    extraction.nodes.push(list.type.create(attrs, selectedItems));
    selectedItems = [];
  };

  list.forEach((item, offset, index) => {
    const itemPos = pos + 1 + offset;
    if (!selectionIntersectsNode(item, itemPos, from, to)) {
      return;
    }

    if (selectionCoversVisibleNodeContent(item, itemPos, from, to)) {
      if (selectedItems.length === 0) {
        selectedItemsStartIndex = index;
      }
      selectedItems.push(item);
      return;
    }

    flushSelectedItems();
    appendSelectedMarkdownNodes(extraction, item, itemPos, from, to);
  });

  flushSelectedItems();
}

function selectionIntersectsNode(node: ProseMirrorNode, pos: number, from: number, to: number) {
  return from < pos + node.nodeSize && to > pos;
}

function selectionCoversVisibleNodeContent(
  node: ProseMirrorNode,
  pos: number,
  from: number,
  to: number,
) {
  const bounds = getVisibleNodeContentBounds(node, pos);
  return from <= bounds.from && to >= bounds.to;
}

function getVisibleNodeContentBounds(
  node: ProseMirrorNode,
  pos: number,
): { from: number; to: number } {
  if (node.isTextblock) {
    return {
      from: pos + 1,
      to: pos + 1 + node.content.size,
    };
  }

  if (node.isLeaf || node.childCount === 0) {
    return {
      from: pos,
      to: pos + node.nodeSize,
    };
  }

  const firstChild = node.child(0);
  const lastChild = node.child(node.childCount - 1);
  const firstBounds = getVisibleNodeContentBounds(firstChild, pos + 1);
  const lastChildPos = pos + 1 + node.content.size - lastChild.nodeSize;
  const lastBounds = getVisibleNodeContentBounds(lastChild, lastChildPos);
  return {
    from: firstBounds.from,
    to: lastBounds.to,
  };
}

function flushPendingClosedBlock(state: unknown): void {
  (state as { flushClose(): void }).flushClose();
}

function serializeTable(table: ProseMirrorNode): string {
  const rows: ProseMirrorNode[] = [];
  table.forEach((row) => rows.push(row));
  if (rows.length === 0) return '';

  const columnCount = Math.max(...rows.map((row) => row.childCount));
  const serializedRows = rows.map((row) => serializeTableRow(row, columnCount));
  const alignments = Array.from({ length: columnCount }, (_, index) =>
    readColumnAlignment(rows, index),
  );
  const separator = alignments.map((align) => {
    if (align === 'center') return ':---:';
    if (align === 'right') return '---:';
    return ':---';
  });

  return [serializedRows[0], separator, ...serializedRows.slice(1)]
    .map((cells) => `| ${cells.join(' | ')} |`)
    .join('\n');
}

function serializeTableRow(row: ProseMirrorNode, columnCount: number): string[] {
  const cells: string[] = [];
  row.forEach((cell) => cells.push(serializeTableCell(cell)));
  while (cells.length < columnCount) cells.push('');
  return cells;
}

function serializeTableCell(cell: ProseMirrorNode): string {
  const parts: string[] = [];
  cell.descendants((node, _pos, parent, index) => {
    if (node.isText) {
      parts.push(serializeInlineText(node, parent!, index));
      return false;
    }
    if (node.type.name === 'hard_break') {
      parts.push('<br>');
      return false;
    }
    return true;
  });
  // 表格分隔符只在最终输出时转义，不能删除正文或代码中的反斜杠。
  return parts.join('').replace(/\n/g, ' ').replace(/\|/g, '\\|').trim();
}

function serializeInlineText(node: ProseMirrorNode, parent: ProseMirrorNode, index: number): string {
  const raw = node.text ?? '';
  const code = node.marks.some((mark) => mark.type.name === 'code');
  let text = escapeTableText(raw);
  if (code) {
    // 代码跨度内不能用反斜杠转义反引号；围栏须长于正文中的最长连续反引号。
    const delimiter = '`'.repeat(
      (raw.match(/`+/g) ?? []).reduce((longest, run) => Math.max(longest, run.length), 0) + 1,
    );
    const padding = /^`|`$/.test(raw) || (/^ .* $/.test(raw) && /[^ ]/.test(raw)) ? ' ' : '';
    text = `${delimiter}${padding}${raw}${padding}${delimiter}`;
  }
  return node.marks.reduce((value, mark) => {
    if (mark.type.name === 'strong') {
      return emphasisNeedsHtml(mark, parent, index) ? `<strong>${value}</strong>` : `**${value}**`;
    }
    if (mark.type.name === 'em') {
      return emphasisNeedsHtml(mark, parent, index) ? `<em>${value}</em>` : `*${value}*`;
    }
    if (mark.type.name === 'code') return value;
    if (mark.type.name === 'strikethrough') return `~~${value}~~`;
    if (mark.type.name === 'underline') return `<u>${value}</u>`;
    if (mark.type.name === 'highlight') return `<mark>${value}</mark>`;
    if (mark.type.name === 'link') {
      const href = serializeMarkdownLinkDestination(String(mark.attrs.href ?? ''));
      const title = mark.attrs.title
        ? ` "${serializeMarkdownLinkTitle(String(mark.attrs.title))}"`
        : '';
      return `[${value}](${href}${title})`;
    }
    return value;
  }, text);
}

function splitTaskParagraph(
  node: ProseMirrorNode,
): { marker: string; content: ProseMirrorNode } | null {
  const firstChild = node.firstChild;
  if (!firstChild?.isText) return null;

  const match = /^\[[ x]\]\s?/.exec(firstChild.text ?? '');
  if (!match) return null;

  const children: ProseMirrorNode[] = [];
  const restText = (firstChild.text ?? '').slice(match[0].length);
  if (restText) children.push(node.type.schema.text(restText, firstChild.marks));
  for (let index = 1; index < node.childCount; index++) {
    children.push(node.child(index));
  }

  return {
    marker: match[0].endsWith(' ') ? match[0] : `${match[0]} `,
    content: node.type.create(node.attrs, Fragment.fromArray(children), node.marks),
  };
}

function escapeTableText(text: string): string {
  return text.replace(/\\/g, '\\\\');
}

function escapeMarkdownTextWithoutManualInlineMarkers(text: string): string {
  let escaped = text.replace(/[`\\[\]|_]/g, (match, index) =>
    match === '_' &&
    index > 0 &&
    index + 1 < text.length &&
    /\w/.test(text[index - 1] ?? '') &&
    /\w/.test(text[index + 1] ?? '')
      ? match
      : `\\${match}`,
  );

  if (/(?:\*{1,3})(?=\S)[\s\S]*?\S\*{1,3}/.test(escaped)) {
    escaped = escaped.replace(/(?<!\\)\*/g, '\\*');
  }
  if (/~~(?=\S)[\s\S]*?\S~~/.test(escaped)) {
    escaped = escaped.replace(/(?<!\\)~/g, '\\~');
  }
  if (/\$\$(?:[\s\S]*?\S)?\$\$|\$(?!\$)(?=\S)[^\n]*?\S\$(?!\$)/.test(escaped)) {
    escaped = escaped.replace(/(?<!\\)\$/g, '\\$');
  }
  escaped = escaped.replace(/(?<!\\)<(?=\/?[A-Za-z][^>]*>)/g, '\\<');
  return escaped;
}

function escapeMarkdownBlockStart(text: string): string {
  return text.replace(
    /^(\s{0,3})(#{1,6}(?:\s|$)|>(?:\s|$)|[-+*](?:\s|$)|\d+[.)](?:\s|$)|`{3,}|~{3,}|(?:[-*_]\s*){3,}$)/,
    (_match, indentation: string, marker: string) =>
      /^\d+[.)]/.test(marker)
        ? `${indentation}${marker.replace(/[.)]/, '\\$&')}`
        : `${indentation}\\${marker}`,
  );
}

function containsLiteralDisplayMathSyntax(parent: ProseMirrorNode): boolean {
  if (!parent.isTextblock || parent.type === schema.nodes.code_block) return false;
  const text = parent.textBetween(0, parent.content.size, '\n', '\n').trim();
  return /^\$\$[\s\S]*\$\$$/.test(text) && text.length > 2;
}

function readColumnAlignment(
  rows: ProseMirrorNode[],
  columnIndex: number,
): TableColumnAlignment | null {
  for (const row of rows) {
    if (columnIndex >= row.childCount) continue;
    const cell = row.child(columnIndex);
    const align = cell?.attrs.align;
    if (align === 'left' || align === 'center' || align === 'right') return align;
  }
  return null;
}

function serializeMarkdownComment(content: string, block: boolean): string {
  const safeContent = content.replace(/-->/g, '-- >').replace(/\r\n/g, '\n');
  if (block) {
    return `<!--\n${safeContent}\n-->`;
  }

  const inlineContent = safeContent.replace(/\n+/g, ' ').trim();
  return inlineContent ? `<!-- ${inlineContent} -->` : '<!---->';
}

export function serializeClipboardText(slice: Slice): string {
  let text = '';
  let hasBlock = false;
  let previousBlockWasEmptyParagraph = false;

  slice.content.nodesBetween(0, slice.content.size, (node) => {
    const nodeText = node.isText
      ? (node.text ?? '')
      : node.type.name === 'hard_break'
        ? '\n'
        : node.isLeaf
          ? (node.type.spec.leafText?.(node) ?? '')
          : '';

    if (node.isBlock && ((node.isLeaf && nodeText) || node.isTextblock)) {
      if (hasBlock) {
        text += previousBlockWasEmptyParagraph ? '\n' : '\n\n';
      }
      hasBlock = true;
      previousBlockWasEmptyParagraph =
        node.type === schema.nodes.paragraph && node.content.size === 0;
    }

    text += nodeText;
  });

  return text;
}
