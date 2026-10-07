import { Plugin, PluginKey, NodeSelection } from 'prosemirror-state';
import type { Node as ProseMirrorNode, ResolvedPos } from 'prosemirror-model';
import type { EditorView } from 'prosemirror-view';

/**
 * codeBlockNavigation 插件 —— 处理代码块 / 公式块的上下方向键导航
 *
 * ProseMirror 默认会跳过 atomic 节点（code_block / math_block），导致 ↓ 直接跳到块下方。
 * 本插件在 handleKeyDown（早于 keymap 和默认处理）中拦截：
 *   1. 光标在文本块边界且紧邻代码块 → 直接进入代码块编辑态
 *   2. 代码块已选中 → 进入编辑态
 */

// 对外暴露的回调：由 ProseMirrorEditorCore 注入，用于获取 NodeView 实例
export type CodeBlockNavCallback = {
  /** 阶段2：代码块已选中，调用 enterEdit 进入编辑态 */
  enterEditAt: (view: EditorView, pos: number, clickLine: number, caret: 'start' | 'end') => void;
  /** 公式块已选中或相邻时，调用 enterEdit 进入编辑态 */
  enterMathEditAt?: (view: EditorView, pos: number, caret: 'start' | 'end') => void;
  /** Mermaid 图表块已选中或相邻时，调用 enterEdit 进入编辑态 */
  enterMermaidEditAt?: (view: EditorView, pos: number, caret: 'start' | 'end') => void;
  /** Arrow 默认选中公式块时，NodeView 会消费该意图并立即进入编辑态 */
  prepareMathKeyboardEntry?: (caret: 'start' | 'end') => void;
};

const codeBlockNavigationKey = new PluginKey('codeBlockNavigation');

type AdjacentEditableBlock = {
  node: ProseMirrorNode;
  pos: number;
};

const EDITABLE_BLOCK_NAMES = new Set(['code_block', 'math_block', 'mermaid_block']);

const VISUAL_LINE_TOLERANCE_PX = 4;

function isSameVisualLine(view: EditorView, a: number, b: number): boolean {
  try {
    const aRect = view.coordsAtPos(a);
    const bRect = view.coordsAtPos(b);
    return Math.abs(aRect.top - bRect.top) <= VISUAL_LINE_TOLERANCE_PX;
  } catch {
    return false;
  }
}

function isOnLastVisualLine(view: EditorView, $from: ResolvedPos): boolean {
  if ($from.parentOffset === $from.parent.content.size) return true;
  const textblockEnd = $from.start($from.depth) + $from.parent.content.size;
  return isSameVisualLine(view, $from.pos, textblockEnd);
}

function isOnFirstVisualLine(view: EditorView, $from: ResolvedPos): boolean {
  if ($from.parentOffset === 0) return true;
  const textblockStart = $from.start($from.depth);
  return isSameVisualLine(view, $from.pos, textblockStart);
}

function findNextEditableBlock(
  $from: ResolvedPos,
  canLeaveTextblock: boolean,
): AdjacentEditableBlock | null {
  if (!canLeaveTextblock) return null;

  for (let depth = $from.depth; depth > 0; depth--) {
    // 只有光标位于当前文本块以及所有祖先容器的末端时，才允许向外寻找下一个文档块。
    if (depth < $from.depth && $from.indexAfter(depth) !== $from.node(depth).childCount) {
      return null;
    }

    const nextPos = $from.after(depth);
    const nodeAfter = $from.doc.resolve(nextPos).nodeAfter;
    if (nodeAfter && EDITABLE_BLOCK_NAMES.has(nodeAfter.type.name)) {
      return { node: nodeAfter, pos: nextPos };
    }
    if (nodeAfter) return null;
  }

  return null;
}

function findPreviousEditableBlock(
  $from: ResolvedPos,
  canLeaveTextblock: boolean,
): AdjacentEditableBlock | null {
  if (!canLeaveTextblock) return null;

  for (let depth = $from.depth; depth > 0; depth--) {
    // 只有光标位于当前文本块以及所有祖先容器的开端时，才允许向外寻找上一个文档块。
    if (depth < $from.depth && $from.index(depth) !== 0) {
      return null;
    }

    const currentPos = $from.before(depth);
    const nodeBefore = $from.doc.resolve(currentPos).nodeBefore;
    if (nodeBefore && EDITABLE_BLOCK_NAMES.has(nodeBefore.type.name)) {
      return { node: nodeBefore, pos: currentPos - nodeBefore.nodeSize };
    }
    if (nodeBefore) return null;
  }

  return null;
}

export function codeBlockNavigationPlugin(callback: CodeBlockNavCallback): Plugin {
  return new Plugin({
    key: codeBlockNavigationKey,
    props: {
      handleKeyDown(_view, event) {
        if (_view.composing || event.isComposing || event.keyCode === 229) return false;
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return false;

        const view = _view;
        const state = view.state;
        const { selection } = state;
        callback.prepareMathKeyboardEntry?.(event.key === 'ArrowDown' ? 'start' : 'end');

        // 代码块已选中（NodeSelection）→ 进入编辑态
        if (selection instanceof NodeSelection && selection.node.type.name === 'code_block') {
          event.preventDefault();
          if (event.key === 'ArrowDown') {
            callback.enterEditAt(view, selection.from, 0, 'end');
          } else {
            // ArrowUp：光标在末行末尾
            const code = selection.node.textContent;
            const lastLine = code.split('\n').length - 1;
            callback.enterEditAt(view, selection.from, lastLine, 'end');
          }
          return true;
        }
        if (selection instanceof NodeSelection && selection.node.type.name === 'math_block') {
          if (!callback.enterMathEditAt) return false;
          event.preventDefault();
          callback.enterMathEditAt(
            view,
            selection.from,
            event.key === 'ArrowDown' ? 'start' : 'end',
          );
          return true;
        }
        if (selection instanceof NodeSelection && selection.node.type.name === 'mermaid_block') {
          if (!callback.enterMermaidEditAt) return false;
          event.preventDefault();
          callback.enterMermaidEditAt(
            view,
            selection.from,
            event.key === 'ArrowDown' ? 'start' : 'end',
          );
          return true;
        }

        // 光标在文本块边界且紧邻代码块 → 直接进入代码块编辑态
        const { $from, empty } = selection;
        if (!empty) return false;
        if (!$from.parent.isTextblock) return false;

        if (event.key === 'ArrowDown') {
          const nextBlock = findNextEditableBlock($from, isOnLastVisualLine(view, $from));
          if (nextBlock) {
            event.preventDefault();
            if (nextBlock.node.type.name === 'math_block') {
              callback.enterMathEditAt?.(view, nextBlock.pos, 'start');
            } else if (nextBlock.node.type.name === 'mermaid_block') {
              callback.enterMermaidEditAt?.(view, nextBlock.pos, 'start');
            } else {
              callback.enterEditAt(view, nextBlock.pos, 0, 'start');
            }
            return true;
          }
        } else {
          const previousBlock = findPreviousEditableBlock($from, isOnFirstVisualLine(view, $from));
          if (previousBlock) {
            event.preventDefault();
            if (previousBlock.node.type.name === 'math_block') {
              callback.enterMathEditAt?.(view, previousBlock.pos, 'end');
            } else if (previousBlock.node.type.name === 'mermaid_block') {
              callback.enterMermaidEditAt?.(view, previousBlock.pos, 'end');
            } else {
              const lastLine = previousBlock.node.textContent.split('\n').length - 1;
              callback.enterEditAt(view, previousBlock.pos, lastLine, 'end');
            }
            return true;
          }
        }

        return false;
      },
    },
  });
}
