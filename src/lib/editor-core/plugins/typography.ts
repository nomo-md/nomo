import { Plugin, PluginKey, TextSelection, type Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet, type EditorView } from 'prosemirror-view';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { getTypographyOptions, subscribeTypography } from '../../typography/options';
import {
  getTypographyInteraction,
  subscribeTypographyInteraction,
  type TypographyInteractionSnapshot,
} from '../../typography/interaction';
import {
  createTypographyMeasureCache,
  measureParagraph,
  measureParagraphSync,
  type MeasuredParagraph,
} from '../../typography/measure';
import { createTypographySolver } from '../../typography/solver';
import { layoutParagraph } from '../../typography/knuthPlass';
import { preserveSourceWhitespace } from '../../typography/rules';
import { createRenderPlan, PARAGRAPH_CSS, unitStyleCss } from '../../typography/renderPlan';
import type {
  ParagraphLayout,
  TypographyController,
  TypographyOptions,
} from '../../typography/types';
import { logDebug } from '../../services/logger';
import { refreshTypographyBreaks } from '../nodeViews/TypographyBreakNodeView';
import { analyzeInlineSource } from '../InlineSourceCodec';
import { inlineSourceEditingPluginKey } from './inlineSourceEditing';

const key = new PluginKey<DecorationSet>('nomo-typography');
const meta = 'nomo:typography';
const controllers = new WeakMap<EditorView, TypographyController>();
let nextGeometryProjection = 0;
const graphemeSegmenter =
  typeof Intl.Segmenter === 'function'
    ? new Intl.Segmenter('zh', { granularity: 'grapheme' })
    : null;
const graphemeBoundaries = new WeakMap<ProseMirrorNode, number[]>();
export const isTypographyTransaction = (transaction: Transaction) =>
  transaction.getMeta(meta) !== undefined;
export const flushEditorTypography = (view: EditorView) =>
  controllers.get(view)?.flush() ?? Promise.resolve();

/** A selection changes visible source width without changing the PM document. */
function inlineSourceGeometrySnapshot(view: EditorView): string {
  const source = inlineSourceEditingPluginKey.getState(view.state);
  if (!source) return '';
  const selection = view.state.selection;
  const expanded = view.editable
    ? source.expandedMarkers.map((range) => `${range.from}:${range.to}`).join(',')
    : '';
  return `${view.editable}:${expanded}:${selection.empty ? '' : `${selection.from}:${selection.to}`}`;
}

function paragraphSourceGeometry(view: EditorView, pos: number, node: ProseMirrorNode): string {
  const source = inlineSourceEditingPluginKey.getState(view.state);
  if (!source) return '';
  const from = pos + 1;
  const to = from + node.content.size;
  const expanded = view.editable
    ? source.expandedMarkers
        .filter((range) => range.from < to && range.to > from)
        .map((range) => `${Math.max(from, range.from) - from}:${Math.min(to, range.to) - from}`)
        .join(',')
    : '';
  const ranges = view.state.selection.ranges;
  const selectedCode = ranges
    .filter(
      (range) =>
        range.$from.pos !== range.$to.pos &&
        analyzeInlineSource(node).spans.some(
          (span) =>
            span.type === 'code' &&
            range.$from.pos < pos + 1 + span.to &&
            range.$to.pos > pos + 1 + span.from,
        ),
    )
    .map(
      (range) =>
        `${Math.max(0, range.$from.pos - pos - 1)}:${Math.min(node.content.size, range.$to.pos - pos - 1)}`,
    )
    .join(',');
  return `${expanded}:${selectedCode}`;
}

export function typographyPlugin() {
  return new Plugin<DecorationSet>({
    key,
    state: {
      init: () => DecorationSet.empty,
      apply: (tr, value) => tr.getMeta(meta) ?? value.map(tr.mapping, tr.doc),
    },
    props: {
      decorations: (state) => key.getState(state),
      handleKeyDown: moveTypographyCursor,
    },
    view: (view) => createViewController(view),
  });
}

/** Visual widgets have no source positions, so native DOM motion can skip or revisit a caret. */
function moveTypographyCursor(view: EditorView, event: KeyboardEvent): boolean {
  if (
    !view.editable ||
    view.composing ||
    event.isComposing ||
    event.ctrlKey ||
    event.metaKey ||
    event.altKey ||
    !['ArrowLeft', 'ArrowRight'].includes(event.key) ||
    !getTypographyOptions().enabled ||
    !graphemeSegmenter
  )
    return false;
  const { selection } = view.state;
  if (!(selection instanceof TextSelection) || !selection.$anchor.sameParent(selection.$head))
    return false;
  const { $head } = selection;
  if (!$head.depth || !$head.parent.isTextblock) return false;
  const paragraphPos = $head.before();
  if (
    !key
      .getState(view.state)
      ?.find(paragraphPos, paragraphPos + $head.parent.nodeSize)
      .some(
        (decoration) =>
          decoration.from === paragraphPos && decoration.spec.paragraph === $head.parent,
      )
  )
    return false;
  const direction = event.key === 'ArrowRight' ? 1 : -1;
  let target: number;
  if (!selection.empty && !event.shiftKey) {
    target = direction > 0 ? selection.to : selection.from;
  } else {
    // Preserve native atom navigation and the editor's formula/comment entry commands.
    const adjacent = direction > 0 ? $head.nodeAfter : $head.nodeBefore;
    if (!adjacent?.isText) return false;
    let boundaries = graphemeBoundaries.get($head.parent);
    if (!boundaries) {
      const text = $head.parent.textBetween(0, $head.parent.content.size, '', '\ufffc');
      if (text.length !== $head.parent.content.size) return false;
      boundaries = [
        0,
        ...Array.from(graphemeSegmenter.segment(text), (part) => part.index + part.segment.length),
      ];
      graphemeBoundaries.set($head.parent, boundaries);
    }
    const offset =
      direction > 0
        ? boundaries.find((boundary) => boundary > $head.parentOffset)
        : boundaries[boundaries.findIndex((boundary) => boundary >= $head.parentOffset) - 1];
    if (offset === undefined) return false;
    target = $head.start() + offset;
  }
  view.dispatch(
    view.state.tr
      .setSelection(
        TextSelection.create(view.state.doc, event.shiftKey ? selection.anchor : target, target),
      )
      .scrollIntoView(),
  );
  return true;
}

interface TypographyViewportAnchor {
  pane: HTMLElement;
  doc: ProseMirrorNode;
  pos: number;
  top: number;
}

