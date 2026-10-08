import { normalizeHeadingTitle, type OutlineItem } from '../../lib/outline/outlineService';
import { slugifyHeading } from '../../lib/toc/tocService';
import { getOutlineItemAtLine } from './outlineState';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';

export type OutlineScrollAnchor =
  | {
      kind: 'outline';
      outlineId: string;
      anchorPos?: number;
      offsetFromTop?: number;
      scrollTop?: number;
      sectionProgress: number;
      documentProgress: number;
      sourceLine: number;
    }
  | {
      kind: 'document';
      anchorPos?: number;
      offsetFromTop?: number;
      scrollTop?: number;
      sectionProgress: number;
      documentProgress: number;
      sourceLine: number;
    };

export type OutlineScrollBehavior = 'instant' | 'smooth';

interface ScrollToAnchorOptions {
  behavior?: OutlineScrollBehavior;
  anchorMode?: 'semantic' | 'source';
}

interface LegacyOutlineScrollAnchor {
  outlineId: string;
  sectionProgress: number;
}

interface SemanticHeadingAnchor {
  id: string;
  element: HTMLElement;
}

interface SemanticHeadingCache {
  editor: Element | null;
  signature: string;
  anchors: SemanticHeadingAnchor[];
  contentScrollHeight: number;
  clientHeight: number;
}

const semanticHeadingCache = new WeakMap<HTMLElement, SemanticHeadingCache>();
const outlineScrollAnimations = new WeakMap<HTMLElement, number>();

const OUTLINE_SCROLL_DURATION_MS = 260;
const OUTLINE_SCROLL_REDUCED_DURATION_MS = 140;

export function getSourceLineHeight(sourceEditor: MarkdownSourceEditorHandle | undefined) {
  return sourceEditor?.getLineHeight() ?? 24;
}

export function getSourceHeadingSelection(markdown: string, item: OutlineItem) {
  const lines = markdown.split(/\r?\n/);
  const start = lines.slice(0, item.line - 1).join('\n').length + (item.line > 1 ? 1 : 0);
  return { start, end: start + lines[item.line - 1].length };
}

function getSourceVisibleLine(
  scrollTop: number,
  lineHeight: number,
  sourceEditor: MarkdownSourceEditorHandle | undefined,
  _sourcePane: HTMLElement | undefined,
) {
  return (
    sourceEditor?.lineAtHeight(scrollTop) ?? Math.max(1, Math.floor(scrollTop / lineHeight) + 1)
  );
}

function getSourceLineTopInPane(
  sourceLine: number,
  lineHeight: number,
  sourceEditor: MarkdownSourceEditorHandle | undefined,
  _sourcePane: HTMLElement | undefined,
) {
  return (
    sourceEditor?.getLineTop(sourceLine) ?? Math.max(0, Math.floor(sourceLine) - 1) * lineHeight
  );
}

export function getActiveOutlineIdFromSource(
  outline: OutlineItem[],
  scrollTop: number,
  lineHeight: number,
  sourceEditor?: MarkdownSourceEditorHandle,
  sourcePane?: HTMLElement,
) {
  if (!outline.length) {
    return '';
  }
  const visibleLine = getSourceVisibleLine(scrollTop, lineHeight, sourceEditor, sourcePane);
  return getOutlineItemAtLine(outline, visibleLine)?.id ?? outline[0]?.id ?? '';
}

export function getSourceScrollAnchor(
  outline: OutlineItem[],
  scrollTop: number,
  lineHeight: number,
  sourceEditor?: MarkdownSourceEditorHandle,
  sourcePane?: HTMLElement,
): OutlineScrollAnchor | null {
  const visibleLine = getSourceVisibleLine(scrollTop, lineHeight, sourceEditor, sourcePane);
  return getSourceScrollAnchorAtLine(
    outline,
    visibleLine,
    scrollTop,
    lineHeight,
    sourceEditor,
    sourcePane,
  );
}

