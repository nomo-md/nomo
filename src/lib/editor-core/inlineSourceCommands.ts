import type { Mark, Node as PmNode } from 'prosemirror-model';
import { toggleMark } from 'prosemirror-commands';
import { TextSelection, type Command, type EditorState, type Transaction } from 'prosemirror-state';
import {
  analyzeInlineSource,
  analyzeInlineSourceText,
  inlineFormatDelimiters,
} from './InlineSourceCodec';
import {
  findEmptyInlineSourceTemplate,
  getEmptyInlineSourceTemplates,
} from './plugins/inlineSourceEditing';

export type InlineSourceFormat =
  | 'strong'
  | 'em'
  | 'code'
  | 'strikethrough'
  | 'underline'
  | 'highlight';

const FORMATS: readonly InlineSourceFormat[] = [
  'strong',
  'em',
  'code',
  'strikethrough',
  'underline',
  'highlight',
];
type SourceSpan = ReturnType<typeof analyzeInlineSource>['spans'][number];
type TextEdit = { from: number; to: number; text: string; order?: number; marks?: readonly Mark[] };
type SourceBlock = {
  node: PmNode;
  start: number;
  from: number;
  to: number;
  source: string;
  spans: SourceSpan[];
};

/** Commands and toolbar state use the same parsed source ranges as the display plugin. */
export function getActiveInlineSourceFormats(state: EditorState): Set<InlineSourceFormat> {
  const result = new Set<InlineSourceFormat>();
  if (inStandaloneHtmlBlock(state)) {
    for (const type of FORMATS) {
      const mark = state.schema.marks[type];
      if (
        state.selection.empty
          ? mark.isInSet(state.storedMarks ?? state.selection.$from.marks())
          : state.doc.rangeHasMark(state.selection.from, state.selection.to, mark)
      )
        result.add(type);
    }
    return result;
  }
  const blocks = selectedBlocks(state);
  for (const type of FORMATS) {
    const relevantBlocks =
      type === 'code' && !state.selection.empty ? blocks.filter(hasSelectedCodeText) : blocks;
    if (state.selection.empty) {
      if (
        blocks.some((block) =>
          block.spans.some(
            (span) => span.type === type && block.from > span.openFrom && block.from < span.closeTo,
          ),
        )
      )
        result.add(type);
    } else if (
      relevantBlocks.length &&
      relevantBlocks.every((block) => isSelectionFormatted(block, type))
    ) {
      result.add(type);
    }
  }
  for (const template of getEmptyInlineSourceTemplates(state)) {
    if (state.selection.from > template.from && state.selection.to < template.to) {
      result.add(template.type as InlineSourceFormat);
    }
  }
  return result;
}

/** Insert/remove real delimiters; no stored mark, fake caret position, or arrow-key interception. */
export function toggleInlineSourceFormat(type: InlineSourceFormat): Command {
  return (state, dispatch) => {
    if (inStandaloneHtmlBlock(state)) return toggleMark(state.schema.marks[type])(state, dispatch);
    const blocks = selectedBlocks(state).filter(
      (block) => type !== 'code' || state.selection.empty || hasSelectedCodeText(block),
    );
    if (!blocks.length) return false;
    const template = state.selection.empty ? findEmptyInlineSourceTemplate(state, type) : undefined;
    if (template) {
      if (dispatch) {
        const pair = inlineFormatDelimiters(type, '');
        const tr = state.tr;
        applyEdits(state, tr, [
          { from: template.from, to: template.from + pair.open.length, text: '' },
          { from: template.to - pair.close.length, to: template.to, text: '' },
        ]);
        tr.setMeta('inlineSourceTemplate', {
          action: 'remove',
          from: template.from,
          to: template.to,
        });
        restoreSelection(state, tr);
        dispatch(tr.scrollIntoView());
      }
      return true;
    }

    if (state.selection.empty) {
      const block = blocks[0];
      const span = block.spans
        .filter(
          (candidate) =>
            candidate.type === type &&
            block.from > candidate.openFrom &&
            block.from < candidate.closeTo,
        )
        .sort((a, b) => a.closeTo - a.openFrom - (b.closeTo - b.openFrom))[0];
      if (span) {
        if (dispatch) {
          const tr = state.tr;
          applyEdits(state, tr, delimiterRemoval(block.start, span));
          restoreSelection(state, tr);
          dispatch(tr.scrollIntoView());
        }
        return true;
      }
      if (dispatch) {
        const { open, close } = inlineFormatDelimiters(type, '');
        const from = state.selection.from;
        const tr = state.tr;
        const link =
          type === 'code'
            ? state.selection.$from.marks().find((mark) => mark.type.name === 'link')
            : undefined;
        applyEdits(state, tr, [
          {
            from,
            to: from,
            text: open + close,
            marks: link ? codeMarks(state, from, link) : undefined,
          },
        ]);
        markSourceBlocks(tr, blocks);
        const cursor = from + open.length;
        tr.setSelection(TextSelection.create(tr.doc, cursor));
        tr.setMeta('inlineSourceTemplate', {
          action: 'add',
          template: { from, to: from + open.length + close.length, cursor, type },
        });
        dispatch(tr.scrollIntoView());
      }
      return true;
    }

    const remove = blocks.every((block) => isSelectionFormatted(block, type));
    const edits = blocks.flatMap((block) =>
      remove ? removeFormatEdits(block, type) : applyFormatEdits(block, type),
    );
    if (!edits.length) return false;
    if (dispatch) {
      const tr = state.tr;
      applyEdits(state, tr, edits);
      markSourceBlocks(tr, blocks);
      restoreSelection(state, tr);
      dispatch(tr.scrollIntoView());
    }
    return true;
  };
}