/** 文档位置不随段落行数变化；双栏跟随栏的位置仍交给原有同步控制器。 */
function captureViewportAnchor(
  view: EditorView,
  point?: TypographyInteractionSnapshot['anchor'],
): TypographyViewportAnchor | null {
  const pane = view.dom.closest<HTMLElement>('.semantic-pane');
  const grid = pane?.closest<HTMLElement>('.editor-grid');
  if (
    !pane ||
    !pane.clientHeight ||
    (grid?.classList.contains('split-view') && grid.dataset.syncLeader === 'source')
  )
    return null;
  const paneRect = pane.getBoundingClientRect();
  const surface = view.dom.getBoundingClientRect();
  const left = Math.max(paneRect.left, surface.left);
  const right = Math.min(paneRect.right, surface.right);
  const top = Math.max(paneRect.top, surface.top);
  const bottom = Math.min(paneRect.bottom, surface.bottom);
  if (right <= left || bottom <= top) return null;
  const inside =
    point && point.left >= left && point.left < right && point.top >= top && point.top < bottom;
  try {
    const hit = view.posAtCoords({
      left: inside ? point.left : (left + right) / 2,
      top: inside ? point.top : (top + bottom) / 2,
    });
    if (!hit) return null;
    return { pane, doc: view.state.doc, pos: hit.pos, top: view.coordsAtPos(hit.pos).top };
  } catch {
    // 未挂载、不可映射的 NodeView 或浏览器尚无几何时，不猜测文档位置。
    return null;
  }
}

function restoreViewportAnchor(view: EditorView, anchor: TypographyViewportAnchor | null) {
  if (!anchor || anchor.doc !== view.state.doc || !anchor.pane.isConnected) return;
  try {
    const delta = view.coordsAtPos(anchor.pos).top - anchor.top;
    if (Math.abs(delta) > 0.1) {
      const max = Math.max(0, anchor.pane.scrollHeight - anchor.pane.clientHeight);
      anchor.pane.scrollTop = Math.min(max, Math.max(0, anchor.pane.scrollTop + delta));
    }
  } catch {
    // 几何恢复不改变正文或选区；位置已失效时保留当前视角。
  }
}