export function getSourceScrollAnchorAtLine(
  outline: OutlineItem[],
  sourceLine: number,
  scrollTop: number,
  lineHeight: number,
  sourceEditor?: MarkdownSourceEditorHandle,
  sourcePane?: HTMLElement,
): OutlineScrollAnchor | null {
  const anchorLine = Math.max(1, Math.floor(sourceLine));
  const sourceScrollContainer = sourcePane ?? sourceEditor?.getScrollElement();
  const documentProgress = getScrollProgress(sourceScrollContainer, scrollTop);
  const sourceLineTop = getSourceLineTopInPane(
    anchorLine,
    lineHeight,
    sourceEditor,
    sourceScrollContainer ?? undefined,
  );

  if (!outline.length) {
    return {
      kind: 'document',
      anchorPos: anchorLine,
      offsetFromTop: scrollTop - sourceLineTop,
      scrollTop,
      sectionProgress: documentProgress,
      documentProgress,
      sourceLine: anchorLine,
    };
  }

  const activeOutlineId =
    getOutlineItemAtLine(outline, anchorLine)?.id ??
    getActiveOutlineIdFromSource(
      outline,
      scrollTop,
      lineHeight,
      sourceEditor,
      sourceScrollContainer ?? undefined,
    );
  const currentIndex = Math.max(
    0,
    outline.findIndex((item) => item.id === activeOutlineId),
  );
  const currentItem = outline[currentIndex] ?? outline[0];
  const nextItem = outline[currentIndex + 1];
  const totalLineCount = getSourceTotalLineCount(
    sourceEditor,
    Math.max(1, Math.floor(scrollTop / lineHeight) + 1),
  );
  const sectionEndLine = nextItem?.line ?? totalLineCount + 1;
  const sectionLineCount = Math.max(1, sectionEndLine - currentItem.line);
  const sectionProgress = clamp((anchorLine - currentItem.line) / sectionLineCount, 0, 1);

  return {
    kind: 'outline',
    outlineId: currentItem.id,
    anchorPos: anchorLine,
    offsetFromTop: scrollTop - sourceLineTop,
    scrollTop,
    sectionProgress,
    documentProgress,
    sourceLine: anchorLine,
  };
}

export function scrollSourceToAnchor(
  outline: OutlineItem[],
  sourcePane: HTMLElement | undefined,
  sourceEditor: MarkdownSourceEditorHandle | undefined,
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor | null,
) {
  if (!sourcePane || !anchor) {
    return;
  }

  if (isDocumentAnchor(anchor)) {
    setScrollTop(sourcePane, getScrollTopByProgress(sourcePane, anchor.documentProgress));
    return;
  }

  const currentIndex = outline.findIndex((item) => item.id === anchor.outlineId);
  if (currentIndex === -1) {
    setScrollTop(sourcePane, getScrollTopByProgress(sourcePane, getAnchorDocumentProgress(anchor)));
    return;
  }

  const currentItem = outline[currentIndex];
  const nextItem = outline[currentIndex + 1];
  const totalLineCount = getSourceTotalLineCount(sourceEditor, currentItem.line);
  const sectionEndLine = nextItem?.line ?? totalLineCount + 1;
  const sectionLineCount = Math.max(1, sectionEndLine - currentItem.line);
  const targetLine = currentItem.line + Math.round(sectionLineCount * anchor.sectionProgress);
  setScrollTop(
    sourcePane,
    clampScrollTop(
      sourcePane,
      getSourceLineTopInPane(
        targetLine,
        getSourceLineHeight(sourceEditor),
        sourceEditor,
        sourcePane,
      ),
    ),
  );
}

export function restoreSourceReadingPosition(
  outline: OutlineItem[],
  sourcePane: HTMLElement | undefined,
  sourceEditor: MarkdownSourceEditorHandle | undefined,
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor | null,
  options: ScrollToAnchorOptions = {},
) {
  if (!sourcePane || !anchor) {
    return;
  }

  if (options.anchorMode === 'source' && hasPixelAnchor(anchor)) {
    setScrollTop(
      sourcePane,
      getSourceLineTopInPane(
        anchor.anchorPos,
        getSourceLineHeight(sourceEditor),
        sourceEditor,
        sourcePane,
      ) + anchor.offsetFromTop,
    );
    return;
  }

  if (isDocumentAnchor(anchor)) {
    setScrollTop(
      sourcePane,
      getAnchorScrollTopFallback(
        anchor,
        sourcePane,
        getScrollTopByProgress(sourcePane, anchor.documentProgress),
      ),
    );
    return;
  }

  const currentIndex = outline.findIndex((item) => item.id === anchor.outlineId);
  if (currentIndex === -1) {
    setScrollTop(
      sourcePane,
      getAnchorScrollTopFallback(
        anchor,
        sourcePane,
        getScrollTopByProgress(sourcePane, getAnchorDocumentProgress(anchor)),
      ),
    );
    return;
  }

  scrollSourceToAnchor(outline, sourcePane, sourceEditor, anchor);
}

