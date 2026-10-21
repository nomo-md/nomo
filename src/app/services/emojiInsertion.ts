import type { EditorCore, EditorMode, EditorSelectionSnapshot } from '../../lib/editor-core';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import type { EditorViewMode, SplitActivePane } from '../types';
import type { DocumentSelection } from '../../lib/markdown-document/protocol';
import { getMarkdownApplicationSession } from './markdownApplicationSession';

export interface EmojiInsertionTarget {
  globalSelection?: DocumentSelection;
  tabId: string;
  editor: EditorCore;
  source: MarkdownSourceEditorHandle | undefined;
  mode: EditorMode;
  viewMode: EditorViewMode;
  pane: SplitActivePane;
  selection: EditorSelectionSnapshot;
  contentRevision: number;
}

export interface EmojiInsertionContext {
  tabId: string;
  editor: EditorCore;
  source: MarkdownSourceEditorHandle | undefined;
  viewMode: EditorViewMode;
  pane: SplitActivePane;
  readonly: boolean;
  disabled: boolean;
  runtimeAvailable: boolean;
}

/** 选择器异步加载期间，文档身份、当前编辑侧和正文修订必须仍然匹配。 */
export function isEmojiTargetCurrent(
  target: EmojiInsertionTarget,
  current: EmojiInsertionContext,
): boolean {
  return (
    current.runtimeAvailable &&
    !current.readonly &&
    !current.disabled &&
    current.tabId === target.tabId &&
    current.editor === target.editor &&
    current.viewMode === target.viewMode &&
    current.pane === target.pane &&
    (target.mode !== 'source' || current.source === target.source)
  );
}

export function insertEmojiAtTarget(
  target: EmojiInsertionTarget,
  current: EmojiInsertionContext,
  text: string,
): boolean {
  if (!isEmojiTargetCurrent(target, current)) return false;
  const application = getMarkdownApplicationSession(target.editor);
  if (application && target.globalSelection) return application.insertAt(target.globalSelection, target.contentRevision, text);
  if (target.mode === 'source') {
    const snapshot = target.source?.getSelectionStatsSnapshot?.();
    if (snapshot && snapshot.documentId !== target.tabId) return false;
    const { anchor, head } = target.selection;
    if (
      !target.source?.replaceRange?.(
        Math.min(anchor, head),
        Math.max(anchor, head),
        text,
        target.contentRevision,
      )
    )
      return false;
    target.source.focus();
  } else {
    const snapshot = target.editor.getSnapshot();
    if ((snapshot.contentRevision ?? snapshot.version) !== target.contentRevision) return false;
    target.editor.restoreSelectionSnapshot(target.selection);
    if (!target.editor.execute({ type: 'insertText', text })) return false;
    target.editor.focus();
  }
  return true;
}
