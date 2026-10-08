import { Fragment, Slice, type Mark, type Node as ProseMirrorNode, type ResolvedPos } from 'prosemirror-model';
import type { Transaction } from 'prosemirror-state';
import { parseMarkdown } from './markdown';
import { schema } from './schema';
import { projectInlineSource, escapeInlineSourceLiteral } from './InlineSourceCodec';

export const plainTextPasteMeta = 'nomoPlainTextPaste';

export type ClipboardMarkdownClassification =
  | { kind: 'plain'; doc: ProseMirrorNode }
  | { kind: 'markdown'; doc: ProseMirrorNode }
  | { kind: 'front-matter'; doc: ProseMirrorNode };

/**
 * 通过比较 Markdown 解析树和纯文本解析树识别真正具有语义的剪贴板内容。
 * 这样普通换行文本不会被误判，扩展语法也不需要维护第二套正则列表。
 */
export function classifyClipboardMarkdown(text: string): ClipboardMarkdownClassification {
  const normalized = normalizeClipboardText(text);
  const markdownDoc = parseMarkdown(normalized);
  if (String(markdownDoc.attrs.frontMatterPrefix ?? '')) {
    return { kind: 'front-matter', doc: markdownDoc };
  }

  const plainDoc = createPlainTextDocument(normalized);
  return projectInlineSource(markdownDoc).eq(plainDoc)
    ? { kind: 'plain', doc: markdownDoc }
    : { kind: 'markdown', doc: markdownDoc };
}

export function createPlainTextSlice(text: string, $context: ResolvedPos): Slice {
  const literal = normalizeClipboardText(text);
  const richContext = Array.from({ length: $context.depth }, (_, i) => $context.node(i + 1))
    .some((node) => node.type.spec.code || node.type.name === 'html_block');
  const marks = richContext ? $context.marks() : [
    ...$context.marks().filter((mark) => mark.type.name === 'link'),
    schema.marks.inline_source.create(),
  ];
  return Slice.maxOpen(Fragment.fromArray(createPlainTextBlocks(
    richContext ? literal : escapePlainMarkdownText(literal), marks,
  )));
}

function escapePlainMarkdownText(text: string): string {
  return escapeInlineSourceLiteral(text).replace(/\|/g, '\\|').replace(
    /^(\s{0,3})(#{1,6}(?=\s|$)|[-+](?=\s)|\d+[.)](?=\s)|-{3,}\s*$)/gm,
    (_match, indent: string, marker: string) => /^\d/.test(marker)
      ? `${indent}${marker.replace(/[.)]/, '\\$&')}` : `${indent}\\${marker}`,
  );
}

export function createMarkdownClipboardSlice(doc: ProseMirrorNode): Slice {
  if (doc.childCount === 1 && doc.firstChild?.type === schema.nodes.paragraph) {
    return Slice.maxOpen(doc.content);
  }
  return new Slice(doc.content, 0, 0);
}

export function isPlainTextPaste(transactions: readonly Transaction[]): boolean {
  return transactions.some((transaction) => transaction.getMeta(plainTextPasteMeta) === true);
}

function normalizeClipboardText(text: string): string {
  return text.replace(/\r\n?/g, '\n');
}

function createPlainTextDocument(text: string): ProseMirrorNode {
  return schema.nodes.doc.create(null, createPlainTextBlocks(text, []));
}

function createPlainTextBlocks(text: string, marks: readonly Mark[]): ProseMirrorNode[] {
  const paragraphs: ProseMirrorNode[] = [];
  let inlineContent: ProseMirrorNode[] = [];
  let textStart = 0;

  const appendText = (value: string) => {
    if (value) {
      inlineContent.push(schema.text(value, marks));
    }
  };
  const closeParagraph = () => {
    paragraphs.push(schema.nodes.paragraph.create(null, inlineContent));
    inlineContent = [];
  };

  for (const match of text.matchAll(/\n+/g)) {
    const lineBreaks = match[0].length;
    appendText(text.slice(textStart, match.index));
    textStart = match.index + lineBreaks;

    if (lineBreaks === 1) {
      inlineContent.push(schema.nodes.hard_break.create({ soft: true }));
      continue;
    }

    closeParagraph();
    for (let index = 2; index < lineBreaks; index += 1) {
      paragraphs.push(schema.nodes.paragraph.create());
    }
  }

  appendText(text.slice(textStart));
  closeParagraph();
  return paragraphs;
}
