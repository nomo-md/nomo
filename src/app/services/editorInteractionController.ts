import { tick } from 'svelte';
import type { EditorCommand, EditorCore, EditorMode } from '../../lib/editor-core';
import type { OutlineItem } from '../../lib/outline/outlineService';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import { createTocBlock } from '../../lib/toc/tocService';
import { t } from '../i18n';
import {
  type OutlineScrollAnchor,
  getSemanticScrollAnchorForBlock,
  getSourceScrollAnchor,
  scrollSemanticToAnchor,
  scrollSourceToAnchor,
  setScrollTop,
} from './outlineNavigation';

interface EditorInteractionOptions {
  getEditor(): EditorCore;
  getLargeDocumentMode(): boolean;
  getMode(): EditorMode;
  getSplitView?(): boolean;
  getOutline(): OutlineItem[];
  getSemanticPane(): HTMLElement | undefined;
  getSourcePane(): HTMLElement | undefined;
  getSourceEditor(): MarkdownSourceEditorHandle | undefined;
  getPendingSourceScrollTop(): number | null;
  setPendingSourceScrollTop(value: number | null): void;
  suppressSourceLayoutScroll?(): void;
  setSuppressOutlineScrollUntil(value: number): void;
  setStatusMessage(value: string): void;
  getSourceLineHeight(): number;
}

interface EditorModeSwitchResult {
  status: 'ready' | 'superseded';
  generation: number;
  alignmentStatus?: 'aligned' | 'degraded' | 'skipped';
}

