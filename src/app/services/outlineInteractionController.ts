import type { EditorCore, EditorMode } from '../../lib/editor-core';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import {
  reorderOutlineSection,
  type OutlineMoveFailureReason,
  type OutlineSectionMoveRequest,
} from '../../lib/outline/outlineReorder';
import type { OutlineItem } from '../../lib/outline/outlineService';
import { t } from '../i18n';
import {
  getActiveOutlineIdFromSemantic,
  getActiveOutlineIdFromSource,
  getSourceHeadingSelection,
  getSourceLineHeight as getTextareaLineHeight,
  scrollSemanticToAnchor,
  setScrollTop,
  smoothScrollElementTo,
} from './outlineNavigation';
import {
  isOutlineItemExpandable as getOutlineItemExpandable,
  pruneCollapsedOutlineIds as getPrunedCollapsedOutlineIds,
  toggleOutlineItemExpanded as getToggledOutlineItemIds,
} from './outlineState';

interface OutlineInteractionOptions {
  getMode(): EditorMode;
  getMarkdown(): string;
  getOutline(): OutlineItem[];
  getCollapsedOutlineIds(): Set<string>;
  setCollapsedOutlineIds(value: Set<string>): void;
  getOutlineVisible(): boolean;
  setOutlineVisible(value: boolean): void;
  setActiveOutlineId(value: string): void;
  getSuppressOutlineScrollUntil(): number;
  setSuppressOutlineScrollUntil(value: number): void;
  getSemanticPane(): HTMLElement;
  getSourcePane(): HTMLElement;
  getSourceEditor(): MarkdownSourceEditorHandle;
  getEditor(): EditorCore;
  getReadonly(): boolean;
  setStatusMessage(value: string): void;
  onExplicitJumpIntent?(): void;
  /** 文档视图已挂起或替换时，取消其延迟定位，避免操作其他文档。 */
  isActive?(): boolean;
}