function createViewController(view: EditorView) {
  const solver = createTypographySolver();
  const measureCache = createTypographyMeasureCache();
  const pane = view.dom.closest<HTMLElement>('.semantic-pane');
  type ParagraphTarget = { node: ProseMirrorNode; pos: number; element: HTMLElement };
  const paragraphTargets = new Map<Element, ParagraphTarget>();
  const visibleParagraphs = new Set<Element>();
  let indexedDoc: ProseMirrorNode | null = null;
  let visibilityReady = false;
  let breaksDoc: ProseMirrorNode | null = null;
  let breaksEnabled: boolean | undefined;
  let zoomReusePending = false;
  let zoomProjectionActive = false;
  let zoomAnchorQueued = false;
  let zoomGestureAnchor: TypographyViewportAnchor | null = null;
  let geometryAnchorFrame: number | null = null;
  let pendingGeometryAnchor: TypographyViewportAnchor | null = null;
  let geometryProjection = 0;
  let preparedGeometryRevision = -1;
  let geometryProjectionStyle: HTMLStyleElement | undefined;
  let revision = 0;
  let geometryRevision = 0;
  let completed = -1;
  let destroyed = false;
  let composing = false;
  let dragging = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let running: Promise<void> | null = null;
  let publishing = false;
  let lastWidth = 0;
  const objectSizes = new WeakMap<Element, string>();
  const observedObjects = new Set<Element>();
  let cachedDoc = view.state.doc;
  let cachedSourceGeometry = inlineSourceGeometrySnapshot(view);
  let pendingSourceLayout = false;
  let pendingContentLayout = false;
  let sourceLayoutQueued = false;
  let dragSelectionStarted = false;
  let interaction = getTypographyInteraction();
  let interactionSubscription: ReturnType<typeof subscribeTypographyInteraction> | undefined;
  let layoutFrame: number | null = null;
  let pendingDisableClear = false;
  let geometryRecovery = interaction.phase !== 'idle';
  let completedGeometryRevision = -1;
  let completedEditorGeometry = '';
  let layoutRequestRevision = 0;
  let viewportOnly = interaction.phase === 'preview';
  let viewportScrollTop = view.dom.closest<HTMLElement>('.semantic-pane')?.scrollTop ?? 0;
  let cancelledPasses = 0;
  let initializing = true;
  let finishPublishFrame: (() => void) | undefined;

  const isPreviewing = () => interaction.phase === 'preview';
  const isZoomPreview = () => isPreviewing() && interaction.geometrySource === 'zoom';

  const isComposing = () =>
    composing ||
    view.composing ||
    Boolean(inlineSourceEditingPluginKey.getState(view.state)?.composing);
  const canLayoutSource = () => {
    const source = inlineSourceEditingPluginKey.getState(view.state);
    const sourceDragging = source?.dragging;
    const selectionDrag = source?.dragSelectionStarted ?? dragSelectionStarted;
    return (
      !destroyed &&
      !isComposing() &&
      (!(dragging || sourceDragging) || (view.state.selection.empty && !selectionDrag))
    );
  };
  const projectCurrentWidth = (preserveAlignment = isZoomPreview()) => {
    if (destroyed || !getTypographyOptions().enabled) return;
    // During zoom, native reflow follows every CSS change. Keep one projection
    // for the gesture instead of invalidating all paragraph styles each frame.
    if (preserveAlignment && zoomProjectionActive && view.dom.hasAttribute('data-kp-reflow')) return;
    zoomProjectionActive = preserveAlignment;
    geometryProjection = ++nextGeometryProjection;
    geometryProjectionStyle ??= document.createElement('style');
    geometryProjectionStyle.dataset.kpOwned = 'geometry-projection';
    const staleParagraph = `.ProseMirror[data-kp-reflow="${geometryProjection}"] [data-kp-layout="ready"]:not([data-kp-geometry="${geometryProjection}"])`;
    // Keep semantic text/selection DOM intact, but never display fixed breaks and
    // pixel spacing computed for a different width while the worker catches up.
    geometryProjectionStyle.textContent = `
      ${staleParagraph} { white-space: pre-wrap !important; overflow-wrap: anywhere !important; word-break: normal !important; }
      ${staleParagraph} .kp-break { display: none !important; }
      ${staleParagraph} .kp-unit { margin-left: 0 !important; margin-right: 0 !important; padding-right: 0 !important; line-height: inherit !important; }
      ${preserveAlignment ? `${staleParagraph}[data-kp-alignment="justify"] { text-align: justify !important; }` : ''}
    `;
    if (!geometryProjectionStyle.isConnected) document.head.append(geometryProjectionStyle);
    view.dom.dataset.kpReflow = String(geometryProjection);
  };
  const finishGeometryProjection = () => {
    zoomProjectionActive = false;
    if (!view.dom.hasAttribute('data-kp-reflow')) return;
    const anchor = captureViewportAnchor(view, interaction.anchor);
    view.dom.removeAttribute('data-kp-reflow');
    geometryProjectionStyle?.remove();
    restoreViewportAnchor(view, anchor);
    viewportScrollTop = pane?.scrollTop ?? 0;
  };
  const restoreGeometryAnchor = () => {
    if (geometryAnchorFrame !== null) cancelAnimationFrame(geometryAnchorFrame);
    geometryAnchorFrame = null;
    const anchor = pendingGeometryAnchor;
    pendingGeometryAnchor = null;
    restoreViewportAnchor(view, anchor);
    viewportScrollTop = pane?.scrollTop ?? 0;
    lastWidth = view.dom.clientWidth;
    view.dom
      .closest('.editor-grid')
      ?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
  };
  const publish = (
    decorations: Decoration[],
    sourceOnly = false,
    ranges?: { from: number; to: number }[],
    geometryAnchor?: TypographyViewportAnchor | null,
  ) => {
    if (destroyed || isComposing() || (sourceOnly ? !canLayoutSource() : dragging)) return false;
    if (geometryAnchorFrame !== null || pendingGeometryAnchor) restoreGeometryAnchor();
    const anchor =
      geometryAnchor === undefined
        ? captureViewportAnchor(view, interaction.anchor)
        : geometryAnchor;
    const previous = key.getState(view.state) ?? DecorationSet.empty;
    const next = ranges
      ? previous
          .remove(
            ranges.flatMap((range) =>
              previous
                .find(range.from, range.to)
                .filter((decoration) => decoration.from >= range.from && decoration.to <= range.to),
            ),
          )
          .add(view.state.doc, decorations)
      : DecorationSet.create(view.state.doc, decorations);
    publishing = true;
    try {
      if (!ranges || ranges.length || decorations.length)
        view.dispatch(view.state.tr.setMeta(meta, next).setMeta('addToHistory', false));
    } finally {
      publishing = false;
    }
    restoreViewportAnchor(view, anchor);
    viewportScrollTop = view.dom.closest<HTMLElement>('.semantic-pane')?.scrollTop ?? 0;
    view.dom
      .closest('.editor-grid')
      ?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
    return true;
  };
  const clearDisabledLayout = () => {
    if (!pendingDisableClear || initializing || destroyed || isComposing() || dragging) return;
    if (!key.getState(view.state)?.find().length || publish([])) {
      pendingDisableClear = false;
    }
  };
  const queueRunFrame = () => {
    // Native reflow owns zoom frames; solving and decoration replacement resume
    // at settlement so they cannot stretch a short animation into slow motion.
    if (layoutFrame !== null || destroyed || isZoomPreview() || (isPreviewing() && !viewportOnly))
      return;
    if (typeof requestAnimationFrame !== 'function' || document.hidden) {
      timer = setTimeout(() => void run(), 0);
      return;
    }
    layoutFrame = requestAnimationFrame(() => {
      layoutFrame = null;
      void run();
    });
  };
  const schedule = () => {
    if (destroyed || publishing) return;
    revision++;
    clearTimeout(timer);
    if (isPreviewing()) viewportOnly = true;
    if (isPreviewing() || geometryRecovery || interaction.phase === 'settling') {
      queueRunFrame();
      return;
    }
    timer = setTimeout(() => {
      void run();
    }, 150);
  };
  const geometryChanged = () => {
    projectCurrentWidth();
    geometryRevision++;
    schedule();
  };
  const fontsChanged = () => {
    measureCache.clear();
    geometryChanged();
  };
  const contentLoaded = () => {
    // Async NodeViews may replace an image element without changing the document.
    indexedDoc = null;
    geometryChanged();
  };
  const editorGeometrySignature = () => {
    const font = getComputedStyle(view.dom);
    return JSON.stringify([
      geometryRevision,
      view.dom.clientWidth,
      font.fontFamily,
      font.fontSize,
      font.fontWeight,
      font.fontStyle,
      font.fontStretch,
      font.lineHeight,
      font.letterSpacing,
      font.wordSpacing,
    ]);
  };
  const requestVisibleLayout = () => {
    layoutRequestRevision++;
    geometryRecovery = true;
    viewportOnly = true;
    clearTimeout(timer);
    queueRunFrame();
  };
  // Native visibility tracking avoids reading every paragraph's geometry on each
  // input frame. Rebuild DOM bindings only when the semantic document changes.
  const visibilityObserver =
    typeof IntersectionObserver !== 'undefined'
      ? new IntersectionObserver(
          (entries) => {
            let changed = false;
            for (const entry of entries) {
              if (!paragraphTargets.has(entry.target)) continue;
              const wasVisible = visibleParagraphs.has(entry.target);
              if (entry.isIntersecting) visibleParagraphs.add(entry.target);
              else visibleParagraphs.delete(entry.target);
              changed ||= wasVisible !== entry.isIntersecting;
            }
            visibilityReady = true;
            if (changed && getTypographyOptions().enabled && (isPreviewing() || geometryRecovery))
              requestVisibleLayout();
          },
          { root: pane, rootMargin: '160px 0px' },
        )
      : null;
  const indexParagraphs = (doc: ProseMirrorNode) => {
    if (indexedDoc === doc) return;
    indexedDoc = doc;
    visibilityObserver?.disconnect();
    visibilityReady = false;
    paragraphTargets.clear();
    visibleParagraphs.clear();
    doc.descendants((node, pos) => {
      if (node.type.name !== 'paragraph' || !node.content.size) return;
      const element = view.nodeDOM(pos);
      if (element instanceof HTMLElement) {
        paragraphTargets.set(element, { node, pos, element });
        visibilityObserver?.observe(element);
      }
      return false;
    });
    for (const element of view.dom.querySelectorAll('img,.math-inline')) {
      if (!observedObjects.has(element)) {
        observedObjects.add(element);
        objectObserver?.observe(element);
      }
    }
    for (const element of observedObjects) {
      if (!view.dom.contains(element)) {
        objectObserver?.unobserve(element);
        observedObjects.delete(element);
      }
    }
  };
  const paragraphSignature = (
    element: HTMLElement,
    options: TypographyOptions,
    sourceGeometry: string,
  ) => {
    const font = getComputedStyle(element);
    return JSON.stringify([
      options,
      geometryRevision,
      element.clientWidth,
      font.fontFamily,
      font.fontSize,
      font.fontWeight,
      font.fontStyle,
      font.fontStretch,
      font.lineHeight,
      font.letterSpacing,
      font.wordSpacing,
      sourceGeometry,
    ]);
  };
  const queueSourceLayout = () => {
    if (
      sourceLayoutQueued ||
      !(pendingSourceLayout || pendingContentLayout) ||
      destroyed ||
      isPreviewing() ||
      geometryRecovery
    )
      return;
    sourceLayoutQueued = true;
    // PM has installed the edited/newly visible source DOM by this checkpoint, but the
    // browser has not painted it. Publish its spacing and breaks in the same frame.
    queueMicrotask(() => {
      sourceLayoutQueued = false;
      if (
        !(pendingSourceLayout || pendingContentLayout) ||
        !canLayoutSource() ||
        isPreviewing() ||
        geometryRecovery
      )
        return;
      const contentChanged = pendingContentLayout;
      pendingSourceLayout = false;
      pendingContentLayout = false;
      const options = getTypographyOptions();
      if (!options.enabled) return;
      const value = key.getState(view.state);
      if (!value) return;
      const token = revision;
      const doc = view.state.doc;
      const deadline = performance.now() + (contentChanged ? 16 : 32);
      const targets = new Map<
        number,
        { from: number; to: number; node: ProseMirrorNode; nativeEditing: boolean }
      >();
      for (const decoration of value.find(undefined, undefined, (spec) => spec.paragraph?.type.name === 'paragraph')) {
        const paragraph = decoration.spec.paragraph as ProseMirrorNode | undefined;
        const node = doc.nodeAt(decoration.from);
        if (
          paragraph?.type.name === 'paragraph' &&
          node?.type.name === 'paragraph' &&
          decoration.spec.typographySourceGeometry !== undefined &&
          ((contentChanged && node !== paragraph) ||
            decoration.spec.typographySourceGeometry !==
              paragraphSourceGeometry(view, decoration.from, node))
        )
          targets.set(decoration.from, {
            from: decoration.from,
            to: decoration.from + node.nodeSize,
            node,
            nativeEditing: contentChanged && Boolean(decoration.spec.editingFallback),
          });
      }
      // New/split paragraphs may have no retained node decoration. The active
      // paragraph must still settle before paint, rather than wait for idle work.
      if (contentChanged) {
        const { $head } = view.state.selection;
        if ($head.depth && $head.parent.type.name === 'paragraph') {
          const from = $head.before();
          if (
            !targets.has(from) &&
            !value
              .find(from, from + $head.parent.nodeSize)
              .some((decoration) => decoration.spec.paragraph === $head.parent)
          )
            targets.set(from, {
              from,
              to: from + $head.parent.nodeSize,
              node: $head.parent,
              nativeEditing: false,
            });
        }
      }
      const changedRanges: { from: number; to: number }[] = [];
      const replacements: Decoration[] = [];
      let index = 0;
      const activeFrom = view.state.selection.$head.depth
        ? view.state.selection.$head.before()
        : -1;
      const orderedTargets = [...targets.values()];
      if (contentChanged)
        orderedTargets.sort(
          (a, b) => Number(b.from === activeFrom) - Number(a.from === activeFrom),
        );
      for (const target of orderedTargets) {
        const node = target.node;
        const element = view.nodeDOM(target.from);
        if (!(element instanceof HTMLElement)) continue;
        const sourceGeometry = paragraphSourceGeometry(view, target.from, node);
        const signature = paragraphSignature(element, options, sourceGeometry);
        let rendered: Decoration[] | null = null;
        if (
          !target.nativeEditing &&
          index < 2 &&
          node.content.size <= (contentChanged ? 1024 : 2048) &&
          performance.now() < deadline - 4
        ) {
          try {
            const measureBudget = Math.min(
              contentChanged ? 10 : 24,
              deadline - performance.now() - 4,
            );
            const measured = measureParagraphSync(element, options, measureBudget, {
              cache: measureCache,
            });
            if (measured && performance.now() < deadline) {
              if (view.editable) measured.input = preserveSourceWhitespace(measured.input);
              const result = layoutParagraph({
                ...measured.input,
                timeBudgetMs: Math.min(6, deadline - performance.now()),
                maxCandidates: 20_000,
              });
              if (result.status === 'ready' && performance.now() <= deadline) {
                rendered = renderDecorations(
                  view,
                  target.from,
                  node,
                  measured,
                  result,
                  signature,
                  sourceGeometry,
                  geometryProjection,
                );
              }
            }
          } catch {
            // The native fallback below is cached for this exact visibility state.
          }
        }
        replacements.push(
          ...(rendered ?? [
            fallbackDecoration(
              target.from,
              node,
              contentChanged ? 'editing-budget-fallback' : 'source-visibility-fallback',
              {
                signature,
                sourceGeometry,
                editingFallback: contentChanged,
                originalStyle:
                  element.getAttribute('data-kp-original-style') ??
                  element.getAttribute('style') ??
                  '',
              },
            ),
          ]),
        );
        changedRanges.push({ from: target.from, to: target.to });
        index++;
      }
      if (
        !changedRanges.length ||
        token !== revision ||
        doc !== view.state.doc ||
        !canLayoutSource()
      )
        return;
      const retained = value
        .find()
        .filter(
          (decoration) =>
            !changedRanges.some(
              (range) => decoration.from >= range.from && decoration.to <= range.to,
            ),
        );
      // An async pass can already be awaiting measurement/worker results while
      // this microtask replaces its DOM bindings, despite an unchanged document.
      revision++;
      publish([...retained, ...replacements], true);
      // Leave completed unchanged: the asynchronous pass may still owe work to
      // other paragraphs. It reuses these identical ready/fallback signatures.
    });
  };
  const run = async (): Promise<void> => {
    if (destroyed || isZoomPreview() || (isPreviewing() && !viewportOnly) || isComposing() || dragging)
      return;
    if (geometryAnchorFrame !== null || pendingGeometryAnchor) restoreGeometryAnchor();
    clearDisabledLayout();
    if (sourceLayoutQueued) {
      await Promise.resolve();
      return run();
    }
    if (zoomReusePending) {
      zoomReusePending = false;
      // Read layout once in the scheduled frame, not in every input callback.
      // Responsive objects invalidate independently through ResizeObserver.
      if (completed === revision && completedEditorGeometry === editorGeometrySignature()) {
        completedGeometryRevision = interaction.geometryRevision;
        viewportOnly = false;
        geometryRecovery = false;
        finishGeometryProjection();
        interactionSubscription?.complete(interaction.generation);
        return;
      }
      geometryRecovery = true;
      viewportOnly = true;
    }
    if (
      completed === revision &&
      completedGeometryRevision === interaction.geometryRevision &&
      !viewportOnly
    ) {
      geometryRecovery = false;
      finishGeometryProjection();
      interactionSubscription?.complete(interaction.generation);
      return;
    }
    if (running) {
      await running;
      return;
    }
    const token = revision;
    const doc = view.state.doc;
    const options = getTypographyOptions();
    const geometryAtStart = interaction.geometryRevision;
    const layoutRequestAtStart = layoutRequestRevision;
    const interactivePass = isPreviewing();
    viewportOnly = false;
    if (breaksDoc !== doc || breaksEnabled !== options.enabled) {
      refreshTypographyBreaks(view);
      breaksDoc = doc;
      breaksEnabled = options.enabled;
    }
    if (!options.enabled) {
      if (publish([])) {
        completed = token;
        completedGeometryRevision = interaction.geometryRevision;
      }
      interactionSubscription?.complete(interaction.generation);
      return;
    }
    running = (async () => {
      const startedAt = performance.now();
      const timings = { measureMs: 0, solveMs: 0, renderMs: 0, publishMs: 0 };
      const generation = interaction.generation;
      // 宽度输入复用在途测量；缩放则先让出交互帧，停止后再追补。
      const stale = () =>
        destroyed ||
        isZoomPreview() ||
        revision !== token ||
        view.state.doc !== doc ||
        isComposing() ||
        dragging ||
        (!interactivePass && isPreviewing());
      const viewport = pane?.getBoundingClientRect();
      const viewportTop = viewport?.top ?? 0;
      const viewportBottom = viewport?.bottom ?? window.innerHeight;
      const viewportHeight = Math.max(1, viewportBottom - viewportTop);
      type Target = ParagraphTarget & {
        distance: number;
        priority: boolean;
      };
      const targets: Target[] = [];
      indexParagraphs(doc);
      const available =
        interactivePass && visibilityReady
          ? Array.from(visibleParagraphs, (element) => paragraphTargets.get(element)!)
          : paragraphTargets.values();
      for (const target of available) {
        const { element } = target;
        if (!element.isConnected) {
          indexedDoc = null;
          requestVisibleLayout();
          continue;
        }
        if (element.clientWidth <= 0) continue;
        const rect =
          !visibilityReady || visibleParagraphs.has(element)
            ? element.getBoundingClientRect()
            : null;
        const buffer = Math.min(160, viewportHeight * 0.25);
        const priority =
          !!rect && rect.bottom >= viewportTop - buffer && rect.top <= viewportBottom + buffer;
        if (interactivePass && !priority) continue;
        targets.push({
          ...target,
          distance: rect
            ? Math.max(viewportTop - rect.bottom, rect.top - viewportBottom, 0)
            : Infinity,
          priority,
        });
      }
      targets.sort(
        (a, b) =>
          Number(b.priority) - Number(a.priority) || a.distance - b.distance || a.pos - b.pos,
      );
      const priorityCount =
        geometryRecovery && !interactivePass
          ? targets.filter((target) => target.priority).length
          : 0;
      type Candidate = { target: Target; signature: string; decorations: Decoration[] };
      let pending: Candidate[] = [];
      let skippedGeometry = false;
      const currentSignature = (target: Target) =>
        paragraphSignature(
          target.element,
          options,
          paragraphSourceGeometry(view, target.pos, target.node),
        );
      const current = (target: Target, signature: string) =>
        !stale() &&
        view.nodeDOM(target.pos) === target.element &&
        currentSignature(target) === signature;
      let publishedAt = performance.now();
      let yieldedAt = publishedAt;
      const commit = async () => {
        if (stale()) return false;
        if (!pending.length) return true;
        if (!interactivePass)
          await new Promise<void>((resolve) => {
            let frame: number | undefined;
            let settled = false;
            const finish = () => {
              if (settled) return;
              settled = true;
              if (frame !== undefined) cancelAnimationFrame(frame);
              clearTimeout(timeout);
              finishPublishFrame = undefined;
              resolve();
            };
            const timeout = setTimeout(finish, document.hidden ? 0 : 32);
            finishPublishFrame = finish;
            if (typeof requestAnimationFrame === 'function' && !document.hidden) {
              const id = requestAnimationFrame(finish);
              if (settled) cancelAnimationFrame(id);
              else frame = id;
            }
          });
        if (stale()) return false;
        // CSS 已跟手更新，提交前逐段校验；旧尺寸结果不能覆盖当前断行。
        const accepted = pending.filter((candidate) =>
          current(candidate.target, candidate.signature),
        );
        if (accepted.length !== pending.length) skippedGeometry = true;
        pending = [];
        const publishStart = performance.now();
        const committed =
          !accepted.length ||
          publish(
            accepted.flatMap((candidate) => candidate.decorations),
            false,
            accepted.map(({ target }) => ({
              from: target.pos,
              to: target.pos + target.node.nodeSize,
            })),
          );
        timings.publishMs += performance.now() - publishStart;
        publishedAt = yieldedAt = performance.now();
        return committed;
      };
      for (let index = 0; index < targets.length; index++) {
        const target = targets[index];
        if (stale()) break;
        const sourceGeometry = paragraphSourceGeometry(view, target.pos, target.node);
        const signature = paragraphSignature(target.element, options, sourceGeometry);
        const retained =
          key.getState(view.state)?.find(target.pos, target.pos + target.node.nodeSize) ?? [];
        if (
          retained.some(
            (d) =>
              d.from === target.pos &&
              d.to === target.pos + target.node.nodeSize &&
              d.spec.typographySignature === signature &&
              d.spec.paragraph === target.node &&
              !d.spec.editingFallback,
          )
        ) {
          if (performance.now() - yieldedAt >= 6) {
            await new Promise<void>((resolve) => setTimeout(resolve, 0));
            yieldedAt = performance.now();
          }
          continue;
        }
        try {
          // 每段读取当前 CSS，避免在输入帧内同步采样整个视口。
          const measureStart = performance.now();
          const measured = await measureParagraph(target.element, options, stale, {
            cache: measureCache,
          });
          if (measured && view.editable) measured.input = preserveSourceWhitespace(measured.input);
          timings.measureMs += performance.now() - measureStart;
          if (stale()) break;
          if (!current(target, signature)) {
            skippedGeometry = true;
            if (interactivePass) break;
            continue;
          }
          let decorations: Decoration[];
          const fallback = (reason: string) => [
            fallbackDecoration(target.pos, target.node, reason, { signature, sourceGeometry }),
          ];
          if (!measured) decorations = fallback('unsupported-or-unmeasurable');
          else {
            const solveStart = performance.now();
            const result = await solver.solve(measured.input);
            timings.solveMs += performance.now() - solveStart;
            if (stale()) break;
            if (!current(target, signature)) {
              skippedGeometry = true;
              if (interactivePass) break;
              continue;
            }
            if (result.status === 'fallback') {
              decorations = fallback(result.reason);
              logDebug('typography', '段落使用原生排版', {
                reason: result.reason,
                candidates: result.candidates,
              });
            } else {
              const renderStart = performance.now();
              decorations =
                renderDecorations(
                  view,
                  target.pos,
                  target.node,
                  measured,
                  result,
                  signature,
                  sourceGeometry,
                  geometryProjection,
                ) ?? fallback('stale-dom-bindings');
              timings.renderMs += performance.now() - renderStart;
            }
          }
          pending.push({ target, signature, decorations });
        } catch {
          if (stale()) break;
          if (!current(target, signature)) {
            skippedGeometry = true;
            if (interactivePass) break;
            continue;
          }
          pending.push({
            target,
            signature,
            decorations: [
              fallbackDecoration(target.pos, target.node, 'measurement-failed', {
                signature,
                sourceGeometry,
              }),
            ],
          });
          logDebug('typography', '段落测量失败，使用原生排版', { reason: 'measurement-failed' });
        }
        const through = index + 1;
        // 交互期间按帧递进更新，不等待整屏或另一编辑器的计算。
        if (
          pending.length &&
          (interactivePass
            ? performance.now() - publishedAt >= 6
            : through === priorityCount ||
              (through > priorityCount && performance.now() - publishedAt >= 16))
        ) {
          if (!(await commit())) break;
        }
      }
      const succeeded = !stale() && (await commit());
      const needsRetry =
        skippedGeometry ||
        geometryAtStart !== interaction.geometryRevision ||
        layoutRequestAtStart !== layoutRequestRevision;
      if (succeeded) {
        pendingSourceLayout = false;
        if (needsRetry) viewportOnly = true;
        else if (!interactivePass) {
          completed = token;
          completedGeometryRevision = geometryAtStart;
          completedEditorGeometry = editorGeometrySignature();
          geometryRecovery = false;
          viewportOnly = false;
          finishGeometryProjection();
          interactionSubscription?.complete(interaction.generation);
        }
      } else cancelledPasses++;
      logDebug('typography', '排版批次耗时', {
        revision: token,
        generation,
        geometryRevision: geometryAtStart,
        paragraphs: targets.length,
        priorityParagraphs: priorityCount,
        ...Object.fromEntries(
          Object.entries(timings).map(([name, value]) => [name, Math.round(value)]),
        ),
        totalMs: Math.round(performance.now() - startedAt),
        cancelled: !succeeded,
        cancelledPasses,
      });
    })().finally(() => {
      running = null;
      if (
        !destroyed &&
        !isComposing() &&
        !dragging &&
        (viewportOnly ||
          (!isPreviewing() &&
            geometryRecovery &&
            (completed !== revision || completedGeometryRevision !== interaction.geometryRevision)))
      )
        queueRunFrame();
    });
    await running;
  };
  const clearActive = (event: Event) => {
    if (composing || view.composing) return;
    const keyboard = event as KeyboardEvent;
    if (
      event.type === 'keydown' &&
      [
        'ArrowLeft',
        'ArrowRight',
        'ArrowUp',
        'ArrowDown',
        'Home',
        'End',
        'PageUp',
        'PageDown',
      ].includes(keyboard.key)
    ) {
      // Keep the current DOM stable until native selection changes from navigation have settled.
      if (getTypographyOptions().enabled && completed !== revision) schedule();
      return;
    }
    if (
      event.type === 'keydown' &&
      (keyboard.ctrlKey ||
        keyboard.metaKey ||
        keyboard.altKey ||
        (keyboard.key.length > 1 &&
          !['Backspace', 'Delete', 'Enter', 'Process'].includes(keyboard.key)))
    )
      return;
    // Cancel obsolete bindings, but keep mapped spacing/breaks until the local
    // post-transaction layout replaces them before paint. Clearing them here
    // makes ordinary mixed text shrink to native spacing and then expand again.
    schedule();
  };
  const compositionStart = () => {
    composing = true;
    revision++;
    clearTimeout(timer);
  };
  const compositionEnd = () => {
    composing = false;
    clearDisabledLayout();
    queueSourceLayout();
    schedule();
  };
  const pointerDown = () => {
    dragging = true;
    dragSelectionStarted = false;
    revision++;
    clearTimeout(timer);
  };
  const pointerUp = () => {
    if (dragging) {
      dragging = false;
      dragSelectionStarted = false;
      clearDisabledLayout();
      queueSourceLayout();
      schedule();
    }
  };
  view.dom.addEventListener('keydown', clearActive, true);
  view.dom.addEventListener('beforeinput', clearActive, true);
  for (const event of ['paste', 'cut', 'drop']) view.dom.addEventListener(event, clearActive, true);
  view.dom.addEventListener('compositionstart', compositionStart);
  view.dom.addEventListener('compositionend', compositionEnd);
  view.dom.addEventListener('pointerdown', pointerDown);
  window.addEventListener('pointerup', pointerUp);
  window.addEventListener('pointercancel', pointerUp);
  window.addEventListener('blur', pointerUp);
  view.dom.addEventListener('load', contentLoaded, true);
  const observer =
    typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver(() => {
          const width = view.dom.clientWidth;
          if (width !== lastWidth) {
            lastWidth = width;
            projectCurrentWidth();
            requestVisibleLayout();
          }
        })
      : null;
  const objectObserver =
    typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver((entries) => {
          let changed = false;
          for (const entry of entries) {
            // Width detects formula/image completion without reacting to our line-height decorations.
            const size =
              entry.target.tagName === 'IMG'
                ? `${entry.contentRect.width}:${entry.contentRect.height}`
                : `${entry.contentRect.width}`;
            const previous = objectSizes.get(entry.target);
            objectSizes.set(entry.target, size);
            if (previous !== undefined && previous !== size) changed = true;
          }
          if (changed) geometryChanged();
        })
      : null;
  observer?.observe(view.dom);
  document.fonts?.addEventListener('loadingdone', fontsChanged);
  const unsubscribe = subscribeTypography(() => {
    cacheReset();
    if (getTypographyOptions().enabled) schedule();
  });
  const cacheReset = () => {
    solver.reset();
    measureCache.clear();
    geometryRevision++;
    revision++;
    cachedDoc = view.state.doc;
    if (!getTypographyOptions().enabled) {
      pendingDisableClear = true;
      clearDisabledLayout();
      finishGeometryProjection();
    }
    if (!getTypographyOptions().enabled) {
      clearTimeout(timer);
      if (!pendingDisableClear) {
        completed = revision;
        completedGeometryRevision = interaction.geometryRevision;
      }
      interactionSubscription?.complete(interaction.generation);
    }
  };
  const viewportScrolled = () => {
    const scrollTop = pane?.scrollTop ?? 0;
    if (scrollTop === viewportScrollTop) return;
    viewportScrollTop = scrollTop;
    zoomGestureAnchor = null;
    // A scroll arriving before the next frame supersedes the old reading anchor.
    if (geometryAnchorFrame !== null || zoomAnchorQueued) {
      if (geometryAnchorFrame !== null) cancelAnimationFrame(geometryAnchorFrame);
      geometryAnchorFrame = null;
      pendingGeometryAnchor = null;
    }
    if (!getTypographyOptions().enabled || destroyed || isComposing() || dragging) return;
    if (isPreviewing() || geometryRecovery) {
      requestVisibleLayout();
    }
  };
  pane?.addEventListener('scroll', viewportScrolled, { passive: true });
  interactionSubscription = subscribeTypographyInteraction(
    (snapshot) => {
      if (snapshot.generation < interaction.generation) return;
      const previous = interaction;
      interaction = snapshot;
      if (destroyed) return;
      if (!isZoomPreview()) zoomGestureAnchor = null;
      if (previous.phase !== 'preview' && snapshot.phase === 'preview') {
        clearTimeout(timer);
        if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
        layoutFrame = null;
      }
      if (snapshot.geometryRevision !== previous.geometryRevision) {
        if (preparedGeometryRevision !== snapshot.geometryRevision) projectCurrentWidth();
        // 完整布局已就绪且逻辑几何不变时，纯 zoom 沿用断行，仅缩放和保护视角。
        if (
          snapshot.geometrySource === 'zoom' &&
          completed === revision &&
          (zoomReusePending || completedGeometryRevision === previous.geometryRevision)
        ) {
          zoomReusePending = true;
          viewportOnly = true;
          queueRunFrame();
        } else {
          zoomReusePending = false;
          requestVisibleLayout();
        }
      } else if (snapshot.phase === 'preview' && viewportOnly) {
        queueRunFrame();
      } else if (snapshot.phase === 'settling') {
        if (!getTypographyOptions().enabled) {
          queueMicrotask(() => interactionSubscription?.complete(snapshot.generation));
        } else {
          geometryRecovery = true;
          queueRunFrame();
        }
      }
    },
    {
      geometryParticipant: () =>
        view.dom.isConnected &&
        getTypographyOptions().enabled &&
        (geometryAnchorFrame !== null ||
          !pane ||
          (pane.clientHeight > 0 && view.dom.clientWidth > 0)),
      beforeGeometryChange: (snapshot) => {
        if (geometryAnchorFrame === null && !zoomAnchorQueued) {
          if (snapshot.geometrySource === 'zoom') {
            const anchorMoved =
              snapshot.anchor?.left !== interaction.anchor?.left ||
              snapshot.anchor?.top !== interaction.anchor?.top;
            // One gesture keeps the same semantic reading position. Repeated
            // hit-testing across per-glyph DOM can cost more than the zoom itself.
            if (!zoomGestureAnchor || zoomGestureAnchor.doc !== view.state.doc || anchorMoved)
              zoomGestureAnchor = captureViewportAnchor(view, snapshot.anchor);
            pendingGeometryAnchor = zoomGestureAnchor;
          } else {
            zoomGestureAnchor = null;
            pendingGeometryAnchor = captureViewportAnchor(view, snapshot.anchor);
          }
        }
        projectCurrentWidth(snapshot.geometrySource === 'zoom');
        preparedGeometryRevision = snapshot.geometryRevision;
        return () => {
          if (snapshot.geometrySource === 'zoom') {
            // Restore reading position before this zoom paints, while coalescing
            // repeated inputs in the same task. No paragraph solving runs here.
            if (!zoomAnchorQueued) {
              zoomAnchorQueued = true;
              queueMicrotask(() => {
                zoomAnchorQueued = false;
                if (!destroyed && pendingGeometryAnchor) restoreGeometryAnchor();
              });
            }
            return;
          }
          // CSS still follows every input immediately. Coalesce the subsequent
          // layout read and scroll correction so rapid inputs share one frame.
          if (geometryAnchorFrame !== null) return;
          if (typeof requestAnimationFrame === 'function' && !document.hidden)
            geometryAnchorFrame = requestAnimationFrame(restoreGeometryAnchor);
          else restoreGeometryAnchor();
        };
      },
    },
  );
  const controller: TypographyController = {
    async flush() {
      clearTimeout(timer);
      const deadline = Date.now() + 30_000;
      while (!destroyed) {
        if (isPreviewing() || isComposing() || dragging) {
          if (Date.now() > deadline) throw new Error('排版仍在等待编辑完成');
          await new Promise((resolve) => setTimeout(resolve, 25));
          continue;
        }
        if (completed === revision && completedGeometryRevision === interaction.geometryRevision)
          return;
        if (Date.now() > deadline) throw new Error('排版未能在时限内稳定');
        await run();
        if (completed !== revision || completedGeometryRevision !== interaction.geometryRevision)
          await new Promise((resolve) => setTimeout(resolve, 25));
      }
    },
    invalidate: schedule,
    destroy() {
      destroyed = true;
      revision++;
      clearTimeout(timer);
      finishPublishFrame?.();
      if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
      if (geometryAnchorFrame !== null) cancelAnimationFrame(geometryAnchorFrame);
      pendingGeometryAnchor = null;
      zoomGestureAnchor = null;
      view.dom.removeAttribute('data-kp-reflow');
      geometryProjectionStyle?.remove();
      interactionSubscription?.destroy();
      pane?.removeEventListener('scroll', viewportScrolled);
      measureCache.clear();
      solver.destroy();
      observer?.disconnect();
      objectObserver?.disconnect();
      visibilityObserver?.disconnect();
      unsubscribe();
      document.fonts?.removeEventListener('loadingdone', fontsChanged);
      view.dom.removeEventListener('keydown', clearActive, true);
      view.dom.removeEventListener('beforeinput', clearActive, true);
      for (const event of ['paste', 'cut', 'drop'])
        view.dom.removeEventListener(event, clearActive, true);
      view.dom.removeEventListener('compositionstart', compositionStart);
      view.dom.removeEventListener('compositionend', compositionEnd);
      view.dom.removeEventListener('pointerdown', pointerDown);
      window.removeEventListener('pointerup', pointerUp);
      window.removeEventListener('pointercancel', pointerUp);
      window.removeEventListener('blur', pointerUp);
      view.dom.removeEventListener('load', contentLoaded, true);
      controllers.delete(view);
    },
  };
  controllers.set(view, controller);
  initializing = false;
  if (pendingDisableClear) queueMicrotask(clearDisabledLayout);
  schedule();
  return {
    update() {
      if (!publishing) clearDisabledLayout();
      if (
        !inlineSourceEditingPluginKey.getState(view.state) &&
        dragging &&
        !view.state.selection.empty
      )
        dragSelectionStarted = true;
      const sourceGeometry = inlineSourceGeometrySnapshot(view);
      if (sourceGeometry !== cachedSourceGeometry) {
        cachedSourceGeometry = sourceGeometry;
        if (view.state.doc === cachedDoc) pendingSourceLayout = true;
        schedule();
      }
      if (view.state.doc !== cachedDoc) {
        cachedDoc = view.state.doc;
        pendingSourceLayout = false;
        pendingContentLayout = view.editable;
        schedule();
      }
      queueSourceLayout();
    },
    destroy: controller.destroy,
  };
}