export function getActiveOutlineIdFromSemantic(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
) {
  if (!outline.length || !semanticPane) {
    return '';
  }

  const headingAnchors = getSemanticHeadingAnchors(semanticPane, outline);
  if (headingAnchors.length === 0) {
    return outline[0]?.id ?? '';
  }

  const viewportTop = semanticPane.getBoundingClientRect().top;
  const threshold = viewportTop + 96;
  let activeIndex = 0;
  headingAnchors.forEach(({ element }, index) => {
    if (element.getBoundingClientRect().top <= threshold) {
      activeIndex = index;
    }
  });
  return headingAnchors[Math.min(activeIndex, headingAnchors.length - 1)]?.id ?? '';
}

export function getSemanticScrollAnchor(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
  savedScrollTop?: number,
): OutlineScrollAnchor | null {
  return getSemanticScrollAnchorAtTop(
    outline,
    semanticPane,
    semanticPane?.scrollTop ?? 0,
    savedScrollTop,
  );
}

export function getSemanticScrollAnchorForBlock(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
  blockElement: HTMLElement | null | undefined,
  savedScrollTop?: number,
): OutlineScrollAnchor | null {
  if (!semanticPane || !blockElement) {
    return getSemanticScrollAnchor(outline, semanticPane, savedScrollTop);
  }
  return getSemanticScrollAnchorAtTop(
    outline,
    semanticPane,
    getElementTopInScrollContainer(blockElement, semanticPane),
    savedScrollTop,
  );
}

function getSemanticScrollAnchorAtTop(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
  anchorTop: number,
  savedScrollTop?: number,
): OutlineScrollAnchor | null {
  const headingAnchors = getSemanticHeadingAnchors(semanticPane, outline);
  // 面板处于 display:none 时 getBoundingClientRect().height 为 0，
  // 且浏览器会将 scrollHeight/clientHeight 也重置为 0，
  // 此时标题元素的位置测量全部失效，必须回退到 documentProgress 模式，
  // 并使用切换前保存的 scrollTop 和缓存的面板尺寸来计算进度。
  const paneHidden = semanticPane && semanticPane.getBoundingClientRect().height === 0;
  if (!outline.length || !semanticPane || headingAnchors.length === 0 || paneHidden) {
    if (!semanticPane) {
      return null;
    }
    const scrollTop = paneHidden ? (savedScrollTop ?? semanticPane.scrollTop) : anchorTop;
    // 面板隐藏时 scrollHeight 为 0，需要从缓存中获取真实的面板尺寸
    const cached = semanticHeadingCache.get(semanticPane);
    const maxScrollTop =
      cached && cached.contentScrollHeight > 0
        ? Math.max(0, cached.contentScrollHeight - cached.clientHeight)
        : getContentMaxScrollTop(semanticPane);
    const documentProgress = maxScrollTop > 0 ? clamp(scrollTop / maxScrollTop, 0, 1) : 0;
    return {
      kind: 'document',
      anchorPos: 1,
      offsetFromTop: 0,
      scrollTop,
      sectionProgress: documentProgress,
      documentProgress,
      sourceLine: 1,
    };
  }

  const visibleTop = clamp(anchorTop, 0, getMaxScrollTop(semanticPane));
  let activeIndex = 0;
  headingAnchors.forEach(({ element }, index) => {
    if (getElementTopInScrollContainer(element, semanticPane) <= visibleTop + 24) {
      activeIndex = index;
    }
  });

  const currentTop = getElementTopInScrollContainer(
    headingAnchors[activeIndex].element,
    semanticPane,
  );
  const nextTop = headingAnchors[activeIndex + 1]
    ? getElementTopInScrollContainer(headingAnchors[activeIndex + 1].element, semanticPane)
    : getContentScrollHeight(semanticPane);
  const sectionHeight = Math.max(1, nextTop - currentTop);
  const sectionProgress = clamp((visibleTop - currentTop) / sectionHeight, 0, 1);
  const outlineId =
    headingAnchors[Math.min(activeIndex, headingAnchors.length - 1)]?.id ?? outline[0].id;
  const sourceLine = outline.find((item) => item.id === outlineId)?.line ?? 1;

  return {
    kind: 'outline',
    outlineId,
    anchorPos: activeIndex,
    offsetFromTop: visibleTop - currentTop,
    scrollTop: visibleTop,
    sectionProgress,
    documentProgress: getScrollProgress(semanticPane, visibleTop),
    sourceLine,
  };
}

