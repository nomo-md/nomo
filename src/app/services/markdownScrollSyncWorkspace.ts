import type { EditorCore } from '../../lib/editor-core';
import type { MarkdownSyncAnchor } from '../../lib/editor-core/scrollSyncMapping';
import { extractFrontMatterBlock } from '../../lib/markdown/frontMatter';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import type { EditorViewMode, SplitActivePane } from '../types';
import { createMarkdownScrollSync, type SyncPane } from './markdownScrollSync';

interface SyncWorkspaceParams {
  mode: EditorViewMode;
  documentId: string;
  markdown: string;
  sourceEditor: MarkdownSourceEditorHandle | undefined;
  editorCore: EditorCore;
  activePane: SplitActivePane;
  paused: boolean;
  largeDocumentMode: boolean;
}

/** 工作区只适配输入和坐标；跨栏写入全部交给同步控制器。 */
export function syncEditorPanes(node: HTMLElement, initial: SyncWorkspaceParams) {
  let params = initial;
  let snapshotKey = '';
  let contentMatch: {
    key: string;
    markdown: string;
    snapshotMarkdown: string;
    sourceMarkdown: string;
    matches: boolean;
  } | null = null;
  let anchors: readonly MarkdownSyncAnchor[] = [];
  let pointerPane: SyncPane | null = null;
  let geometryFrame = 0;
  let destroyed = false;
  const observed = new Set<Element>();
  const getElement = (pane: SyncPane) =>
    node.querySelector<HTMLElement>(
      pane === 'source' ? '.source-pane .cm-scroller' : '.semantic-pane',
    ) ?? undefined;
  const inScrollCoordinates = (pane: HTMLElement, y: number) => {
    const rect = pane.getBoundingClientRect();
    const scale = pane.clientHeight > 0 ? rect.height / pane.clientHeight : 1;
    return (y - rect.top) / (scale || 1) + pane.scrollTop;
  };
  const core = params.editorCore;
  const controller = createMarkdownScrollSync({
    snapshot() {
      const snapshot = params.editorCore.getScrollSyncSnapshot();
      const key = `${params.documentId}:${snapshot.revision}:${snapshot.renderRevision}`;
      const sourceMarkdown = params.sourceEditor?.getMarkdown();
      if (
        snapshot.ready &&
        sourceMarkdown !== undefined &&
        (!contentMatch ||
          contentMatch.key !== key ||
          contentMatch.markdown !== params.markdown ||
          contentMatch.snapshotMarkdown !== snapshot.markdown ||
          contentMatch.sourceMarkdown !== sourceMarkdown)
      ) {
        // CodeMirror 使用 LF，磁盘内容可能保留 CRLF；只规范化比较副本，不改正文。
        // 绑定修订及三方文本，源码单独追上时也重算，滚动帧不重复扫描全文。
        const normalized = snapshot.markdown.replace(/\r\n/g, '\n');
        contentMatch = {
          key,
          markdown: params.markdown,
          snapshotMarkdown: snapshot.markdown,
          sourceMarkdown,
          matches:
            params.markdown.replace(/\r\n/g, '\n') === normalized &&
            sourceMarkdown.replace(/\r\n/g, '\n') === normalized,
        };
      }
      const ready =
        snapshot.ready &&
        sourceMarkdown !== undefined &&
        contentMatch?.matches === true &&
        params.sourceEditor?.isLayoutReady?.() !== false;
      if (ready && key !== snapshotKey) {
        snapshotKey = key;
        anchors = snapshot.anchors;
        const frontMatter = extractFrontMatterBlock(snapshot.markdown);
        if (frontMatter) {
          const endLine = frontMatter.raw.trimEnd().split('\n').length + 1;
          anchors = [
            {
              key: 'front-matter:start',
              fromLine: 1,
              toLine: endLine - 1,
              pos: -1,
              endPos: -1,
              kind: 'front-matter',
              edge: 'start',
              depth: 0,
            },
            {
              key: 'front-matter:end',
              fromLine: endLine,
              toLine: endLine,
              pos: -1,
              endPos: -1,
              kind: 'front-matter',
              edge: 'end',
              depth: 0,
            },
            ...anchors,
          ];
        }
      }
      return { revision: key, ready, anchors };
    },
    pane(pane) {
      const element = getElement(pane);
      if (!element || !params.sourceEditor) return null;
      return {
        element,
        position(anchor) {
          if (pane === 'source') {
            const source = params.sourceEditor!;
            if (anchor.fromLine > source.getLineCount()) {
              return source.getLineTop(1) + source.getContentHeight();
            }
            return source.getLineTop(anchor.fromLine);
          }
          const rect =
            anchor.kind === 'front-matter'
              ? node
                  .querySelector<HTMLElement>('.semantic-pane .front-matter-card')
                  ?.getBoundingClientRect()
              : params.editorCore.getScrollSyncAnchorRect(anchor);
          if (!rect) return null;
          return inScrollCoordinates(element, anchor.edge === 'end' ? rect.bottom : rect.top);
        },
        caret(currentAnchors) {
          if (pane === 'source') return params.sourceEditor?.getSyncCaretTop?.() ?? null;
          const metadata = document.activeElement?.closest('.semantic-pane .front-matter-card');
          if (metadata) return inScrollCoordinates(element, metadata.getBoundingClientRect().top);
          const caret = params.editorCore.getScrollSyncCaret();
          if (!caret) return null;
          if (!caret.blockOnly && caret.viewportTop != null)
            return inScrollCoordinates(element, caret.viewportTop);
          const anchor = currentAnchors
            .filter(
              (item) =>
                item.edge === 'start' && item.pos <= caret.head && item.endPos >= caret.head,
            )
            .sort((a, b) => b.depth - a.depth)[0];
          const rect = anchor ? params.editorCore.getScrollSyncAnchorRect(anchor) : null;
          return rect ? inScrollCoordinates(element, rect.top) : null;
        },
      };
    },
    status(status, leader, revision) {
      if (!import.meta.env.DEV) return;
      node.dataset.syncStatus = status;
      node.dataset.syncLeader = leader;
      node.dataset.syncRevision = revision;
    },
  });

  function changed() {
    if (destroyed || geometryFrame) return;
    geometryFrame = requestAnimationFrame(() => {
      geometryFrame = 0;
      if (destroyed) return;
      const targets = [
        node,
        getElement('source'),
        getElement('semantic'),
        node.querySelector('.source-pane .cm-content'),
        node.querySelector('.semantic-pane .ProseMirror'),
      ].filter((target): target is Element => Boolean(target));
      for (const target of observed)
        if (!targets.includes(target)) {
          resize?.unobserve(target);
          observed.delete(target);
        }
      for (const target of targets)
        if (!observed.has(target)) {
          resize?.observe(target);
          observed.add(target);
        }
      controller.layoutChanged();
    });
  }
  const resize = typeof ResizeObserver === 'function' ? new ResizeObserver(changed) : null;
  const mutations =
    typeof MutationObserver === 'function'
      ? new MutationObserver((records) => {
          if (
            records.some(
              (record) =>
                record.target instanceof Element &&
                record.target.closest('.semantic-pane .ProseMirror'),
            )
          )
            changed();
        })
      : null;
  mutations?.observe(node, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ['class', 'style'],
  });
  const unsubscribe = core.subscribe((event) => {
    if (
      event.reason === 'content-pending' ||
      event.reason === 'content-sync' ||
      event.reason === 'source-input'
    )
      changed();
    controller.caretChanged('semantic');
  });
  const paneForEvent = (event: Event): SyncPane | null => {
    if (!(event.target instanceof Element)) return null;
    if (event.target.closest('.source-pane')) return 'source';
    if (event.target.closest('.semantic-pane')) return 'semantic';
    return null;
  };
  const wheel = (event: WheelEvent) => {
    const pane = paneForEvent(event);
    if (!pane || event.ctrlKey) return;
    const inner = event.target instanceof Element
      ? event.target.closest<HTMLElement>('.code-content, .code-input')
      : null;
    if (inner && (event.deltaX !== 0 ||
        (event.deltaY < 0 && inner.scrollTop > 0) ||
        (event.deltaY > 0 && inner.scrollTop < inner.scrollHeight - inner.clientHeight - 1))) {
      controller.userIntent(pane, 'local');
      return;
    }
    controller.userIntent(pane, 'scroll');
  };
  const pointerDown = (event: PointerEvent) => {
    const pane = paneForEvent(event);
    if (!pane || event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest('.code-card')) {
      controller.userIntent(pane, 'local');
      return;
    }
    const element = getElement(pane);
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const scrollbar =
      event.target === element ||
      event.clientX >= rect.right - 16 ||
      event.clientX <= rect.left + 16;
    pointerPane = pane;
    controller.userIntent(pane, scrollbar ? 'scroll' : 'caret');
    if (!scrollbar) controller.dragging(pane, true);
  };
  const pointerUp = () => {
    if (pointerPane) controller.dragging(pointerPane, false);
    pointerPane = null;
  };
  const keyDown = (event: KeyboardEvent) => {
    const pane = paneForEvent(event);
    if (pane && event.target instanceof Element && event.target.closest('.code-card')) {
      controller.userIntent(pane, 'local');
      return;
    }
    if (
      !pane ||
      event.isComposing ||
      event.key === 'Shift' ||
      event.key === 'Control' ||
      event.key === 'Meta'
    )
      return;
    if (
      (event.ctrlKey || event.metaKey) &&
      !['Home', 'End', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'z', 'y'].includes(
        event.key,
      )
    )
      return;
    controller.userIntent(pane, ['PageUp', 'PageDown'].includes(event.key) ? 'scroll' : 'caret');
  };
  const input = (event: Event) => {
    const pane = paneForEvent(event);
    if (pane) controller.userIntent(pane,
      event.target instanceof Element && event.target.closest('.code-card') ? 'local' : 'caret');
  };
  const scroll = (event: Event) => {
    const pane = paneForEvent(event);
    if (pane && event.target === getElement(pane)) controller.scroll(pane);
  };
  const composition = (event: Event) => {
    const pane = paneForEvent(event);
    if (pane) {
      controller.userIntent(pane,
        event.target instanceof Element && event.target.closest('.code-card') ? 'local' : 'caret');
      controller.composing(pane, event.type === 'compositionstart');
    }
  };
  const sourceCaret = () => controller.caretChanged('source');
  const navigation = (event: Event) => {
    const pane = (event as CustomEvent<{ pane: SyncPane }>).detail?.pane;
    if (pane === 'source' || pane === 'semantic') controller.userIntent(pane, 'scroll');
  };
  const initialize = () => {
    controller.update({
      enabled: params.mode === 'split' && !params.largeDocumentMode,
      documentId: params.documentId,
      activePane: params.activePane,
      paused: params.paused,
    });
    changed();
  };
  node.addEventListener('wheel', wheel, { capture: true, passive: true });
  node.addEventListener('pointerdown', pointerDown, true);
  window.addEventListener('pointerup', pointerUp);
  window.addEventListener('pointercancel', pointerUp);
  node.addEventListener('keydown', keyDown, true);
  node.addEventListener('input', input, true);
  node.addEventListener('scroll', scroll, true);
  node.addEventListener('compositionstart', composition, true);
  node.addEventListener('compositionend', composition, true);
  node.addEventListener('nomo:source-caret-change', sourceCaret);
  node.addEventListener('nomo:scroll-sync-navigation', navigation);
  node.addEventListener('nomo:editor-viewport-layout-refresh', changed);
  node.addEventListener('nomo:source-layout-change', changed);
  node.addEventListener('nomo:mode-pane-ready', changed);
  node.addEventListener('nomo:mode-pane-transition-complete', changed);
  document.fonts?.addEventListener('loadingdone', changed);
  initialize();
  return {
    update(next: SyncWorkspaceParams) {
      params = next;
      initialize();
    },
    destroy() {
      destroyed = true;
      controller.destroy();
      unsubscribe();
      resize?.disconnect();
      mutations?.disconnect();
      if (geometryFrame) cancelAnimationFrame(geometryFrame);
      node.removeEventListener('wheel', wheel, true);
      node.removeEventListener('pointerdown', pointerDown, true);
      window.removeEventListener('pointerup', pointerUp);
      window.removeEventListener('pointercancel', pointerUp);
      node.removeEventListener('keydown', keyDown, true);
      node.removeEventListener('input', input, true);
      node.removeEventListener('scroll', scroll, true);
      node.removeEventListener('compositionstart', composition, true);
      node.removeEventListener('compositionend', composition, true);
      node.removeEventListener('nomo:source-caret-change', sourceCaret);
      node.removeEventListener('nomo:scroll-sync-navigation', navigation);
      node.removeEventListener('nomo:editor-viewport-layout-refresh', changed);
      node.removeEventListener('nomo:source-layout-change', changed);
      node.removeEventListener('nomo:mode-pane-ready', changed);
      node.removeEventListener('nomo:mode-pane-transition-complete', changed);
      document.fonts?.removeEventListener('loadingdone', changed);
    },
  };
}
