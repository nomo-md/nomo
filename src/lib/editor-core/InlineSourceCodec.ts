import { Fragment, Mark, type Node as ProseMirrorNode } from 'prosemirror-model';
import type Token from 'markdown-it/lib/token.mjs';
import { schema } from './schema';
import { createMarkdownTokenizer, inlineTokenSourceRange } from './markdownTokenizer';

export type InlineFormatName =
  | 'strong'
  | 'em'
  | 'code'
  | 'strikethrough'
  | 'underline'
  | 'highlight';

export interface InlineSourceSpan {
  type: InlineFormatName;
  /** 正文范围，不包含左右定界符；均为 textblock.content 内的 PM 偏移。 */
  from: number;
  to: number;
  openFrom: number;
  openTo: number;
  closeFrom: number;
  closeTo: number;
}

export interface InlineSourceAnalysis {
  text: string;
  source: string;
  spans: InlineSourceSpan[];
  visibleText: string;
  /** visibleText 每个 UTF-16 单元对应的原始 content 偏移。 */
  visiblePositions: number[];
}

const formatNames = new Set<InlineFormatName>([
  'strong',
  'em',
  'code',
  'strikethrough',
  'underline',
  'highlight',
]);
const htmlFormats: Record<string, InlineFormatName> = {
  strong: 'strong',
  b: 'strong',
  em: 'em',
  i: 'em',
  code: 'code',
  s: 'strikethrough',
  del: 'strikethrough',
  strike: 'strikethrough',
  u: 'underline',
  mark: 'highlight',
};
const tokenizer = createMarkdownTokenizer({ inlineSource: true });
const blockAnalysis = new WeakMap<ProseMirrorNode, InlineSourceAnalysis>();

export function isInlineSourceText(node: ProseMirrorNode): boolean {
  return node.isText && node.marks.some((mark) => mark.type.name === 'inline_source');
}

export function isInlineSourceBlock(node: ProseMirrorNode): boolean {
  return node.isTextblock && node.type.name !== 'code_block' && node.type.name !== 'html_block';
}

export function inlineFormatDelimiters(
  type: InlineFormatName,
  text = '',
): { open: string; close: string } {
  if (type === 'code') {
    const width = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length)) + 1;
    const fence = '`'.repeat(width);
    const padding =
      /^`|`$/.test(text) || (/^ [\s\S]* $/.test(text) && /[^ ]/.test(text)) ? ' ' : '';
    return { open: fence + padding, close: padding + fence };
  }
  const pair: Record<Exclude<InlineFormatName, 'code'>, [string, string]> = {
    strong: ['**', '**'],
    em: ['*', '*'],
    strikethrough: ['~~', '~~'],
    underline: ['<u>', '</u>'],
    highlight: ['<mark>', '</mark>'],
  };
  const [open, close] = pair[type];
  return { open, close };
}

