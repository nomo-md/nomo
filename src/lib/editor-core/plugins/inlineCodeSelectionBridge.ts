import { Plugin } from 'prosemirror-state';
import type { Mark, Node as ProseMirrorNode } from 'prosemirror-model';
import { Decoration, DecorationSet } from 'prosemirror-view';
import { analyzeInlineSource, isInlineSourceBlock } from '../InlineSourceCodec';

const SELECTION_BRIDGE_CLASS = 'pm-inline-selection-bridge';
const CODE_BRIDGE_CLASS = 'pm-inline-code-selection-bridge';

type CodeRange = {
  from: number;
  to: number;
};
const codeRangesByDoc = new WeakMap<ProseMirrorNode, CodeRange[]>();

/**
 * 行内代码选区桥接插件。
 *
 * 浏览器原生选区只绘制字形区域，行内 code 的 padding/border 会把选区切断。
 * 这里给真实选区加一层可控背景，并在选区碰到 code 时临时移除 code 胶囊外观。
 */
export function inlineCodeSelectionBridgePlugin(): Plugin<DecorationSet> {
  return new Plugin({
    state: {
      init(_, state) {
        codeRangesByDoc.set(state.doc, findCodeRanges(state.doc));
        return buildInlineCodeSelectionBridgeDecorations(
          state.doc,
          state.selection.from,
          state.selection.to,
        );
      },
      apply(tr, value, _oldState, newState) {
        if (tr.docChanged) codeRangesByDoc.set(newState.doc, findCodeRanges(newState.doc));
        if (tr.docChanged || tr.selectionSet) {
          return buildInlineCodeSelectionBridgeDecorations(
            newState.doc,
            newState.selection.from,
            newState.selection.to,
          );
        }
        return value;
      },
    },
    props: {
      decorations(state) {
        return this.getState(state);
      },
    },
  });
}

function buildInlineCodeSelectionBridgeDecorations(
  doc: ProseMirrorNode,
  selectionFrom: number,
  selectionTo: number,
): DecorationSet {
  if (selectionFrom === selectionTo) {
    return DecorationSet.empty;
  }

  const from = Math.min(selectionFrom, selectionTo);
  const to = Math.max(selectionFrom, selectionTo);
  const decorations: Decoration[] = [
    Decoration.inline(from, to, {
      class: SELECTION_BRIDGE_CLASS,
    }),
  ];

  const ranges = codeRangesByDoc.get(doc) ?? [];
  let lo = 0, hi = ranges.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (ranges[mid].to <= from) lo = mid + 1; else hi = mid;
  }
  for (let i = lo; i < ranges.length && ranges[i].from < to; i++) {
    const range = ranges[i];
    const overlap = getSelectionOverlap(range, from, to);
    if (!overlap) continue;
    decorations.push(
      Decoration.inline(overlap.from, overlap.to, {
        class: CODE_BRIDGE_CLASS,
      }),
    );
  }

  return decorations.length > 0 ? DecorationSet.create(doc, decorations) : DecorationSet.empty;
}

function findCodeRanges(doc: ProseMirrorNode): CodeRange[] {
  const ranges: CodeRange[] = [];

  doc.descendants((node, pos) => {
    if (node.type.name === 'html_block' || node.type.spec.code) return false;
    if (isInlineSourceBlock(node)) {
      for (const span of analyzeInlineSource(node).spans) {
        if (span.type === 'code') ranges.push({ from: pos + 1 + span.from, to: pos + 1 + span.to });
      }
    }
    if (!node.isText || !node.text || !hasCodeMark(node.marks)) return true;

    const from = pos;
    const to = pos + node.nodeSize;
    const previous = ranges[ranges.length - 1];
    if (previous && previous.to === from) {
      previous.to = to;
      return true;
    }

    ranges.push({ from, to });
    return true;
  });

  return ranges.sort((a, b) => a.from - b.from);
}

function hasCodeMark(marks: readonly Mark[]): boolean {
  return marks.some((mark) => mark.type.name === 'code');
}

function getSelectionOverlap(
  range: CodeRange,
  selectionFrom: number,
  selectionTo: number,
): CodeRange | null {
  const from = Math.max(range.from, selectionFrom);
  const to = Math.min(range.to, selectionTo);
  return from < to ? { from, to } : null;
}
