import { Plugin, PluginKey, type Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet, type EditorView } from 'prosemirror-view';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { getTypographyOptions, subscribeTypography } from '../../typography/options';
import { bindingAt, measureParagraph, type MeasuredParagraph } from '../../typography/measure';
import { createTypographySolver } from '../../typography/solver';
import { createRenderPlan, PARAGRAPH_CSS, unitStyleCss } from '../../typography/renderPlan';
import type { ParagraphLayout, TypographyController } from '../../typography/types';
import { logDebug } from '../../services/logger';
import { refreshTypographyBreaks } from '../nodeViews/TypographyBreakNodeView';

const key = new PluginKey<DecorationSet>('nomo-typography');
const meta = 'nomo:typography';
const controllers = new WeakMap<EditorView, TypographyController>();
export const isTypographyTransaction = (transaction: Transaction) =>
  transaction.getMeta(meta) !== undefined;
export const flushEditorTypography = (view: EditorView) =>
  controllers.get(view)?.flush() ?? Promise.resolve();

export function typographyPlugin() {
  return new Plugin<DecorationSet>({
    key,
    state: {
      init: () => DecorationSet.empty,
      apply: (tr, value) => tr.getMeta(meta) ?? value.map(tr.mapping, tr.doc),
    },
    props: { decorations: (state) => key.getState(state) },
    view: (view) => createViewController(view),
  });
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

  const publish = (decorations: Decoration[]) => {
    if (destroyed || composing || view.composing || dragging) return;
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
  const run = async (): Promise<void> => {
    if (destroyed || composing || view.composing || dragging) return;
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
          const font = getComputedStyle(target.element);
          const signature = JSON.stringify([
            options,
            geometryRevision,
            target.element.clientWidth,
            font.fontFamily,
            font.fontSize,
            font.fontWeight,
            font.fontStyle,
            font.fontStretch,
            font.lineHeight,
            font.letterSpacing,
            font.wordSpacing,
          ]);
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
          decorations.push(
            ...renderDecorations(view, target.pos, target.node, measured, result, signature),
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
      (keyboard.ctrlKey ||
        keyboard.metaKey ||
        keyboard.altKey ||
        (keyboard.key.length > 1 &&
          !['Backspace', 'Delete', 'Enter', 'Process'].includes(keyboard.key)))
    )
      return;
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
    schedule();
  };
  const compositionStart = () => {
    composing = true;
    revision++;
    clearTimeout(timer);
  };
  const compositionEnd = () => {
    composing = false;
    schedule();
  };
  const pointerDown = () => {
    dragging = true;
    revision++;
    clearTimeout(timer);
  };
  const pointerUp = () => {
    if (dragging) {
      dragging = false;
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
      while (composing || view.composing || dragging) {
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
      if (view.state.doc !== cachedDoc) {
        cachedDoc = view.state.doc;
        schedule();
      }
    },
    destroy: controller.destroy,
  };
}

function fallbackDecoration(pos: number, node: ProseMirrorNode, reason: string) {
  return Decoration.node(pos, pos + node.nodeSize, {
    'data-kp-layout': 'fallback',
    'data-kp-reason': reason,
  });
}

function renderDecorations(
  view: EditorView,
  pos: number,
  node: ProseMirrorNode,
  measured: MeasuredParagraph,
  result: ParagraphLayout,
  signature: string,
): Decoration[] {
  const plan = createRenderPlan(measured, result);
  const dom = view.nodeDOM(pos) as HTMLElement;
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
      { typographySignature: signature, paragraph: node },
    ),
  ];
  for (const style of plan.styles) {
    const binding = bindingAt(measured.bindings, style.from);
    if (!binding) continue;
    const start = view.posAtDOM(binding.node, binding.offset);
    const end = binding.atomic
      ? start + (binding.sourceSize ?? 1)
      : view.posAtDOM(binding.node, binding.endOffset);
    if (start < end) {
      const attributes: Record<string, string> = { class: 'kp-unit', style: unitStyleCss(style) };
      if (binding.atomic) attributes.nodeName = 'span';
      decorations.push(Decoration.inline(start, end, attributes));
    }
  }
  for (const br of plan.breaks) {
    const binding = bindingAt(measured.bindings, br.at, 'before');
    if (!binding) continue;
    const position = binding.atomic
      ? view.posAtDOM(binding.node, 0) + (binding.sourceSize ?? 1)
      : view.posAtDOM(binding.node, binding.endOffset);
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