/** Append to the caller's transaction so clearing source + links/atoms stays one undo step. */
export function appendClearInlineSourceStyles(state: EditorState, tr: Transaction): boolean {
  const blocks = selectedBlocks(state);
  const edits = blocks.flatMap((block) =>
    FORMATS.flatMap((type) => removeFormatEdits(block, type)),
  );
  for (const template of getEmptyInlineSourceTemplates(state)) {
    const { from, to, empty } = state.selection;
    if (
      (empty && from >= template.from && from <= template.to) ||
      (!empty && from <= template.from && to >= template.to)
    ) {
      edits.push({ from: template.from, to: template.to, text: '' });
    }
  }
  if (!edits.length) return false;
  applyEdits(state, tr, edits);
  return true;
}

function selectedBlocks(state: EditorState): SourceBlock[] {
  const result: SourceBlock[] = [];
  const { from, to } = state.selection;
  const add = (node: PmNode, start: number) => {
    if (!node.isTextblock || node.type.spec.code || node.type.name === 'html_block') return;
    const localFrom = Math.max(0, from - start);
    const localTo = Math.min(node.content.size, to - start);
    if (localFrom > localTo || (!state.selection.empty && localFrom === localTo)) return;
    const analysis = analyzeInlineSource(node);
    result.push({
      node,
      start,
      from: localFrom,
      to: localTo,
      source: analysis.source,
      spans: analysis.spans,
    });
  };
  if (state.selection.empty) {
    const { $from } = state.selection;
    add($from.parent, $from.start());
  } else {
    state.doc.nodesBetween(from, to, (node, pos) => {
      if (node.type.spec.code || node.type.name === 'html_block') return false;
      if (node.isTextblock) {
        add(node, pos + 1);
        return false;
      }
      return true;
    });
  }
  return result;
}

function inStandaloneHtmlBlock(state: EditorState): boolean {
  const { $from, $to } = state.selection;
  return $from.parent === $to.parent && $from.parent.type.name === 'html_block';
}

function hasSelectedCodeText(block: SourceBlock): boolean {
  let found = false;
  block.node.forEach((child, offset) => {
    if (
      (child.isText || child.type.name === 'hard_break') &&
      offset < block.to &&
      offset + child.nodeSize > block.from
    )
      found = true;
  });
  return found;
}

function isSelectionFormatted(block: SourceBlock, type: InlineSourceFormat): boolean {
  const formats = block.spans.filter((span) => span.type === type);
  let hasContent = false;
  for (let pos = block.from; pos < block.to; pos += 1) {
    const child = type === 'code' ? block.node.childAfter(pos).node : null;
    if (child && !child.isText && child.type.name !== 'hard_break') continue;
    if (
      block.spans.some(
        (span) =>
          (pos >= span.openFrom && pos < span.openTo) ||
          (pos >= span.closeFrom && pos < span.closeTo),
      )
    )
      continue;
    hasContent = true;
    if (!formats.some((span) => pos >= span.from && pos < span.to)) return false;
  }
  return hasContent;
}

function delimiterRemoval(start: number, span: SourceSpan): TextEdit[] {
  return [
    { from: start + span.openFrom, to: start + span.openTo, text: '' },
    { from: start + span.closeFrom, to: start + span.closeTo, text: '' },
  ];
}