export function createOutlineInteractionController(options: OutlineInteractionOptions) {
  let outlineMoveGeneration = 0;

  function toggleOutlineVisible() {
    options.setOutlineVisible(!options.getOutlineVisible());
  }

  function isOutlineItemExpandable(index: number) {
    return getOutlineItemExpandable(options.getOutline(), index);
  }

  function toggleOutlineItemExpanded(item: OutlineItem) {
    options.setCollapsedOutlineIds(
      getToggledOutlineItemIds(options.getCollapsedOutlineIds(), item),
    );
  }

  function expandAllOutline() {
    options.setCollapsedOutlineIds(new Set());
  }

  function collapseAllOutline() {
    const outline = options.getOutline();
    options.setCollapsedOutlineIds(
      new Set(
        outline
          .filter((_item, index) => getOutlineItemExpandable(outline, index))
          .map((item) => item.id),
      ),
    );
  }

  function pruneCollapsedOutlineIds() {
    options.setCollapsedOutlineIds(
      getPrunedCollapsedOutlineIds(options.getOutline(), options.getCollapsedOutlineIds()),
    );
  }

  function jumpToOutlineItem(item: OutlineItem) {
    if (options.isActive?.() === false) return;
    options.setActiveOutlineId(item.id);
    options.setSuppressOutlineScrollUntil(Date.now() + 800);
    options.onExplicitJumpIntent?.();

    requestAnimationFrame(() => {
      if (options.isActive?.() === false) return;
      if (options.getMode() === 'semantic') {
        scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane(), {
          outlineId: item.id,
          sectionProgress: 0,
        });
        return;
      }

      const sourceEditor = options.getSourceEditor();
      const sourcePane = options.getSourcePane();
      const selection = getSourceHeadingSelection(options.getMarkdown(), item);
      const restoreScrollTop = sourcePane?.scrollTop ?? 0;

      sourceEditor.focus({ preventScroll: true });
      sourceEditor.setSelection(selection.end);
      if (sourcePane) {
        setScrollTop(sourcePane, restoreScrollTop);
        smoothScrollElementTo(sourcePane, Math.max(0, sourceEditor.getLineTop(item.line) - 40));
      }
    });
  }

  function moveOutlineSection(request: OutlineSectionMoveRequest): boolean {
    if (options.isActive?.() === false) return false;
    if (options.getReadonly()) {
      options.setStatusMessage(t.outlineMoveReadonly());
      return false;
    }

    const previousOutline = options.getOutline();
    const previousCollapsedOutlineIds = new Set(options.getCollapsedOutlineIds());
    const result = reorderOutlineSection(options.getMarkdown(), previousOutline, request);
    if (!result.ok) {
      options.setStatusMessage(getOutlineMoveFailureMessage(result.reason));
      return false;
    }

    if (options.getMode() === 'source') {
      options.getSourceEditor().setMarkdown(result.markdown, { addToHistory: true });
      const generation = ++outlineMoveGeneration;
      options.setStatusMessage(
        t.outlineSectionMoved({ title: previousOutline[request.sourceIndex].title }),
      );
      window.setTimeout(() => {
        if (generation !== outlineMoveGeneration || options.isActive?.() === false) return;
        restoreOutlineMoveState(
          previousOutline,
          previousCollapsedOutlineIds,
          options.getOutline(),
          result.indexMap,
          result.movedHeadingIndex,
          true,
        );
      }, 150);
      return true;
    } else {
      const moved = options.getEditor().execute({ type: 'moveOutlineSection', ...request });
      if (!moved) {
        options.setStatusMessage(t.outlineMoveFailed());
        return false;
      }
      // 章节移动必须让正文与大纲同一拍更新，不能等待常规输入事务的延迟序列化。
      options.getEditor().getMarkdown();
    }

    outlineMoveGeneration += 1;
    restoreOutlineMoveState(
      previousOutline,
      previousCollapsedOutlineIds,
      options.getOutline(),
      result.indexMap,
      result.movedHeadingIndex,
      false,
    );
    return true;
  }

  function restoreOutlineMoveState(
    previousOutline: readonly OutlineItem[],
    previousCollapsedOutlineIds: ReadonlySet<string>,
    nextOutline: readonly OutlineItem[],
    indexMap: readonly number[],
    movedHeadingIndex: number,
    jumpInSource: boolean,
  ) {
    if (options.isActive?.() === false) return;
    const nextCollapsedIds = new Set<string>();
    previousOutline.forEach((item, originalIndex) => {
      if (!previousCollapsedOutlineIds.has(item.id)) return;
      const nextItem = nextOutline[indexMap[originalIndex]];
      if (nextItem) nextCollapsedIds.add(nextItem.id);
    });
    options.setCollapsedOutlineIds(nextCollapsedIds);
    const movedItem = nextOutline[movedHeadingIndex];
    if (!movedItem) return;
    options.setActiveOutlineId(movedItem.id);
    options.setSuppressOutlineScrollUntil(Date.now() + 800);
    if (jumpInSource) requestAnimationFrame(() => jumpToOutlineItem(movedItem));
    options.setStatusMessage(t.outlineSectionMoved({ title: movedItem.title }));
  }

  function updateActiveOutlineFromSourceScroll() {
    if (options.isActive?.() === false) return;
    if (Date.now() < options.getSuppressOutlineScrollUntil()) {
      return;
    }
    const sourcePane = options.getSourcePane();
    if (!sourcePane) {
      options.setActiveOutlineId('');
      return;
    }
    options.setActiveOutlineId(
      getActiveOutlineIdFromSource(
        options.getOutline(),
        sourcePane.scrollTop,
        getSourceLineHeight(),
        options.getSourceEditor(),
        sourcePane,
      ),
    );
  }

  function updateActiveOutlineFromSemanticScroll() {
    if (options.isActive?.() === false) return;
    if (Date.now() < options.getSuppressOutlineScrollUntil()) {
      return;
    }
    options.setActiveOutlineId(
      getActiveOutlineIdFromSemantic(options.getOutline(), options.getSemanticPane()),
    );
  }

  function getSourceLineHeight() {
    return getTextareaLineHeight(options.getSourceEditor());
  }

  return {
    toggleOutlineVisible,
    isOutlineItemExpandable,
    toggleOutlineItemExpanded,
    expandAllOutline,
    collapseAllOutline,
    pruneCollapsedOutlineIds,
    jumpToOutlineItem,
    moveOutlineSection,
    updateActiveOutlineFromSourceScroll,
    updateActiveOutlineFromSemanticScroll,
    getSourceLineHeight,
  };
}

function getOutlineMoveFailureMessage(reason: OutlineMoveFailureReason): string {
  switch (reason) {
    case 'self-or-descendant':
      return t.outlineMoveSelfOrDescendant();
    case 'heading-level-overflow':
      return t.outlineMoveLevelOverflow();
    case 'no-change':
      return t.outlineMoveNoChange();
    default:
      return t.outlineMoveFailed();
  }
}
