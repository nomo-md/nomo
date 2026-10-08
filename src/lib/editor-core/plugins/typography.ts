import { Plugin, PluginKey, TextSelection, type Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet, type EditorView } from 'prosemirror-view';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { getTypographyOptions, subscribeTypography } from '../../typography/options';
import {
  bindingAt,
  measureParagraph,
  measureParagraphSync,
  type MeasuredParagraph,
} from '../../typography/measure';
import { createTypographySolver } from '../../typography/solver';
import { layoutParagraph } from '../../typography/knuthPlass';
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

function createViewController(view: EditorView) {
  const solver = createTypographySolver();
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
  let sourceLayoutQueued = false;
  let dragSelectionStarted = false;

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
  const publish = (decorations: Decoration[], sourceOnly = false) => {
    if (destroyed || isComposing() || (sourceOnly ? !canLayoutSource() : dragging)) return;
    publishing = true;
    try {
      view.dispatch(
        view.state.tr
          .setMeta(meta, DecorationSet.create(view.state.doc, decorations))
          .setMeta('addToHistory', false),
      );
    } finally {
      publishing = false;
    }
  };
  const schedule = () => {
    if (destroyed || publishing) return;
    revision++;
    clearTimeout(timer);
    timer = setTimeout(() => {
      void run();
    }, 150);
  };
  const geometryChanged = () => {
    geometryRevision++;
    schedule();
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
    if (sourceLayoutQueued || !pendingSourceLayout || destroyed) return;
    sourceLayoutQueued = true;
    // PM has installed the newly visible source DOM by this checkpoint, but the
    // browser has not painted it. Publish its spacing and breaks in the same frame.
    queueMicrotask(() => {
      sourceLayoutQueued = false;
      if (!pendingSourceLayout || !canLayoutSource()) return;
      pendingSourceLayout = false;
      const options = getTypographyOptions();
      if (!options.enabled) return;
      const value = key.getState(view.state);
      if (!value) return;
      const token = revision;
      const doc = view.state.doc;
      const deadline = performance.now() + 32;
      const targets = value.find().filter((decoration) => {
        const paragraph = decoration.spec.paragraph as ProseMirrorNode | undefined;
        return (
          paragraph?.type.name === 'paragraph' &&
          doc.nodeAt(decoration.from) === paragraph &&
          decoration.spec.typographySourceGeometry !== undefined &&
          decoration.spec.typographySourceGeometry !==
            paragraphSourceGeometry(view, decoration.from, paragraph)
        );
      });
      const changedRanges: { from: number; to: number }[] = [];
      const replacements: Decoration[] = [];
      for (let index = 0; index < targets.length; index++) {
        const target = targets[index];
        const node = target.spec.paragraph as ProseMirrorNode;
        const element = view.nodeDOM(target.from);
        if (!(element instanceof HTMLElement)) continue;
        const sourceGeometry = paragraphSourceGeometry(view, target.from, node);
        const signature = paragraphSignature(element, options, sourceGeometry);
        let rendered: Decoration[] | null = null;
        if (index < 2 && node.content.size <= 2048 && performance.now() < deadline - 4) {
          try {
            const measureBudget = Math.min(24, deadline - performance.now() - 4);
            const measured = measureParagraphSync(element, options, measureBudget);
            if (measured && performance.now() < deadline) {
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
                );
              }
            }
          } catch {
            // The native fallback below is cached for this exact visibility state.
          }
        }
        replacements.push(
          ...(rendered ?? [
            fallbackDecoration(target.from, node, 'source-visibility-fallback', {
              signature,
              sourceGeometry,
            }),
          ]),
        );
        changedRanges.push({ from: target.from, to: target.to });
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
    if (destroyed || isComposing() || dragging) return;
    if (sourceLayoutQueued) {
      await Promise.resolve();
      return run();
    }
    if (completed === revision) return;
    if (running) {
      await running;
      if (!destroyed) return run();
      return;
    }
    const token = revision;
    const doc = view.state.doc;
    const options = getTypographyOptions();
    refreshTypographyBreaks(view);
    if (!options.enabled) {
      publish([]);
      completed = token;
      return;
    }
    running = (async () => {
      const targets: { node: ProseMirrorNode; pos: number; element: HTMLElement }[] = [];
      doc.descendants((node, pos) => {
        if (node.type.name !== 'paragraph' || !node.content.size) return;
        const element = view.nodeDOM(pos);
        if (element instanceof HTMLElement && element.clientWidth > 0)
          targets.push({ node, pos, element });
      });
      const distance = (element: HTMLElement) => {
        const rect = element.getBoundingClientRect();
        return rect.bottom < 0
          ? -rect.bottom
          : rect.top > window.innerHeight
            ? rect.top - window.innerHeight
            : 0;
      };
      targets.sort((a, b) => distance(a.element) - distance(b.element));
      const decorations: Decoration[] = [];
      for (const element of view.dom.querySelectorAll('img,.math-inline')) {
        if (!observedObjects.has(element)) {
          observedObjects.add(element);
          objectObserver?.observe(element);
        }
      }
      for (const element of observedObjects)
        if (!view.dom.contains(element)) {
          objectObserver?.unobserve(element);
          observedObjects.delete(element);
        }
      const retainedByPosition = new Map(
        targets.map((target) => [
          target.pos,
          key
            .getState(view.state)
            ?.find(target.pos, target.pos + target.node.nodeSize)
            .filter((d) => d.from >= target.pos && d.to <= target.pos + target.node.nodeSize) ?? [],
        ]),
      );
      let publishedAt = performance.now();
      for (let index = 0; index < targets.length; index++) {
        const target = targets[index];
        if (destroyed || revision !== token || view.state.doc !== doc || composing || dragging)
          return;
        try {
          const sourceGeometry = paragraphSourceGeometry(view, target.pos, target.node);
          const signature = paragraphSignature(target.element, options, sourceGeometry);
          const retained = retainedByPosition.get(target.pos) ?? [];
          if (
            retained.some(
              (d) =>
                d.from === target.pos &&
                d.to === target.pos + target.node.nodeSize &&
                d.spec.typographySignature === signature &&
                d.spec.paragraph === target.node,
            )
          ) {
            decorations.push(...retained);
            continue;
          }
          const measured = await measureParagraph(target.element, options);
          if (!measured) {
            decorations.push(
              fallbackDecoration(target.pos, target.node, 'unsupported-or-unmeasurable'),
            );
            continue;
          }
          const startedAt = performance.now();
          const result = await solver.solve(measured.input);
          if (revision !== token || destroyed || view.state.doc !== doc) return;
          logDebug('typography', '段落排版完成', {
            revision: token,
            durationMs: Math.round(performance.now() - startedAt),
            candidates: result.candidates,
            status: result.status,
          });
          if (result.status === 'fallback') {
            logDebug('typography', '段落使用原生排版', {
              reason: result.reason,
              candidates: result.candidates,
            });
            decorations.push(fallbackDecoration(target.pos, target.node, result.reason));
            continue;
          }
          const rendered = renderDecorations(
            view,
            target.pos,
            target.node,
            measured,
            result,
            signature,
            sourceGeometry,
          );
          decorations.push(
            ...(rendered ?? [fallbackDecoration(target.pos, target.node, 'stale-dom-bindings')]),
          );
          if (performance.now() - publishedAt >= 16) {
            // Publish complete paragraphs only. Unprocessed paragraphs retain their prior DOM.
            const remaining = targets
              .slice(index + 1)
              .flatMap((item) => retainedByPosition.get(item.pos) ?? []);
            publish([...decorations, ...remaining]);
            publishedAt = performance.now();
          }
        } catch {
          decorations.push(fallbackDecoration(target.pos, target.node, 'measurement-failed'));
          logDebug('typography', '段落测量失败，使用原生排版', { reason: 'measurement-failed' });
        }
      }
      if (revision === token && view.state.doc === doc) {
        publish(decorations);
        completed = token;
      }
    })().finally(() => {
      running = null;
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
    // Content editing may merge/detach measured text nodes. Cancel in-flight
    // plans before removing their decoration DOM, even if the doc is unchanged.
    schedule();
    const selection = view.state.selection;
    const value = key.getState(view.state);
    if (value) {
      const from = selection.$from.start();
      const to = selection.$to.end();
      const remove = value.find(Math.max(0, from - 1), to + 1);
      if (remove.length) {
        publishing = true;
        try {
          view.dispatch(
            view.state.tr.setMeta(meta, value.remove(remove)).setMeta('addToHistory', false),
          );
        } finally {
          publishing = false;
        }
      }
    }
  };
  const compositionStart = () => {
    composing = true;
    revision++;
    clearTimeout(timer);
  };
  const compositionEnd = () => {
    composing = false;
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
  view.dom.addEventListener('load', geometryChanged, true);
  const observer =
    typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver(() => {
          const width = view.dom.clientWidth;
          if (width !== lastWidth) {
            lastWidth = width;
            schedule();
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
  document.fonts?.addEventListener('loadingdone', geometryChanged);
  const unsubscribe = subscribeTypography(() => {
    cacheReset();
    schedule();
  });
  const cacheReset = () => {
    solver.reset();
    geometryRevision++;
    cachedDoc = view.state.doc;
    publish([]);
  };
  const controller: TypographyController = {
    async flush() {
      clearTimeout(timer);
      const deadline = Date.now() + 30_000;
      while (isComposing() || dragging) {
        if (Date.now() > deadline) throw new Error('排版仍在等待编辑完成');
        await new Promise((resolve) => setTimeout(resolve, 25));
      }
      while (!destroyed && completed !== revision) {
        if (Date.now() > deadline) throw new Error('排版未能在时限内稳定');
        await run();
        if (completed !== revision) await new Promise((resolve) => setTimeout(resolve, 25));
      }
    },
    invalidate: schedule,
    destroy() {
      destroyed = true;
      revision++;
      clearTimeout(timer);
      solver.destroy();
      observer?.disconnect();
      objectObserver?.disconnect();
      unsubscribe();
      document.fonts?.removeEventListener('loadingdone', geometryChanged);
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
      view.dom.removeEventListener('load', geometryChanged, true);
      controllers.delete(view);
    },
  };
  controllers.set(view, controller);
  schedule();
  return {
    update() {
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
  cache?: { signature: string; sourceGeometry: string },
) {
  return Decoration.node(
    pos,
    pos + node.nodeSize,
    {
      'data-kp-layout': 'fallback',
      'data-kp-reason': reason,
    },
    cache
      ? {
          typographySignature: cache.signature,
          typographySourceGeometry: cache.sourceGeometry,
          paragraph: node,
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
): Decoration[] | null {
  const plan = createRenderPlan(measured, result);
  const dom = view.nodeDOM(pos) as HTMLElement;
  if (!(dom instanceof HTMLElement)) return null;
  const positions = new Map<MeasuredParagraph['bindings'][number], { from: number; to: number }>();
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
    const from = view.posAtDOM(binding.node, binding.offset);
    const to = binding.atomic
      ? from + (binding.sourceSize ?? 1)
      : view.posAtDOM(binding.node, binding.endOffset);
    const size = binding.atomic ? (binding.sourceSize ?? 1) : binding.endOffset - binding.offset;
    // PM can return -1 for a detached node, or clamp an obsolete text offset.
    // Reject the whole plan instead of publishing overlapping/wrong-position units.
    if (from < previousEnd || to > contentEnd || from >= to || to - from !== size) return null;
    positions.set(binding, { from, to });
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
        'data-kp-original-style': originalStyle,
        style: PARAGRAPH_CSS,
      },
      { typographySignature: signature, typographySourceGeometry: sourceGeometry, paragraph: node },
    ),
  ];
  for (const style of plan.styles) {
    const binding = bindingAt(measured.bindings, style.from);
    if (!binding) continue;
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
    const binding = bindingAt(measured.bindings, br.at, 'before');
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