export function scrollSemanticToAnchor(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor | null,
  options: ScrollToAnchorOptions = {},
) {
  const headingAnchors = getSemanticHeadingAnchors(semanticPane, outline);
  if (!semanticPane || !anchor) {
    return;
  }

  if (isDocumentAnchor(anchor) || headingAnchors.length === 0 || !hasAnchorOutlineId(anchor)) {
    scrollElementTo(
      semanticPane,
      getScrollTopByProgress(semanticPane, getAnchorDocumentProgress(anchor)),
      options.behavior,
    );
    return;
  }

  const currentIndex = headingAnchors.findIndex((item) => item.id === anchor.outlineId);
  if (currentIndex === -1 || !headingAnchors[currentIndex]) {
    scrollElementTo(
      semanticPane,
      getScrollTopByProgress(semanticPane, getAnchorDocumentProgress(anchor)),
      options.behavior,
    );
    return;
  }

  const currentTop = getElementTopInScrollContainer(
    headingAnchors[currentIndex].element,
    semanticPane,
  );
  const nextTop = headingAnchors[currentIndex + 1]
    ? getElementTopInScrollContainer(headingAnchors[currentIndex + 1].element, semanticPane)
    : getContentScrollHeight(semanticPane);
  const sectionHeight = Math.max(1, nextTop - currentTop);
  const top = clampScrollTop(
    semanticPane,
    currentTop + sectionHeight * anchor.sectionProgress - 32,
  );
  scrollElementTo(semanticPane, top, options.behavior);
}

export function restoreSemanticReadingPosition(
  outline: OutlineItem[],
  semanticPane: HTMLElement | undefined,
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor | null,
  options: ScrollToAnchorOptions = {},
) {
  const headingAnchors = getSemanticHeadingAnchors(semanticPane, outline);
  if (!semanticPane || !anchor) {
    return;
  }

  if (isDocumentAnchor(anchor) || headingAnchors.length === 0 || !hasAnchorOutlineId(anchor)) {
    scrollElementTo(
      semanticPane,
      getAnchorScrollTopFallback(
        anchor,
        semanticPane,
        getScrollTopByProgress(semanticPane, getAnchorDocumentProgress(anchor)),
      ),
      options.behavior,
    );
    return;
  }

  const matchedIndex = headingAnchors.findIndex((item) => item.id === anchor.outlineId);
  const currentIndex =
    matchedIndex >= 0
      ? matchedIndex
      : hasPixelAnchor(anchor)
        ? Math.min(headingAnchors.length - 1, Math.max(0, Math.floor(anchor.anchorPos)))
        : -1;

  if (currentIndex === -1 || !headingAnchors[currentIndex]) {
    scrollElementTo(
      semanticPane,
      getAnchorScrollTopFallback(
        anchor,
        semanticPane,
        getScrollTopByProgress(semanticPane, getAnchorDocumentProgress(anchor)),
      ),
      options.behavior,
    );
    return;
  }

  if (options.anchorMode === 'semantic' && hasPixelAnchor(anchor)) {
    scrollElementTo(
      semanticPane,
      getElementTopInScrollContainer(headingAnchors[currentIndex].element, semanticPane) +
        anchor.offsetFromTop,
      options.behavior,
    );
    return;
  }

  scrollSemanticToAnchor(outline, semanticPane, anchor, options);
}