function fallbackDecoration(
  pos: number,
  node: ProseMirrorNode,
  reason: string,
  cache?: {
    signature: string;
    sourceGeometry: string;
    editingFallback?: boolean;
    originalStyle?: string;
  },
) {
  return Decoration.node(
    pos,
    pos + node.nodeSize,
    {
      'data-kp-layout': 'fallback',
      'data-kp-reason': reason,
      ...(cache?.editingFallback
        ? {
            'data-kp-original-style': cache.originalStyle ?? '',
            style: 'white-space:break-spaces;overflow-wrap:anywhere;',
          }
        : {}),
    },
    cache
      ? {
          typographySignature: cache.signature,
          typographySourceGeometry: cache.sourceGeometry,
          paragraph: node,
          editingFallback: cache.editingFallback,
        }
      : undefined,
  );
}

function renderDecorations(
  view: EditorView,
  pos: number,
  node: ProseMirrorNode,
  measured: MeasuredParagraph,
  result: ParagraphLayout,
  signature: string,
  sourceGeometry: string,
  geometryProjection: number,
): Decoration[] | null {
  const plan = createRenderPlan(measured, result);
  const dom = view.nodeDOM(pos) as HTMLElement;
  if (!(dom instanceof HTMLElement)) return null;
  const positions = new Map<MeasuredParagraph['bindings'][number], { from: number; to: number }>();
  const textPositions = new Map<Node, { from: number; to: number }>();
  const bindingsFrom = new Map<number, MeasuredParagraph['bindings'][number]>();
  const bindingsTo = new Map<number, MeasuredParagraph['bindings'][number]>();
  let previousEnd = pos + 1;
  const contentEnd = pos + 1 + node.content.size;
  for (const binding of measured.bindings) {
    if (!dom.contains(binding.node)) return null;
    if (
      !binding.atomic &&
      (binding.node.nodeType !== Node.TEXT_NODE ||
        binding.offset < 0 ||
        binding.endOffset > (binding.node.textContent?.length ?? 0))
    )
      return null;
    let textRange = textPositions.get(binding.node);
    if (!binding.atomic && !textRange) {
      textRange = {
        from: view.posAtDOM(binding.node, 0),
        to: view.posAtDOM(binding.node, binding.node.textContent?.length ?? 0),
      };
      textPositions.set(binding.node, textRange);
    }
    // Many glyphs share one semantic text node. Its offsets are linear when
    // both ends map to the complete text; custom projections use the full mapper.
    const linearText =
      !binding.atomic &&
      textRange &&
      textRange.to - textRange.from === binding.node.textContent?.length
        ? textRange
        : undefined;
    const from = linearText
      ? linearText.from + binding.offset
      : view.posAtDOM(binding.node, binding.offset);
    const to = binding.atomic
      ? from + (binding.sourceSize ?? 1)
      : linearText
        ? linearText.from + binding.endOffset
        : view.posAtDOM(binding.node, binding.endOffset);
    const size = binding.atomic ? (binding.sourceSize ?? 1) : binding.endOffset - binding.offset;
    // PM can return -1 for a detached node, or clamp an obsolete text offset.
    // Reject the whole plan instead of publishing overlapping/wrong-position units.
    if (from < previousEnd || to > contentEnd || from >= to || to - from !== size) return null;
    positions.set(binding, { from, to });
    if (binding.from < binding.to) {
      if (!bindingsFrom.has(binding.from)) bindingsFrom.set(binding.from, binding);
      bindingsTo.set(binding.to, binding);
    }
    previousEnd = to;
  }
  const originalStyle =
    dom?.getAttribute('data-kp-original-style') ?? dom?.getAttribute('style') ?? '';
  const decorations = [
    Decoration.node(
      pos,
      pos + node.nodeSize,
      {
        'data-kp-layout': 'ready',
        'data-kp-geometry': String(geometryProjection),
        'data-kp-alignment': result.status === 'ready' ? (result.alignment ?? 'justify') : '',
        'data-kp-original-style': originalStyle,
        style: PARAGRAPH_CSS,
      },
      { typographySignature: signature, typographySourceGeometry: sourceGeometry, paragraph: node },
    ),
  ];
  const naturalLineHeight = parseFloat(getComputedStyle(dom).lineHeight);
  for (const style of plan.styles) {
    const binding = bindingsFrom.get(style.from);
    if (!binding) continue;
    // Ordinary text already has the paragraph's line box. Avoid splitting every
    // glyph into a span when KP contributes neither spacing nor extra height.
    if (
      !binding.atomic &&
      style.left === 0 &&
      style.right === 0 &&
      style.height <= naturalLineHeight
    )
      continue;
    const { from: start, to: end } = positions.get(binding)!;
    if (start < end) {
      // Keep expanded text spacing inside the selection's painted box. Margins
      // leave white seams between selected characters; negative compression and
      // atomic objects retain their existing geometry.
      const textGap = binding.atomic ? 0 : Math.max(0, style.right);
      const attributes: Record<string, string> = {
        class: 'kp-unit',
        style: unitStyleCss(textGap > 0 ? { ...style, right: 0 } : style),
      };
      if (textGap > 0) attributes.style += `padding-right:${textGap.toFixed(3)}px;`;
      if (binding.atomic) attributes.nodeName = 'span';
      decorations.push(Decoration.inline(start, end, attributes));
    }
  }
  for (const br of plan.breaks) {
    const binding = bindingsTo.get(br.at);
    if (!binding) continue;
    const position = positions.get(binding)!.to;
    decorations.push(
      Decoration.widget(
        position,
        () => {
          const span = document.createElement('span');
          span.dataset.kpOwned = 'break';
          span.className = 'kp-break';
          span.setAttribute('aria-hidden', 'true');
          if (br.hyphen) span.append(document.createTextNode('-'));
          span.append(document.createElement('br'));
          return span;
        },
        { side: -1, relaxedSide: true, key: `kp:${position}:${br.hyphen}`, ignoreSelection: true },
      ),
    );
  }
  return decorations;
}