function removeFormatEdits(block: SourceBlock, type: InlineSourceFormat): TextEdit[] {
  const edits: TextEdit[] = [];
  const cursor = block.from === block.to;
  for (const span of block.spans.filter((candidate) => candidate.type === type)) {
    if (
      cursor
        ? !(block.from >= span.openFrom && block.from <= span.closeTo)
        : !(block.from < span.closeTo && block.to > span.openFrom)
    )
      continue;
    edits.push(...delimiterRemoval(block.start, span));
    if (cursor) continue;
    const leftTo = Math.min(block.from, span.to);
    const rightFrom = Math.max(block.to, span.from);
    // Split only the selected format. HTML boundaries avoid ambiguous adjacent * runs.
    if (leftTo > span.from) {
      edits.push(...wrapEdits(block, type, span.from, leftTo, true));
    }
    if (rightFrom < span.to) {
      edits.push(...wrapEdits(block, type, rightFrom, span.to, true));
    }
  }
  return edits;
}

function applyFormatEdits(block: SourceBlock, type: InlineSourceFormat): TextEdit[] {
  let from = block.from;
  let to = block.to;
  const edits: TextEdit[] = [];
  // Merge intersecting same-format spans rather than nesting identical delimiters.
  const spans = block.spans.filter((span) => span.type === type);
  const merged = new Set<SourceSpan>();
  let changed = true;
  while (changed) {
    changed = false;
    for (const span of spans) {
      if (merged.has(span) || span.openFrom >= to || span.closeTo <= from) continue;
      merged.add(span);
      from = Math.min(from, span.openFrom);
      to = Math.max(to, span.closeTo);
      edits.push(...delimiterRemoval(block.start, span));
      changed = true;
    }
  }
  return [...edits, ...wrapEdits(block, type, from, to, merged.size > 0, [...merged])];
}

function wrapEdits(
  block: SourceBlock,
  type: InlineSourceFormat,
  from: number,
  to: number,
  preferHtml = false,
  removedSpans: SourceSpan[] = [],
): TextEdit[] {
  if (type === 'code') return wrapCodeEdits(block, from, to, removedSpans);
  let { open, close } = inlineFormatDelimiters(type, block.source.slice(from, to));
  if (type === 'strong' || type === 'em' || type === 'strikethrough') {
    const candidate =
      block.source.slice(0, from) +
      open +
      block.source.slice(from, to) +
      close +
      block.source.slice(to);
    const valid =
      !preferHtml &&
      analyzeInlineSourceText(candidate).spans.some(
        (span) =>
          span.type === type &&
          span.openFrom === from &&
          span.openTo === from + open.length &&
          span.closeFrom === to + open.length,
      );
    if (!valid) {
      const tag = type === 'strong' ? 'strong' : type === 'em' ? 'em' : 'del';
      open = `<${tag}>`;
      close = `</${tag}>`;
    }
  }
  return [
    { from: block.start + from, to: block.start + from, text: open, order: 1 },
    { from: block.start + to, to: block.start + to, text: close, order: 0 },
  ];
}

type CodeSegment = {
  from: number;
  to: number;
  link?: Mark;
  parts: Array<{ from: number; to: number }>;
};