function scrollElementTo(
  scrollContainer: HTMLElement,
  top: number,
  behavior: OutlineScrollBehavior = 'smooth',
) {
  if (behavior === 'instant') {
    setScrollTop(scrollContainer, top);
    return;
  }
  smoothScrollElementTo(scrollContainer, top);
}

export function smoothScrollElementTo(scrollContainer: HTMLElement, top: number) {
  const targetTop = clampScrollTop(scrollContainer, top);
  const startTop = scrollContainer.scrollTop;
  const delta = targetTop - startTop;
  const duration = getOutlineScrollDuration();
  const raf = typeof window !== 'undefined' ? window.requestAnimationFrame?.bind(window) : null;
  const cancelRaf =
    typeof window !== 'undefined' ? window.cancelAnimationFrame?.bind(window) : null;

  const activeFrame = outlineScrollAnimations.get(scrollContainer);
  if (activeFrame !== undefined && cancelRaf) {
    cancelRaf(activeFrame);
  }

  if (!raf || Math.abs(delta) < 1 || duration <= 0) {
    setScrollTop(scrollContainer, targetTop);
    outlineScrollAnimations.delete(scrollContainer);
    return;
  }

  const startedAt = Date.now();
  const tick = () => {
    const elapsed = Date.now() - startedAt;
    const progress = clamp(elapsed / duration, 0, 1);
    setScrollTop(scrollContainer, startTop + delta * easeOutCubic(progress));

    if (progress < 1) {
      outlineScrollAnimations.set(scrollContainer, raf(tick));
    } else {
      setScrollTop(scrollContainer, targetTop);
      outlineScrollAnimations.delete(scrollContainer);
    }
  };

  outlineScrollAnimations.set(scrollContainer, raf(tick));
}

function getSemanticHeadings(semanticPane: HTMLElement | undefined) {
  if (!semanticPane) {
    return [];
  }
  return Array.from(
    semanticPane.querySelectorAll<HTMLElement>(
      '.ProseMirror h1, .ProseMirror h2, .ProseMirror h3, .ProseMirror h4, .ProseMirror h5, .ProseMirror h6',
    ),
  );
}

function getSemanticHeadingAnchors(
  semanticPane: HTMLElement | undefined,
  outline: OutlineItem[],
): SemanticHeadingAnchor[] {
  if (!semanticPane) {
    return [];
  }

  const editor = semanticPane.querySelector('.ProseMirror');
  const signature = outline.map((item) => item.id).join('\u001f');
  const cached = semanticHeadingCache.get(semanticPane);
  if (cached && cached.editor === editor && cached.signature === signature) {
    // replaceViewState 重建 ProseMirror DOM 后，缓存中的旧元素引用
    // 可能已被移出 DOM 树，必须验证仍在文档中才能复用。
    const allStillInDom = cached.anchors.every((a) => editor?.contains(a.element));
    if (allStillInDom) {
      if (semanticPane.clientHeight > 0) {
        cached.contentScrollHeight = getContentScrollHeight(semanticPane);
        cached.clientHeight = semanticPane.clientHeight;
      }
      return cached.anchors;
    }
  }

  const headings = getSemanticHeadings(semanticPane);
  const usedIds = new Map<string, number>();
  const anchors = headings.map((element, index) => {
    const title = normalizeHeadingTitle(element.textContent ?? '');
    const baseId = slugifyHeading(title) || `heading-${index + 1}`;
    const seen = usedIds.get(baseId) ?? 0;
    usedIds.set(baseId, seen + 1);
    return {
      id: seen === 0 ? baseId : `${baseId}-${seen + 1}`,
      element,
    };
  });
  semanticHeadingCache.set(semanticPane, {
    editor,
    signature,
    anchors,
    contentScrollHeight: getContentScrollHeight(semanticPane),
    clientHeight: semanticPane.clientHeight,
  });
  return anchors;
}

function getElementTopInScrollContainer(element: HTMLElement, scrollContainer: HTMLElement) {
  const elementTop = element.getBoundingClientRect().top;
  const containerTop = scrollContainer.getBoundingClientRect().top;
  return elementTop - containerTop + scrollContainer.scrollTop;
}

