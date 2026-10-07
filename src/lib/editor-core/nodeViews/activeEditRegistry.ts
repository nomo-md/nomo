/**
 * 跨 NodeView 的编辑态协调注册表
 *
 * 行内代码、行公式等使用 <input> 编辑态的 NodeView 共享此注册表，
 * 每篇文档分别协调编辑态，组合标签的另一篇文档不会被误提交。
 */

import type { EditorView } from 'prosemirror-view';

type ExitEditFn = () => void;

interface ActiveEdit {
  exit: ExitEditFn;
  commit?: () => void;
}

const activeEdits = new WeakMap<EditorView, ActiveEdit>();

/** 注册退出回调；支持原位提交的编辑器可额外提供不结束编辑态的提交回调。 */
export function registerActiveEdit(
  view: EditorView,
  exitFn: ExitEditFn,
  commitFn?: () => void,
): void {
  // 切换编辑对象时仍然结束上一个编辑态。
  const previous = activeEdits.get(view);
  if (previous && previous.exit !== exitFn) {
    previous.exit();
  }
  activeEdits.set(view, { exit: exitFn, commit: commitFn });
}

/** 注销（退出编辑态时调用） */
export function unregisterActiveEdit(view: EditorView, exitFn: ExitEditFn): void {
  if (activeEdits.get(view)?.exit === exitFn) {
    activeEdits.delete(view);
  }
}

/** 保存或快照只提交临时输入；切换模式时可明确要求结束编辑态。 */
export function commitActiveEdit(view: EditorView, exitEditing = false): void {
  const active = activeEdits.get(view);
  if (!active) return;
  if (!exitEditing && active.commit) {
    active.commit();
    return;
  }
  activeEdits.delete(view);
  active.exit();
}