/** Markdown code cannot enclose a structured link/atom: keep each link identity and skip atoms. */
function wrapCodeEdits(
  block: SourceBlock,
  from: number,
  to: number,
  removedSpans: SourceSpan[],
): TextEdit[] {
  const removed = removedSpans
    .flatMap((span) => [
      { from: span.openFrom, to: span.openTo },
      { from: span.closeFrom, to: span.closeTo },
    ])
    .sort((a, b) => a.from - b.from);
  const segments: CodeSegment[] = [];
  let interrupted = false;
  block.node.forEach((child, offset) => {
    const start = Math.max(from, offset);
    const end = Math.min(to, offset + child.nodeSize);
    if (start >= end) return;
    if (!child.isText && child.type.name !== 'hard_break') {
      interrupted = true;
      return;
    }
    const link = child.marks.find((mark) => mark.type.name === 'link');
    const parts: Array<{ from: number; to: number }> = [];
    let cursor = start;
    for (const range of removed) {
      if (range.to <= cursor || range.from >= end) continue;
      if (range.from > cursor) parts.push({ from: cursor, to: Math.min(range.from, end) });
      cursor = Math.max(cursor, range.to);
    }
    if (cursor < end) parts.push({ from: cursor, to: end });
    if (!parts.length) return;
    const previous = segments[segments.length - 1];
    const sameLink =
      (!previous?.link && !link) || Boolean(previous?.link && link && previous.link.eq(link));
    if (previous && !interrupted && sameLink) {
      previous.to = parts[parts.length - 1].to;
      previous.parts.push(...parts);
    } else {
      segments.push({ from: parts[0].from, to: parts[parts.length - 1].to, link, parts });
    }
    interrupted = false;
  });
  const edits: TextEdit[] = [];
  const sourceMark = block.node.type.schema.marks.inline_source.create();
  for (const segment of segments) {
    const inheritedHtml = block.spans.some(
      (span) =>
        span.type === 'code' &&
        span.from <= segment.from &&
        span.to >= segment.to &&
        /^<code\b/i.test(block.source.slice(span.openFrom, span.openTo)),
    );
    const text = segment.parts.map((part) => block.source.slice(part.from, part.to)).join('');
    const pair = inheritedHtml
      ? { open: '<code>', close: '</code>' }
      : inlineFormatDelimiters('code', text);
    const marks = segment.link ? segment.link.addToSet([sourceMark]) : [sourceMark];
    edits.push({
      from: block.start + segment.from,
      to: block.start + segment.from,
      text: pair.open,
      order: 1,
      marks,
    });
    edits.push({
      from: block.start + segment.to,
      to: block.start + segment.to,
      text: pair.close,
      order: 0,
      marks,
    });
  }
  return edits;
}

function codeMarks(state: EditorState, pos: number, link: Mark): readonly Mark[] {
  return link.addToSet(sourceMarksAt(state, pos));
}

function sourceMarksAt(state: EditorState, pos: number): readonly Mark[] {
  // A delimiter crossing a link boundary must serialize outside the link label;
  // otherwise `[a**b](url)c**` cannot round-trip as emphasis in Markdown.
  let marks: readonly Mark[] = state.doc
    .resolve(pos)
    .marks()
    .filter(
      (mark) =>
        mark.type.name !== 'link' && !FORMATS.includes(mark.type.name as InlineSourceFormat),
    );
  const source = state.schema.marks.inline_source;
  if (source) marks = source.create().addToSet(marks);
  return marks;
}

function applyEdits(state: EditorState, tr: Transaction, input: TextEdit[]): void {
  const unique = new Map<string, TextEdit>();
  for (const edit of input)
    unique.set(
      `${edit.from}:${edit.to}:${edit.text}:${edit.order ?? 0}:${edit.marks?.map((mark) => JSON.stringify(mark.toJSON())).join(',') ?? ''}`,
      edit,
    );
  const edits = [...unique.values()];
  // Deletions first, then mapped insertions: spans may share a delimiter endpoint.
  for (const edit of edits.filter((edit) => edit.to > edit.from).sort((a, b) => b.from - a.from)) {
    const from = tr.mapping.map(edit.from, 1);
    const to = tr.mapping.map(edit.to, -1);
    if (to > from) tr.delete(from, to);
  }
  const insertionMapping = tr.mapping.slice();
  for (const edit of edits
    .filter((edit) => edit.text)
    .sort((a, b) => b.from - a.from || (b.order ?? 0) - (a.order ?? 0))) {
    const position = insertionMapping.map(edit.from, 1);
    tr.insert(
      position,
      state.schema.text(edit.text, edit.marks ?? sourceMarksAt(state, edit.from)),
    );
  }
}

function markSourceBlocks(tr: Transaction, blocks: SourceBlock[]): void {
  const source = tr.doc.type.schema.marks.inline_source;
  if (!source) return;
  for (const block of blocks) {
    const from = tr.mapping.map(block.start, -1);
    const to = tr.mapping.map(block.start + block.node.content.size, 1);
    const ranges: Array<{ from: number; to: number }> = [];
    tr.doc.nodesBetween(from, to, (node, pos) => {
      if (node.isText && !source.isInSet(node.marks))
        ranges.push({ from: pos, to: pos + node.nodeSize });
    });
    for (const range of ranges) tr.addMark(range.from, range.to, source.create());
  }
}

function restoreSelection(state: EditorState, tr: Transaction): void {
  const { anchor, head } = state.selection;
  const mappedAnchor = tr.mapping.map(anchor, anchor <= head ? 1 : -1);
  const mappedHead = tr.mapping.map(head, anchor <= head ? -1 : 1);
  tr.setSelection(
    TextSelection.create(tr.doc, mappedAnchor, state.selection.empty ? mappedAnchor : mappedHead),
  );
}