function isDocumentAnchor(
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor,
): anchor is Extract<OutlineScrollAnchor, { kind: 'document' }> {
  return 'kind' in anchor && anchor.kind === 'document';
}

function hasAnchorOutlineId(
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor,
): anchor is Extract<OutlineScrollAnchor, { kind: 'outline' }> | LegacyOutlineScrollAnchor {
  return (
    'outlineId' in anchor && typeof anchor.outlineId === 'string' && anchor.outlineId.length > 0
  );
}

function getAnchorDocumentProgress(anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor) {
  return 'documentProgress' in anchor ? anchor.documentProgress : anchor.sectionProgress;
}

function hasPixelAnchor(
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor,
): anchor is OutlineScrollAnchor & { anchorPos: number; offsetFromTop: number } {
  return (
    'anchorPos' in anchor &&
    typeof anchor.anchorPos === 'number' &&
    Number.isFinite(anchor.anchorPos) &&
    'offsetFromTop' in anchor &&
    typeof anchor.offsetFromTop === 'number' &&
    Number.isFinite(anchor.offsetFromTop)
  );
}

function getAnchorScrollTopFallback(
  anchor: OutlineScrollAnchor | LegacyOutlineScrollAnchor,
  scrollContainer: HTMLElement,
  progressFallback: number,
) {
  if (
    'scrollTop' in anchor &&
    typeof anchor.scrollTop === 'number' &&
    Number.isFinite(anchor.scrollTop)
  ) {
    return clampScrollTop(scrollContainer, anchor.scrollTop);
  }
  return progressFallback;
}

function getScrollProgress(scrollContainer: HTMLElement | undefined | null, scrollTop: number) {
  if (!scrollContainer) {
    return 0;
  }
  const maxScrollTop = getContentMaxScrollTop(scrollContainer);
  return maxScrollTop > 0 ? clamp(scrollTop / maxScrollTop, 0, 1) : 0;
}

function getScrollTopByProgress(scrollContainer: HTMLElement, progress: number) {
  return clampScrollTop(
    scrollContainer,
    getContentMaxScrollTop(scrollContainer) * clamp(progress, 0, 1),
  );
}

function clampScrollTop(scrollContainer: HTMLElement, scrollTop: number) {
  return clamp(scrollTop, 0, getMaxScrollTop(scrollContainer));
}

export function setScrollTop(scrollContainer: HTMLElement, scrollTop: number) {
  const top = clampScrollTop(scrollContainer, scrollTop);
  if (
    typeof window !== 'undefined' &&
    typeof window.requestAnimationFrame === 'function' &&
    typeof scrollContainer.scrollTo === 'function'
  ) {
    scrollContainer.scrollTo({ top, behavior: 'instant' });
    return;
  }
  scrollContainer.scrollTop = top;
}

function getMaxScrollTop(scrollContainer: HTMLElement) {
  return Math.max(0, scrollContainer.scrollHeight - scrollContainer.clientHeight);
}

function getContentMaxScrollTop(scrollContainer: HTMLElement) {
  return Math.max(0, getContentScrollHeight(scrollContainer) - scrollContainer.clientHeight);
}

function getContentScrollHeight(scrollContainer: HTMLElement) {
  // 阅读进度和大纲区段只描述真实文档，不能把末尾虚拟滚动空间算进最后一节。
  const spacer = scrollContainer.querySelector<HTMLElement>('[data-scroll-past-end]');
  const spacerHeight = spacer?.offsetHeight ?? 0;
  return Math.max(scrollContainer.clientHeight, scrollContainer.scrollHeight - spacerHeight);
}

function getSourceTotalLineCount(
  sourceEditor: MarkdownSourceEditorHandle | undefined,
  fallbackLine: number,
) {
  if (!sourceEditor) {
    return Math.max(1, fallbackLine);
  }
  return Math.max(1, sourceEditor.getLineCount());
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function getOutlineScrollDuration() {
  const reduceMotion =
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  return reduceMotion ? OUTLINE_SCROLL_REDUCED_DURATION_MS : OUTLINE_SCROLL_DURATION_MS;
}

function easeOutCubic(progress: number) {
  return 1 - Math.pow(1 - progress, 3);
}