export function createEditorInteractionController(options: EditorInteractionOptions) {
  let pendingSourceCaretLine: number | null = null;
  let modeSwitchGeneration = 0;
  let viewportMeasureQueued = false;
  let pendingViewportRestore: number | null = null;

  async function setMode(
    nextMode: EditorMode,
    modeSwitchAnchor?: OutlineScrollAnchor | null,
    force = false,
    targetViewMode?: EditorMode | 'split',
  ): Promise<EditorModeSwitchResult> {
    const generation = ++modeSwitchGeneration;
    if (options.getLargeDocumentMode() && nextMode === 'semantic') {
      options.setStatusMessage(t.largeDocumentStayReadonlySource());
      return { status: 'superseded', generation };
    }
    if (!force && nextMode === options.getMode()) {
      return { status: 'ready', generation };
    }

    // 在切换模式前保存两个面板的滚动位置。
    // 浏览器会将 display:none 的元素 scrollTop 重置为 0，
    // 因此必须在 CSS 生效前捕获当前值。
    const outline = options.getOutline();
    const semanticPane = options.getSemanticPane();
    const sourcePane = options.getSourcePane();
    const savedSemanticScrollTop = semanticPane?.scrollTop ?? 0;
    const savedSourceScrollTop = sourcePane?.scrollTop ?? 0;
    const scrollAnchor =
      modeSwitchAnchor ??
      (options.getMode() === 'semantic'
        ? getSemanticModeSwitchAnchor(outline, semanticPane, savedSemanticScrollTop)
        : getSourceModeSwitchAnchor(outline, sourcePane, savedSourceScrollTop));
    options.getEditor().updateOptions({ mode: nextMode });
    await tick();
    if (!targetViewMode) {
      options.setSuppressOutlineScrollUntil(Date.now() + 300);
      scheduleAfterFrames(() => {
        if (generation !== modeSwitchGeneration) return;
        if (nextMode === 'semantic') {
          scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane(), scrollAnchor, {
            behavior: 'instant',
          });
          refreshEditorViewportLayout();
          return;
        }
        void restoreSourceScrollAnchorWhenReady(scrollAnchor, generation);
      }, 2);
      return { status: 'ready', generation };
    }
    const alignmentStatus = await waitForPaneGeometry(targetViewMode, generation);
    if (generation !== modeSwitchGeneration) {
      return { status: 'superseded', generation };
    }
    options.setSuppressOutlineScrollUntil(Date.now() + 300);

    if (nextMode === 'semantic') {
      scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane(), scrollAnchor, {
        behavior: 'instant',
      });
      measureEditorViewportLayout(null);
      await waitForAnimationFrames(1);
      if (generation !== modeSwitchGeneration) {
        return { status: 'superseded', generation };
      }
      scrollSemanticToAnchor(options.getOutline(), options.getSemanticPane(), scrollAnchor, {
        behavior: 'instant',
      });
    } else {
      await restoreSourceScrollAnchorWhenReady(scrollAnchor, generation);
    }

    await waitForAnimationFrames(1);
    return {
      status: generation === modeSwitchGeneration ? 'ready' : 'superseded',
      generation,
      ...(targetViewMode === 'split' ? { alignmentStatus } : {}),
    };
  }

  function updateMarkdown(markdown: string) {
    const sourceEditor = options.getSourceEditor();
    options.setPendingSourceScrollTop(options.getSplitView?.() ? null : options.getSourcePane()?.scrollTop ?? null);
    pendingSourceCaretLine = getSourceSelectionLine(sourceEditor);
    options.getEditor().setMarkdown(markdown, {
      reason: 'source-input',
      sourceInput: true,
    });
    syncSourceTextareaHeight(options.getPendingSourceScrollTop());
  }

  function runCommand(command: EditorCommand) {
    if (options.getMode() === 'source' && (command.type === 'undo' || command.type === 'redo')) {
      const sourceEditor = options.getSourceEditor();
      sourceEditor?.focus();
      if (command.type === 'undo') sourceEditor?.undo();
      else sourceEditor?.redo();
      return;
    }
    if (command.type === 'insertToc' && options.getMode() === 'source') {
      insertTocAtSourceSelection();
      return;
    }

    options.getEditor().execute(command);
    options.getEditor().focus();
  }

  function insertTocAtSourceSelection() {
    const sourceEditor = options.getSourceEditor();
    const markdown = options.getEditor().getMarkdown();
    const selection = sourceEditor?.getSelection();
    const start = selection?.from ?? markdown.length;
    const end = selection?.to ?? start;
    const tocBlock = createTocBlock(markdown);
    const prefix = markdown.slice(0, start);
    const suffix = markdown.slice(end);
    const before = prefix.endsWith('\n') || prefix.length === 0 ? '' : '\n\n';
    const after = suffix.startsWith('\n') || suffix.length === 0 ? '' : '\n\n';
    const nextMarkdown = `${prefix}${before}${tocBlock}${after}${suffix}`;
    const nextSelection = prefix.length + before.length + tocBlock.length;

    sourceEditor?.setMarkdown(nextMarkdown, { addToHistory: true });
    requestAnimationFrame(() => {
      if (!sourceEditor) {
        return;
      }
      sourceEditor.focus();
      sourceEditor.setSelection(nextSelection);
      syncSourceTextareaHeight();
    });
  }

  function syncSourceTextareaHeight(
    restoreScrollTop: number | null = options.getPendingSourceScrollTop(),
  ) {
    scheduleViewportMeasure(restoreScrollTop);
  }

  function refreshEditorViewportLayout() {
    scheduleViewportMeasure(null);
  }

  async function restoreSourceScrollAnchorWhenReady(
    scrollAnchor: OutlineScrollAnchor | null,
    generation: number,
  ) {
    for (let attempt = 0; attempt < 6; attempt += 1) {
      if (generation !== modeSwitchGeneration) return;
      measureEditorViewportLayout(null);
      const sourcePane = options.getSourcePane();
      if (sourcePane && isPaneLayoutVisible(sourcePane)) {
        scrollSourceToAnchor(
          options.getOutline(),
          sourcePane,
          options.getSourceEditor(),
          scrollAnchor,
        );
        await waitForAnimationFrames(1);
        if (generation !== modeSwitchGeneration) return;
        measureEditorViewportLayout(null);
        scrollSourceToAnchor(
          options.getOutline(),
          sourcePane,
          options.getSourceEditor(),
          scrollAnchor,
        );
        return;
      }
      await waitForAnimationFrames(1);
    }
  }

  function measureEditorViewportLayout(restoreScrollTop: number | null) {
    const semanticPane = options.getSemanticPane();
    if (semanticPane && isPaneLayoutVisible(semanticPane)) {
      semanticPane.scrollLeft = 0;
      clampPaneScrollTop(semanticPane);
    }

    const sourcePane = options.getSourcePane();
    if (!isPaneLayoutVisible(sourcePane)) {
      return;
    }
    const scrollTopBeforeMeasure = sourcePane?.scrollTop ?? 0;

    const sourceEditor = options.getSourceEditor();
    options.suppressSourceLayoutScroll?.();
    getEditorGrid()?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));

    if (restoreScrollTop !== null && sourcePane) {
      const nextScrollTop = getSourceScrollTopWithVisibleCaret(
        sourcePane,
        sourceEditor,
        restoreScrollTop,
        pendingSourceCaretLine,
      );
      clampPaneScrollTop(sourcePane, nextScrollTop);
      options.setPendingSourceScrollTop(null);
      pendingSourceCaretLine = null;
    } else {
      clampPaneScrollTop(sourcePane, scrollTopBeforeMeasure);
    }
  }

  function scheduleViewportMeasure(restoreScrollTop: number | null) {
    if (restoreScrollTop !== null) pendingViewportRestore = restoreScrollTop;
    if (viewportMeasureQueued) return;
    viewportMeasureQueued = true;
    const raf = getRequestAnimationFrame();
    raf(() => {
      viewportMeasureQueued = false;
      const restore = pendingViewportRestore;
      pendingViewportRestore = null;
      measureEditorViewportLayout(restore);
    });
  }

  function waitForAnimationFrames(frameCount = 1) {
    const raf = getRequestAnimationFrame();
    return new Promise<void>((resolve) => {
      const run = (remainingFrames: number) => {
        raf(() => {
          if (remainingFrames <= 1) {
            resolve();
            return;
          }
          run(remainingFrames - 1);
        });
      };
      run(Math.max(1, frameCount));
    });
  }

  function scheduleAfterFrames(callback: () => void, frameCount = 1) {
    void waitForAnimationFrames(frameCount).then(callback);
  }

  function getEditorGrid() {
    return (
      options.getSourcePane()?.closest<HTMLElement>('.editor-grid') ??
      options.getSemanticPane()?.closest<HTMLElement>('.editor-grid') ??
      null
    );
  }

  function waitForPaneGeometry(targetViewMode: EditorMode | 'split', generation: number) {
    const editorGrid = getEditorGrid();
    if (!editorGrid) {
      return waitForAnimationFrames(2).then(() =>
        targetViewMode === 'split' ? ('degraded' as const) : ('aligned' as const),
      );
    }

    return new Promise<'aligned' | 'degraded' | 'skipped'>((resolve) => {
      let settled = false;
      let fallbackFrames = 0;
      const readyEventName = 'nomo:editor-pane-geometry-ready';
      const finish = (status: 'aligned' | 'degraded' | 'skipped') => {
        if (settled) return;
        settled = true;
        editorGrid.removeEventListener(readyEventName, handleGeometryReady as EventListener);
        resolve(status);
      };
      const handleGeometryReady = (event: Event) => {
        const detail = (
          event as CustomEvent<{
            mode?: string;
            status?: 'aligned' | 'degraded' | 'skipped';
          }>
        ).detail;
        if (detail?.mode !== targetViewMode) return;
        finish('aligned');
      };
      const waitForFallback = () => {
        getRequestAnimationFrame()(() => {
          fallbackFrames += 1;
          if (generation !== modeSwitchGeneration) {
            finish('degraded');
            return;
          }
          if (fallbackFrames >= 18) {
            editorGrid.dispatchEvent(
              new CustomEvent('nomo:editor-viewport-layout-refresh', {
                detail: { synchronous: true },
              }),
            );
            finish('degraded');
            return;
          }
          waitForFallback();
        });
      };

      editorGrid.addEventListener(readyEventName, handleGeometryReady as EventListener);
      editorGrid.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
      waitForFallback();
    });
  }

  function clampPaneScrollTop(pane: HTMLElement | undefined, preferredScrollTop?: number) {
    if (!pane) {
      return;
    }
    const maxScrollTop = Math.max(0, pane.scrollHeight - pane.clientHeight);
    const nextScrollTop = Math.min(
      maxScrollTop,
      Math.max(0, preferredScrollTop ?? pane.scrollTop),
    );
    if (pane.scrollTop !== nextScrollTop) {
      setScrollTop(pane, nextScrollTop);
    }
  }

  function getSourceScrollTopWithVisibleCaret(
    sourcePane: HTMLElement,
    sourceEditor: MarkdownSourceEditorHandle | undefined,
    preferredScrollTop: number,
    caretLine: number | null,
  ) {
    if (!sourceEditor || caretLine == null) {
      return preferredScrollTop;
    }

    const lineHeight = options.getSourceLineHeight();
    const caretTop = Math.max(0, caretLine - 1) * lineHeight;
    const caretBottom = caretTop + lineHeight;
    const visibleTop = preferredScrollTop;
    const visibleBottom = preferredScrollTop + sourcePane.clientHeight;

    if (caretBottom > visibleBottom) {
      return caretBottom - sourcePane.clientHeight + lineHeight;
    }
    if (caretTop < visibleTop) {
      return Math.max(0, caretTop - lineHeight);
    }
    return preferredScrollTop;
  }

  function isPaneLayoutVisible(pane: HTMLElement | undefined) {
    return Boolean(pane && pane.getClientRects().length > 0);
  }

  function getSemanticModeSwitchAnchor(
    outline: OutlineItem[],
    semanticPane: HTMLElement | undefined,
    savedScrollTop: number,
  ): OutlineScrollAnchor | null {
    if (!semanticPane) {
      return null;
    }

    const selectionRect = options.getEditor().getSelectionAnchorRect();
    const anchorBlock = isRectVisibleInPane(selectionRect, semanticPane)
      ? findSemanticBlockAtRect(semanticPane, selectionRect)
      : findFirstVisibleSemanticBlock(semanticPane);

    return getSemanticScrollAnchorForBlock(
      outline,
      semanticPane,
      anchorBlock,
      savedScrollTop,
    );
  }

  function getSourceModeSwitchAnchor(
    outline: OutlineItem[],
    sourcePane: HTMLElement | undefined,
    savedScrollTop: number,
  ): OutlineScrollAnchor | null {
    const sourceEditor = options.getSourceEditor();
    const lineHeight = options.getSourceLineHeight();
    return getSourceScrollAnchor(
      outline,
      savedScrollTop,
      lineHeight,
      sourceEditor,
      sourcePane,
    );
  }

  function isRectVisibleInPane(rect: DOMRect | null, pane: HTMLElement) {
    if (!rect) {
      return false;
    }
    const paneRect = pane.getBoundingClientRect();
    if (rect.height > paneRect.height * 1.5) {
      return false;
    }
    return rect.bottom > paneRect.top && rect.top < paneRect.bottom;
  }

  function findSemanticBlockAtRect(pane: HTMLElement, rect: DOMRect | null) {
    if (!rect || typeof document === 'undefined') {
      return null;
    }
    const paneRect = pane.getBoundingClientRect();
    const x = clamp(rect.left + rect.width / 2, paneRect.left + 1, paneRect.right - 1);
    const y = clamp(rect.top + rect.height / 2, paneRect.top + 1, paneRect.bottom - 1);
    const element = document.elementFromPoint(x, y);
    const editor = pane.querySelector<HTMLElement>('.ProseMirror');
    if (!editor || !(element instanceof HTMLElement) || !editor.contains(element)) {
      return findVisibleSemanticBlockNearY(pane, y);
    }

    let current: HTMLElement | null = element;
    while (current && current.parentElement !== editor) {
      current = current.parentElement;
    }
    return current ?? findVisibleSemanticBlockNearY(pane, y);
  }

  function findFirstVisibleSemanticBlock(pane: HTMLElement) {
    const paneRect = pane.getBoundingClientRect();
    return getSemanticBlocks(pane).find((block) => {
      const rect = block.getBoundingClientRect();
      return rect.bottom > paneRect.top && rect.top < paneRect.bottom;
    });
  }

  function findVisibleSemanticBlockNearY(pane: HTMLElement, y: number) {
    return getSemanticBlocks(pane).find((block) => {
      const rect = block.getBoundingClientRect();
      return rect.bottom >= y && rect.top <= y;
    });
  }

  function getSemanticBlocks(pane: HTMLElement) {
    return Array.from(pane.querySelectorAll<HTMLElement>('.ProseMirror > *'));
  }

  function getSourceSelectionLine(sourceEditor: MarkdownSourceEditorHandle | undefined) {
    if (!sourceEditor) {
      return null;
    }
    return sourceEditor.lineAtOffset(sourceEditor.getSelection().from);
  }

  function clamp(value: number, min: number, max: number) {
    return Math.min(max, Math.max(min, value));
  }

  function getRequestAnimationFrame() {
    if (typeof window === 'undefined' || typeof window.requestAnimationFrame !== 'function') {
      return (callback: FrameRequestCallback) => {
        callback(Date.now());
        return 0;
      };
    }
    return window.requestAnimationFrame.bind(window);
  }

  return {
    setMode,
    updateMarkdown,
    runCommand,
    syncSourceTextareaHeight,
    refreshEditorViewportLayout,
  };
}
