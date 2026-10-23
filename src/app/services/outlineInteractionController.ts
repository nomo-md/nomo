import type { EditorCore, EditorMode } from '../../lib/editor-core';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import {
  reorderOutlineSection,
  type OutlineMoveFailureReason,
  type OutlineSectionMoveRequest,
} from '../../lib/outline/outlineReorder';
import type { OutlineItem } from '../../lib/outline/outlineService';
import type { OutlineProjection } from './outlineProjection';
import { t } from '../i18n';
import { getMarkdownApplicationSession } from './markdownApplicationSession';
import { getOutlineViewportOffset } from '../../lib/outline/outlineViewport';
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
  getOutline(): readonly OutlineItem[];
  getOutlineProjection?(): OutlineProjection | undefined;
  getCollapsedOutlineIds(): Set<string>;
  setCollapsedOutlineIds(value: Set<string>): void;
  getOutlineVisible(): boolean;
  setOutlineVisible(value: boolean): void;
  setActiveOutlineId(value: string): void;
  getSuppressOutlineScrollUntil(): number;
  setSuppressOutlineScrollUntil(value: number): void;
  getSemanticPane(): HTMLElement;
  getSourcePane(): HTMLElement;
  getSourceEditor(): MarkdownSourceEditorHandle | undefined;
  getEditor(): EditorCore;
  getReadonly(): boolean;
  setStatusMessage(value: string): void;
  onExplicitJumpIntent?(): void;
  /** 文档视图已挂起或替换时，取消其延迟定位，避免操作其他文档。 */
  isActive?(): boolean;
}

export function createOutlineInteractionController(options: OutlineInteractionOptions) {
  let outlineMoveGeneration = 0;
  let outlineJumpGeneration = 0;

  function toggleOutlineVisible() {
    options.setOutlineVisible(!options.getOutlineVisible());
  }

  function isOutlineItemExpandable(index: number) {
    return getOutlineItemExpandable(options.getOutline(), index);
  }

  function toggleOutlineItemExpanded(item: OutlineItem) {
    const projection = options.getOutlineProjection?.();
    if (projection) {
      projection.toggle(item);
      options.setCollapsedOutlineIds(projection.collapsedIds);
      return;
    }
    options.setCollapsedOutlineIds(
      getToggledOutlineItemIds(options.getCollapsedOutlineIds(), item),
    );
  }

  function expandAllOutline() {
    options.getOutlineProjection?.()?.setDefaultLevel(7);
    options.setCollapsedOutlineIds(new Set());
  }

  function collapseAllOutline() {
    const projection = options.getOutlineProjection?.();
    if (projection) {
      projection.setDefaultLevel(1);
      options.setCollapsedOutlineIds(projection.collapsedIds);
      return;
    }
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
    if (options.getOutlineProjection?.()) return;
    options.setCollapsedOutlineIds(
      getPrunedCollapsedOutlineIds(options.getOutline(), options.getCollapsedOutlineIds()),
    );
  }

  function jumpToOutlineItem(item: OutlineItem) {
    if (options.isActive?.() === false) return;
    const editor = options.getEditor();
    const generation = ++outlineJumpGeneration;
    options.onExplicitJumpIntent?.();
    const application = getMarkdownApplicationSession(editor);
    if (application) {
      const heading = application.document?.state.index?.outline.find(heading => heading.id === item.id);
      if (heading) {
        options.setActiveOutlineId(item.id);
        options.setSuppressOutlineScrollUntil(0);
        void application.revealOutlineHeading(heading.byte).then(() => {
          if (generation === outlineJumpGeneration && options.getEditor() === editor && options.isActive?.() !== false)
            updateSessionOutline();
        }).catch(error => {
          if (generation === outlineJumpGeneration && options.getEditor() === editor && options.isActive?.() !== false)
            options.setStatusMessage(String(error));
        });
      }
      return;
    }
    options.setActiveOutlineId(item.id);
    options.setSuppressOutlineScrollUntil(Date.now() + 800);

    requestAnimationFrame(() => {
      if (generation !== outlineJumpGeneration || options.getEditor() !== editor || options.isActive?.() === false) return;
      if (options.getMode() === 'semantic') {
        scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane(), {
          outlineId: item.id,
          sectionProgress: 0,
        }, { viewportOffset: getOutlineViewportOffset(options.getSemanticPane().clientHeight) });
        return;
      }

      const sourceEditor = options.getSourceEditor();
      if (!sourceEditor) return;
      const sourcePane = options.getSourcePane();
      const selection = getSourceHeadingSelection(options.getMarkdown(), item);
      const restoreScrollTop = sourcePane?.scrollTop ?? 0;

      sourceEditor.focus({ preventScroll: true });
      sourceEditor.setSelection(selection.end);
      if (sourcePane) {
        setScrollTop(sourcePane, restoreScrollTop);
        smoothScrollElementTo(sourcePane, Math.max(0, sourceEditor.getLineTop(item.line) - getOutlineViewportOffset(sourcePane.clientHeight)));
      }
    });
  }

  function moveOutlineSection(request: OutlineSectionMoveRequest): boolean {
    if (options.isActive?.() === false) return false;
    if (options.getReadonly()) {
      options.setStatusMessage(t.outlineMoveReadonly());
      return false;
    }
    const application = getMarkdownApplicationSession(options.getEditor());
    if (application) {
      const index = application.document?.state.index;
      if (!index || index.progress.state !== 'ready' || index.revision !== application.document?.state.status.revision) {
        options.setStatusMessage(t.outlineLoading());
        return false;
      }
      const revision = index.revision;
      const headings = index.outline;
      void application.transformMarkdown(text => {
        if (application.document?.state.status.revision !== revision || options.isActive?.() === false)
          throw new Error(t.outlineLoading());
        const result = reorderOutlineSection(text, headings, request);
        if (!result.ok) throw new Error(getOutlineMoveFailureMessage(result.reason));
        return result.markdown;
      }).catch(error => options.setStatusMessage(String(error)));
      return true;
    }

    const previousOutline = options.getOutline();
    const previousCollapsedOutlineIds = new Set(options.getCollapsedOutlineIds());
    const result = reorderOutlineSection(options.getMarkdown(), previousOutline, request);
    if (!result.ok) {
      options.setStatusMessage(getOutlineMoveFailureMessage(result.reason));
      return false;
    }

    if (options.getMode() === 'source') {
      const sourceEditor = options.getSourceEditor();
      if (!sourceEditor) return false;
      sourceEditor.setMarkdown(result.markdown, { addToHistory: true });
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
    if (updateSessionOutline()) return;
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
    if (updateSessionOutline()) return;
    options.setActiveOutlineId(
      getActiveOutlineIdFromSemantic(options.getOutline(), options.getSemanticPane()),
    );
  }

  function updateSessionOutline() {
    const application = getMarkdownApplicationSession(options.getEditor());
    if (!application) return false;
    options.setActiveOutlineId(application.activeOutlineId);
    return true;
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