/** 纯文本粘贴显式转义，避免在新模型里意外变成 Markdown 格式。 */
export function escapeInlineSourceLiteral(text: string): string {
  return text.replace(
    /[\\`*_~<>\[\]$]|&(?=#(?:x[\da-f]+|\d+);|[a-z][a-z\d]+;)/gi,
    (character) => `\\${character}`,
  );
}

export function analyzeInlineSource(block: ProseMirrorNode): InlineSourceAnalysis {
  const cached = blockAnalysis.get(block);
  if (cached) return cached;
  let source = '';
  let virtualSource = '';
  let syntaxSource = '';
  const sourcePositions: Array<number | null> = [];
  let activeLink: Mark | undefined;
  const virtual = (text: string) => {
    virtualSource += text;
    syntaxSource += text;
    sourcePositions.push(...Array.from({ length: text.length }, () => null));
  };
  block.forEach((child, offset) => {
    const link = child.marks.find((mark) => mark.type.name === 'link');
    if (!(link ? activeLink?.eq(link) : !activeLink)) {
      if (activeLink) virtual('](#)');
      if (link) virtual('[');
      activeLink = link;
    }
    const text = child.isText
      ? child.text!
      : child.type.name === 'hard_break'
        ? '\n'
        : '\ufffc'.repeat(child.nodeSize);
    source += text;
    virtualSource += text;
    const literal = child.marks.some(
      (mark) => mark.type.name === 'inline_source' && mark.attrs.literal === true,
    );
    syntaxSource += literal ? '\ufffc'.repeat(text.length) : text;
    for (let index = 0; index < text.length; index++) sourcePositions.push(offset + index);
  });
  if (activeLink) virtual('](#)');
  const virtualAnalysis = analyzeSource(virtualSource, syntaxSource);
  const spans = virtualAnalysis.spans.flatMap((span): InlineSourceSpan[] => {
    const openFrom = sourcePositions[span.openFrom];
    const openLast = sourcePositions[span.openTo - 1];
    const closeFrom = sourcePositions[span.closeFrom];
    const closeLast = sourcePositions[span.closeTo - 1];
    if (openFrom == null || openLast == null || closeFrom == null || closeLast == null) return [];
    return [
      {
        type: span.type,
        from: openLast + 1,
        to: closeFrom,
        openFrom,
        openTo: openLast + 1,
        closeFrom,
        closeTo: closeLast + 1,
      },
    ];
  });
  let visibleText = '';
  const visiblePositions: number[] = [];
  virtualAnalysis.visiblePositions.forEach((position, index) => {
    const original = sourcePositions[position];
    if (original == null) return;
    visibleText += virtualAnalysis.visibleText[index];
    visiblePositions.push(original);
  });
  const result: InlineSourceAnalysis = {
    source,
    text: source,
    spans,
    visibleText,
    visiblePositions,
  };
  blockAnalysis.set(block, result);
  return result;
}

interface Replacement {
  from: number;
  to: number;
  text: string;
  positions: number[];
}

export function analyzeInlineSourceText(source: string): InlineSourceAnalysis {
  return analyzeSource(source, source);
}

function analyzeSource(source: string, syntaxSource: string): InlineSourceAnalysis {
  const tokens = tokenizer.parseInline(syntaxSource, {})[0]?.children ?? [];
  const spans: InlineSourceSpan[] = [];
  const stack: Array<{ type: InlineFormatName; from: number; to: number; tag?: string }> = [];
  const replacements: Replacement[] = [];
  for (const token of tokens) {
    const range = inlineTokenSourceRange(token);
    if (!range) continue;
    if (token.type === 'code_inline' || token.type === 'inline_source_code') {
      const htmlCode = token.type === 'inline_source_code';
      const from =
        range.from + (htmlCode ? token.meta.inlineSourceCode.openLength : token.markup.length);
      const to =
        range.to - (htmlCode ? token.meta.inlineSourceCode.closeLength : token.markup.length);
      spans.push({
        type: 'code',
        from,
        to,
        openFrom: range.from,
        openTo: from,
        closeFrom: to,
        closeTo: range.to,
      });
      if (htmlCode) {
        for (const match of source
          .slice(from, to)
          .matchAll(/&(?:#x[\da-f]+|#\d+|[a-z][a-z\d]+);/gi)) {
          const decoded = tokenizer.utils.unescapeAll(match[0]);
          const start = from + match.index!;
          if (decoded !== match[0])
            replacements.push({
              from: start,
              to: start + match[0].length,
              text: decoded,
              positions: Array.from({ length: decoded.length }, () => start),
            });
        }
        continue;
      }
      let raw = source.slice(from, to).replace(/\n/g, ' ');
      let offset = from;
      if (/^ ([\s\S]+) $/.test(raw) && /[^ ]/.test(raw)) {
        raw = raw.slice(1, -1);
        offset++;
      }
      replacements.push({
        from,
        to,
        text: raw,
        positions: Array.from({ length: raw.length }, (_, i) => offset + i),
      });
      continue;
    }
    const name = token.type.replace(/_(open|close)$/, '');
    const type = name === 's' ? 'strikethrough' : name;
    if (
      (type === 'strong' || type === 'em' || type === 'strikethrough') &&
      /_(open|close)$/.test(token.type)
    ) {
      const opening = token.type.endsWith('_open');
      const from = opening ? range.to - token.markup.length : range.from;
      const to = opening ? range.to : range.from + token.markup.length;
      if (opening) stack.push({ type, from, to });
      else closeSpan(stack, spans, type, from, to);
      continue;
    }
    if (token.type === 'html_inline') {
      const match = /^<(\/)?([a-z][\w-]*)(?:\s[^>]*)?>$/i.exec(token.content);
      const tag = match?.[2].toLowerCase();
      const htmlType = tag ? htmlFormats[tag] : undefined;
      if (!match || !htmlType || /\/>$/.test(token.content)) continue;
      if (!match[1]) stack.push({ type: htmlType, from: range.from, to: range.to, tag });
      else closeSpan(stack, spans, htmlType, range.from, range.to, tag);
    } else if (token.type === 'text_special') {
      const start =
        token.info === 'escape' && token.content.length < range.to - range.from
          ? range.from + 1
          : range.from;
      replacements.push({
        from: range.from,
        to: range.to,
        text: token.content,
        positions: Array.from({ length: token.content.length }, (_, i) =>
          Math.min(range.to - 1, start + i),
        ),
      });
    }
  }
  spans.sort((a, b) => a.openFrom - b.openFrom || b.closeTo - a.closeTo);
  for (const span of spans) {
    replacements.push({ from: span.openFrom, to: span.openTo, text: '', positions: [] });
    replacements.push({ from: span.closeFrom, to: span.closeTo, text: '', positions: [] });
  }
  replacements.sort((a, b) => a.from - b.from || b.to - a.to);
  let visibleText = '';
  const visiblePositions: number[] = [];
  let cursor = 0;
  for (const replacement of replacements) {
    if (replacement.from < cursor) continue;
    for (; cursor < replacement.from; cursor++) {
      visibleText += source[cursor];
      visiblePositions.push(cursor);
    }
    visibleText += replacement.text;
    for (const position of replacement.positions) visiblePositions.push(position);
    cursor = replacement.to;
  }
  for (; cursor < source.length; cursor++) {
    visibleText += source[cursor];
    visiblePositions.push(cursor);
  }
  return { text: source, source, spans, visibleText, visiblePositions };
}

function closeSpan(
  stack: Array<{ type: InlineFormatName; from: number; to: number; tag?: string }>,
  spans: InlineSourceSpan[],
  type: InlineFormatName,
  from: number,
  to: number,
  tag?: string,
): void {
  let index = stack.length - 1;
  while (index >= 0 && (stack[index].type !== type || stack[index].tag !== tag)) index--;
  if (index < 0) return;
  const opening = stack[index];
  stack.splice(index, 1);
  spans.push({
    type,
    from: opening.to,
    to: from,
    openFrom: opening.from,
    openTo: opening.to,
    closeFrom: from,
    closeTo: to,
  });
}

/** 旧 marks / HTML 导入只在边界转换一次；不改变代码、HTML 及其它原子块。 */
export function materializeInlineSource(doc: ProseMirrorNode): ProseMirrorNode {
  return materializeSourceNode(doc, 0, 0);
}

interface MaterializePositionMaps {
  left: number[];
  right: number[];
}

/** assoc < 0 留在新增符号之前，assoc >= 0 移到符号之后；映射在转换时生成。 */
export function materializeInlineSourceWithMapping(doc: ProseMirrorNode): {
  doc: ProseMirrorNode;
  mapPosition(position: number, assoc?: number): number;
} {
  const maps: MaterializePositionMaps = {
    left: new Array<number>(doc.content.size + 1),
    right: new Array<number>(doc.content.size + 1),
  };
  const converted = materializeSourceNode(doc, 0, 0, maps);
  return {
    doc: converted,
    mapPosition(position, assoc = 1) {
      const clamped = Math.max(0, Math.min(doc.content.size, position));
      return (
        (assoc < 0 ? maps.left : maps.right)[clamped] ?? Math.min(clamped, converted.content.size)
      );
    },
  };
}

function materializeSourceNode(
  doc: ProseMirrorNode,
  oldContent: number,
  newContent: number,
  maps?: MaterializePositionMaps,
): ProseMirrorNode {
  const identity = () => {
    if (maps)
      for (let i = 0; i <= doc.content.size; i++) {
        maps.left[oldContent + i] = newContent + i;
        maps.right[oldContent + i] = newContent + i;
      }
    return doc;
  };
  if (doc.type.name === 'code_block' || doc.type.name === 'html_block' || doc.isLeaf)
    return identity();
  if (!isInlineSourceBlock(doc)) {
    const children: ProseMirrorNode[] = [];
    let changed = false;
    let newOffset = 0;
    if (maps) maps.left[oldContent] = maps.right[oldContent] = newContent;
    doc.forEach((child, oldOffset) => {
      const beforeOld = oldContent + oldOffset;
      const beforeNew = newContent + newOffset;
      if (maps) maps.left[beforeOld] = maps.right[beforeOld] = beforeNew;
      const next = child.isLeaf
        ? child
        : materializeSourceNode(child, beforeOld + 1, beforeNew + 1, maps);
      children.push(next);
      changed ||= next !== child;
      // 子节点内容末端已包含关闭符号的左右关联位置，不在这里覆盖。
      newOffset += next.nodeSize;
      if (maps)
        maps.left[beforeOld + child.nodeSize] = maps.right[beforeOld + child.nodeSize] =
          newContent + newOffset;
    });
    return changed ? doc.copy(Fragment.fromArray(children)) : doc;
  }
  let alreadySource = true;
  doc.forEach((child) => {
    if (
      (child.isText && !isInlineSourceText(child)) ||
      child.marks.some((mark) => formatNames.has(mark.type.name as InlineFormatName))
    )
      alreadySource = false;
  });
  if (alreadySource) return identity();
  const children: ProseMirrorNode[] = [];
  const sourceMark = schema.marks.inline_source.create();
  let output = 0;
  let active: Array<{ type: InlineFormatName; close: string; marks: readonly Mark[] }> = [];
  const append = (text: string, marks: readonly Mark[] = []) => {
    if (text) {
      children.push(
        schema.text(
          text,
          sourceMark.addToSet(
            marks.filter((mark) => !formatNames.has(mark.type.name as InlineFormatName)),
          ),
        ),
      );
      output += text.length;
    }
  };
  const beforeInsert = (offset: number) => {
    if (!maps) return;
    maps.left[oldContent + offset] ??= newContent + output;
    maps.right[oldContent + offset] = newContent + output;
  };
  const afterInsert = (offset: number) => {
    if (maps) maps.right[oldContent + offset] = newContent + output;
  };
  doc.forEach((child, offset, index) => {
    beforeInsert(offset);
    const desired = child.isText
      ? child.marks
          .filter((mark) => formatNames.has(mark.type.name as InlineFormatName))
          .map((mark) => mark.type.name as InlineFormatName)
      : [];
    const linkMarks = child.marks.filter((mark) => mark.type.name === 'link');
    let common = 0;
    while (
      common < active.length &&
      active[common].type === desired[common] &&
      (active[common].type !== 'code' || Mark.sameSet(active[common].marks, linkMarks))
    )
      common++;
    while (active.length > common) {
      const closing = active.pop()!;
      append(closing.close, closing.marks);
    }
    for (const type of desired.slice(common)) {
      let content = child.textContent;
      let next = index + 1;
      for (
        ;
        next < doc.childCount &&
        doc.child(next).isText &&
        doc.child(next).marks.some((mark) => mark.type.name === type) &&
        (type !== 'code' ||
          Mark.sameSet(
            linkMarks,
            doc.child(next).marks.filter((mark) => mark.type.name === 'link'),
          ));
        next++
      )
        content += doc.child(next).textContent;
      let pair = inlineFormatDelimiters(type, content);
      if (type === 'strong' || type === 'em' || type === 'strikethrough') {
        const before = index > 0 ? doc.child(index - 1).textContent.slice(-1) : '';
        const after = next < doc.childCount ? doc.child(next).textContent.slice(0, 1) : '';
        const punctuation = (value: string) => Boolean(value && /[\p{P}\p{S}]/u.test(value));
        const word = (value: string) => Boolean(value && !/\s/u.test(value) && !punctuation(value));
        if (
          /^\s|\s$/.test(content) ||
          (punctuation(content.slice(0, 1)) && word(before)) ||
          (punctuation(content.slice(-1)) && word(after))
        ) {
          const tag = type === 'strong' ? 'strong' : type === 'em' ? 'em' : 'del';
          pair = { open: `<${tag}>`, close: `</${tag}>` };
        }
      }
      const delimiterMarks = type === 'code' ? linkMarks : [];
      append(pair.open, delimiterMarks);
      active.push({ type, close: pair.close, marks: delimiterMarks });
    }
    afterInsert(offset);
    if (child.isText) {
      const raw = child.text!;
      const literal = !isInlineSourceText(child) && !desired.includes('code');
      if (maps) {
        const escaped = literal
          ? new Set(
              Array.from(
                raw.matchAll(/[\\`*_~<>\[\]$]|&(?=#(?:x[\da-f]+|\d+);|[a-z][a-z\d]+;)/gi),
                (match) => match.index,
              ),
            )
          : new Set<number>();
        let inserted = 0;
        for (let i = 0; i < raw.length; i++) {
          const oldPosition = oldContent + offset + i;
          const position = newContent + output + i + inserted;
          maps.left[oldPosition] ??= position;
          if (escaped.has(i)) inserted++;
          maps.right[oldPosition] = newContent + output + i + inserted;
          maps.left[oldPosition + 1] = maps.right[oldPosition + 1] =
            newContent + output + i + inserted + 1;
        }
      }
      append(literal ? escapeInlineSourceLiteral(raw) : raw, child.marks);
    } else {
      const marks = child.marks.filter(
        (mark) => !formatNames.has(mark.type.name as InlineFormatName),
      );
      children.push(Mark.sameSet(child.marks, marks) ? child : child.mark(marks));
      if (maps)
        for (let i = 1; i <= child.nodeSize; i++) {
          maps.left[oldContent + offset + i] = maps.right[oldContent + offset + i] =
            newContent + output + i;
        }
      output += child.nodeSize;
    }
  });
  beforeInsert(doc.content.size);
  while (active.length) {
    const closing = active.pop()!;
    append(closing.close, closing.marks);
  }
  afterInsert(doc.content.size);
  return doc.copy(Fragment.fromArray(children));
}

