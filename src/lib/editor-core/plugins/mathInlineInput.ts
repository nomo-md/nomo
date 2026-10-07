import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { Plugin } from 'prosemirror-state';
import { schema } from '../schema';
import { MathInlineNodeView } from '../nodeViews/MathInlineNodeView';
import { isPlainTextPaste } from '../clipboardMarkdown';
import type { EditorView } from 'prosemirror-view';

interface InlineMathMatch {
  from: number;
  to: number;
  tex: string;
  isNew?: boolean; // 标记是否是新创建的行公式
}

export function mathInlineInputPlugin(getView?: () => EditorView | null): Plugin {
  return new Plugin({
    appendTransaction(transactions, _oldState, newState) {
      if (isPlainTextPaste(transactions)) return null;
      if (!transactions.some((tr) => tr.docChanged)) {
        return null;
      }

      const matches = findInlineMathTextMatches(newState.doc);
      if (matches.length === 0) {
        return null;
      }

      const tr = newState.tr;
      let newMatchPos: number | null = null;

      for (const match of matches.reverse()) {
        tr.replaceWith(match.from, match.to, schema.nodes.math_inline.create({ tex: match.tex }));
        if (match.isNew) {
          newMatchPos = match.from;
        }
      }

      // 如果有新创建的行公式，触发立即编辑
      if (newMatchPos !== null) {
        // 请求立即进入编辑态
        MathInlineNodeView.requestInstantEdit(getView?.() ?? undefined);
      }

      return tr;
    },
  });
}

function findInlineMathTextMatches(doc: ProseMirrorNode): InlineMathMatch[] {
  const matches: InlineMathMatch[] = [];

  doc.descendants((node, pos, parent) => {
    if (node.type === schema.nodes.code_block) {
      return false;
    }

    if (!node.isText || !node.text || hasCodeMark(node)) {
      return true;
    }

    if (parent?.type === schema.nodes.code_block) {
      return false;
    }

    for (const match of scanTextForInlineMath(node.text, pos)) {
      matches.push(match);
    }
    return true;
  });

  return matches;
}

function scanTextForInlineMath(text: string, absoluteTextPos: number): InlineMathMatch[] {
  const matches: InlineMathMatch[] = [];
  let index = 0;

  while (index < text.length) {
    const start = text.indexOf('$', index);
    if (start === -1) break;

    if (isEscapedDollar(text, start) || text[start + 1] === '$' || text[start - 1] === '$') {
      index = start + 1;
      continue;
    }

    const end = findClosingDollar(text, start + 1);
    if (end === -1) break;

    const tex = text.slice(start + 1, end);
    if (isValidInlineMathTex(tex)) {
      matches.push({
        from: absoluteTextPos + start,
        to: absoluteTextPos + end + 1,
        tex: tex.trim().replace(/\\\$/g, '$'),
        isNew: true, // 标记为新创建的行公式
      });
    }

    index = end + 1;
  }

  return matches;
}

function findClosingDollar(text: string, from: number): number {
  for (let index = from; index < text.length; index++) {
    if (text[index] !== '$') continue;
    if (text[index + 1] === '$' || text[index - 1] === '$') continue;
    if (!isEscapedDollar(text, index)) {
      return index;
    }
  }
  return -1;
}

function isValidInlineMathTex(tex: string): boolean {
  return Boolean(tex.trim());
}

function isEscapedDollar(text: string, dollarIndex: number): boolean {
  let slashCount = 0;
  for (let index = dollarIndex - 1; index >= 0 && text[index] === '\\'; index--) {
    slashCount += 1;
  }
  return slashCount % 2 === 1;
}

function hasCodeMark(node: ProseMirrorNode): boolean {
  return node.marks.some((mark) => mark.type === schema.marks.code);
}