export function projectInlineSource(doc: ProseMirrorNode): ProseMirrorNode {
  return projectInlineSourceWithMapping(doc).doc;
}

/** 输出专用临时语义树；不写回编辑状态，未变化的 atom 继续复用同一实例。 */
export function projectInlineSourceWithMapping(doc: ProseMirrorNode): {
  doc: ProseMirrorNode;
  mapPosition(position: number, assoc?: number): number;
} {
  const positions = new Array<number>(doc.content.size + 1);
  const visit = (
    node: ProseMirrorNode,
    oldContent: number,
    newContent: number,
  ): ProseMirrorNode => {
    if (node.type.name === 'code_block' || node.type.name === 'html_block' || node.isLeaf) {
      for (let i = 0; i <= node.content.size; i++) positions[oldContent + i] = newContent + i;
      return node;
    }
    if (isInlineSourceBlock(node)) {
      let hasSource = false;
      node.forEach((child) => {
        hasSource ||= isInlineSourceText(child);
      });
      if (!hasSource) {
        for (let i = 0; i <= node.content.size; i++) positions[oldContent + i] = newContent + i;
        return node;
      }
      const analysis = analyzeInlineSource(node);
      const result: ProseMirrorNode[] = [];
      const owners: Array<{ node: ProseMirrorNode; from: number; to: number }> = [];
      node.forEach((child, from) => owners.push({ node: child, from, to: from + child.nodeSize }));
      let owner = 0;
      let output = 0;
      let oldBoundary = 0;
      for (let i = 0; i < analysis.visibleText.length; i++) {
        const original = analysis.visiblePositions[i];
        while (oldBoundary <= original) positions[oldContent + oldBoundary++] = newContent + output;
        while (owner + 1 < owners.length && original >= owners[owner].to) owner++;
        const child = owners[owner]?.node;
        if (!child) continue;
        if (!child.isText) {
          if (original !== owners[owner].from) continue;
          result.push(child);
          output += child.nodeSize;
          continue;
        }
        let marks = child.marks.filter(
          (mark) =>
            mark.type.name !== 'inline_source' &&
            !formatNames.has(mark.type.name as InlineFormatName),
        );
        for (const span of analysis.spans) {
          if (original >= span.from && original < span.to)
            marks = schema.marks[span.type].create().addToSet(marks) as Mark[];
        }
        const value = analysis.visibleText[i];
        const previous = result[result.length - 1];
        if (previous?.isText && Mark.sameSet(previous.marks, marks))
          result[result.length - 1] = schema.text(previous.text! + value, marks);
        else result.push(schema.text(value, marks));
        output++;
      }
      while (oldBoundary <= node.content.size)
        positions[oldContent + oldBoundary++] = newContent + output;
      return node.copy(Fragment.fromArray(result));
    }
    const children: ProseMirrorNode[] = [];
    let newOffset = 0;
    positions[oldContent] = newContent;
    node.forEach((child, oldOffset) => {
      const beforeOld = oldContent + oldOffset;
      const beforeNew = newContent + newOffset;
      positions[beforeOld] = beforeNew;
      const projected = child.isLeaf ? child : visit(child, beforeOld + 1, beforeNew + 1);
      children.push(projected);
      if (!child.isLeaf)
        positions[beforeOld + child.nodeSize - 1] = beforeNew + projected.nodeSize - 1;
      newOffset += projected.nodeSize;
      positions[beforeOld + child.nodeSize] = newContent + newOffset;
    });
    return children.every((child, i) => child === node.child(i))
      ? node
      : node.copy(Fragment.fromArray(children));
  };
  const projected = visit(doc, 0, 0);
  return {
    doc: projected,
    mapPosition(position) {
      const clamped = Math.max(0, Math.min(doc.content.size, position));
      return positions[clamped] ?? Math.min(clamped, projected.content.size);
    },
  };
}

/** 留给 parser 使用；只有这些 token 继续走既有的结构化 inline handler。 */
export function isRetainedInlineToken(token: Token): boolean {
  return (
    [
      'link_open',
      'link_close',
      'image',
      'math_inline',
      'footnote_ref',
      'softbreak',
      'hardbreak',
    ].includes(token.type) ||
    (token.type === 'text_special' && token.info === 'entity') ||
    (token.type === 'html_inline' &&
      (/^<\/?a(?:\s|>)/i.test(token.content) ||
        /^<br\s*\/>$/i.test(token.content) ||
        /^<!--[\s\S]*-->$/.test(token.content)))
  );
}
