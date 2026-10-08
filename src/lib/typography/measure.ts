/// <reference lib="es2022.intl" />
import { createParagraphInput, softBreakText, supportedText } from './rules';
import type { LayoutItem, ParagraphInput, TypographyOptions } from './types';

export interface DomBinding {
  node: Node;
  offset: number;
  endOffset: number;
  from: number;
  to: number;
  atomic: boolean;
  width: number;
  sourceSize?: number;
}

export interface MeasuredParagraph {
  input: ParagraphInput;
  bindings: DomBinding[];
  text: string;
}

const SKIP = '.ProseMirror-widget,.ProseMirror-trailingBreak,[data-kp-owned],script,style';
const ATOMIC =
  '.math-inline,.katex,.footnote-ref,.image-node,img,code,.pm-inline-source-code,input[type="checkbox"]';
const segmenter =
  typeof Intl.Segmenter === 'function'
    ? new Intl.Segmenter('zh', { granularity: 'grapheme' })
    : null;

function snapshotFont(style: CSSStyleDeclaration) {
  const font = {
    font: style.font,
    fontFamily: style.fontFamily,
    fontSize: style.fontSize,
    fontWeight: style.fontWeight,
    fontStyle: style.fontStyle,
    fontStretch: style.fontStretch,
    letterSpacing: style.letterSpacing,
    wordSpacing: style.wordSpacing,
    fontFeatureSettings: style.fontFeatureSettings,
    fontVariantLigatures: style.fontVariantLigatures,
  };
  return { ...font, key: Object.values(font).slice(1).join('|') };
}

/** DOM-free search uses real browser measurements collected here in bounded batches. */
export async function measureParagraph(
  element: HTMLElement,
  options: TypographyOptions,
): Promise<MeasuredParagraph | null> {
  const batches = measureParagraphBatches(element, options, 1_500);
  try {
    let step = batches.next();
    while (!step.done) {
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      step = batches.next();
    }
    return step.value;
  } finally {
    batches.return(null);
  }
}

/** Cursor-only layout can finish in the current frame, or fall back when its budget expires. */
export function measureParagraphSync(
  element: HTMLElement,
  options: TypographyOptions,
  budgetMs = 24,
): MeasuredParagraph | null {
  const budget = Number.isFinite(budgetMs) ? budgetMs : 24;
  if (budget <= 0) return null;
  const batches = measureParagraphBatches(element, options, budget);
  try {
    let step = batches.next();
    while (!step.done) step = batches.next();
    return step.value;
  } finally {
    batches.return(null);
  }
}

function* measureParagraphBatches(
  element: HTMLElement,
  options: TypographyOptions,
  budgetMs: number,
): Generator<void, MeasuredParagraph | null, void> {
  let elapsed = performance.now();
  const deadline = elapsed + budgetMs;
  if (!segmenter) return null;
  if (
    element.querySelector('input:not([type="checkbox"]),textarea,video,iframe,svg:not(.katex svg)')
  )
    return null;
  const style = getComputedStyle(element);
  if (style.writingMode !== 'horizontal-tb') return null;
  // Table column alignment and explicitly aligned HTML are semantic layout choices.
  if (['center', 'right', 'end', '-webkit-center', '-webkit-right'].includes(style.textAlign))
    return null;
  const width =
    element.clientWidth -
    (parseFloat(style.paddingLeft) || 0) -
    (parseFloat(style.paddingRight) || 0);
  if (width < 1) return null;
  const rootFont = parseFloat(style.fontSize) || 16;
  const lineHeight = parseFloat(style.lineHeight) || rootFont * 1.75;
  const computedStyles = new Map<HTMLElement, CSSStyleDeclaration>([[element, style]]);
  const readStyle = (node: HTMLElement) => {
    let computed = computedStyles.get(node);
    if (!computed) {
      computed = getComputedStyle(node);
      computedStyles.set(node, computed);
    }
    return computed;
  };
  const runs: { node: Node; text: string; atomic: boolean; soft?: boolean; hard?: boolean }[] = [];
  let timedOut = false;
  const walk = (node: Node) => {
    if (performance.now() > deadline) {
      timedOut = true;
      return;
    }
    if (node.nodeType === Node.TEXT_NODE) {
      if (node.textContent) runs.push({ node, text: node.textContent, atomic: false });
      return;
    }
    if (!(node instanceof HTMLElement)) return;
    if (node.matches(SKIP)) return;
    if (readStyle(node).display === 'none') return;
    if (node.matches('[data-nomo-break],br')) {
      runs.push({
        node,
        text: '\n',
        atomic: true,
        soft: node.dataset.nomoBreak === 'soft',
        hard: node.dataset.nomoBreak !== 'soft',
      });
      return;
    }
    if (node.matches(ATOMIC)) {
      runs.push({ node, text: '\ufffc', atomic: true });
      return;
    }
    for (const child of Array.from(node.childNodes)) {
      walk(child);
      if (timedOut) return;
    }
  };
  for (const child of Array.from(element.childNodes)) {
    walk(child);
    if (timedOut) return null;
  }
  if (runs.reduce((length, run) => length + run.text.length, 0) > 20_000) return null;
  if (
    !supportedText(
      runs
        .filter((r) => !r.atomic)
        .map((r) => r.text)
        .join(''),
    )
  )
    return null;
  // Read source styles before writing the sandbox. Existing KP decorations can
  // split a paragraph into one DOM text node per character with identical fonts.
  const fontSnapshots = new Map<HTMLElement, ReturnType<typeof snapshotFont>>();
  const runFonts: {
    parent: HTMLElement;
    font: ReturnType<typeof snapshotFont>;
    atomicStyles: (readonly [string, string])[];
  }[] = [];
  for (const run of runs) {
    if (performance.now() > deadline) return null;
    const parent =
      run.node.nodeType === Node.TEXT_NODE ? run.node.parentElement! : (run.node as HTMLElement);
    const computed = readStyle(parent);
    let font = fontSnapshots.get(parent);
    if (!font) {
      font = snapshotFont(computed);
      fontSnapshots.set(parent, font);
    }
    const atomicStyles = run.atomic
      ? [
          'font-family',
          'font-size',
          'font-weight',
          'font-style',
          'padding',
          'border',
          'vertical-align',
          'display',
        ].map((property) => [property, computed.getPropertyValue(property)] as const)
      : [];
    runFonts.push({ parent, font, atomicStyles });
  }
  if (performance.now() > deadline) return null;
  const units: LayoutItem[] = [];
  const bindings: DomBinding[] = [];
  const forced = new Set<number>();
  const sandbox = document.createElement('span');
  sandbox.dataset.kpOwned = 'measurement';
  sandbox.style.cssText =
    'position:fixed;left:-100000px;top:0;visibility:hidden;white-space:pre;pointer-events:none;';
  let from = 0;
  try {
    document.body.append(sandbox);
    const canvas = document.createElement('canvas').getContext('2d');
    for (let r = 0; r < runs.length; r++) {
      const run = runs[r];
      if (performance.now() > deadline) return null;
      const { parent, font, atomicStyles } = runFonts[r];
      const size = parseFloat(font.fontSize) || rootFont;
      sandbox.style.font = font.font;
      sandbox.style.fontFamily = font.fontFamily;
      sandbox.style.fontSize = font.fontSize;
      sandbox.style.fontWeight = font.fontWeight;
      sandbox.style.fontStyle = font.fontStyle;
      sandbox.style.fontStretch = font.fontStretch;
      if (canvas)
        canvas.font = `${font.fontStyle} ${font.fontWeight} ${font.fontSize} ${font.fontFamily}`;
      if (run.atomic) {
        let text = run.text;
        if (run.soft) text = softBreakText(runs[r - 1]?.text ?? '', runs[r + 1]?.text ?? '');
        const breakNode = run.soft || run.hard;
        let objectWidth = 0,
          objectAscent = 0,
          objectDescent = 0;
        if (!breakNode) {
          const copy = parent.cloneNode(true) as HTMLElement;
          const original = copy.getAttribute('data-kp-unit-original-style');
          if (original !== null) copy.style.cssText = original;
          // The measurement sandbox is outside .ProseMirror, so its descendants
          // cannot rely on editor-scoped display:none rules for source markers.
          copy
            .querySelectorAll('.pm-inline-source-marker.is-hidden')
            .forEach((node) => node.remove());
          copy.classList.remove('kp-unit');
          copy.querySelectorAll('.kp-unit').forEach((node) => node.replaceWith(...node.childNodes));
          for (const [property, value] of atomicStyles) copy.style.setProperty(property, value);
          sandbox.style.lineHeight = style.lineHeight;
          const marker = document.createElement('span');
          marker.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline;';
          sandbox.style.width = `${width}px`;
          sandbox.replaceChildren(copy, marker);
          const rect = copy.getBoundingClientRect();
          const baseline = marker.getBoundingClientRect().top;
          const copyStyle = getComputedStyle(copy);
          objectWidth =
            rect.width +
            (parseFloat(copyStyle.marginLeft) || 0) +
            (parseFloat(copyStyle.marginRight) || 0);
          objectAscent = Math.max(0, baseline - rect.top);
          objectDescent = Math.max(0, rect.bottom - baseline);
        }
        const natural = breakNode
          ? text === ' '
            ? (canvas?.measureText(' ').width ?? size * 0.25)
            : 0
          : objectWidth;
        // Atomic here means an unbroken layout box, not an atomic PM node. Its
        // real text (including hidden padding) still owns every caret position.
        const sourceSize = parent.matches('code,.pm-inline-source-code')
          ? (parent.textContent?.length ?? 1)
          : 1;
        bindings.push({
          node: run.node,
          offset: 0,
          endOffset: 1,
          from,
          to: from + sourceSize,
          atomic: true,
          width: natural,
          sourceSize,
        });
        const to = from + sourceSize;
        units.push({
          kind: breakNode ? 'glue' : 'box',
          from,
          to,
          text,
          width: natural,
          fontSize: size,
          ascent: breakNode ? 0 : Math.max(size * 0.8, objectAscent),
          descent: breakNode ? 0 : Math.max(size * 0.2, objectDescent),
          stretch: run.soft && text ? size * 0.5 : 0,
          shrink: run.soft ? natural * 0.4 : 0,
          discardable: !!breakNode,
        });
        from = to;
        if (run.hard) forced.add(from);
        continue;
      }
      // Match ProseMirror's editable text shaping in every output surface.
      sandbox.style.fontFeatureSettings = '"liga" 0';
      sandbox.style.fontVariantLigatures = 'none';
      sandbox.style.letterSpacing = font.letterSpacing;
      sandbox.style.wordSpacing = font.wordSpacing;
      sandbox.style.setProperty('text-autospace', 'no-autospace');
      sandbox.style.setProperty('text-spacing-trim', 'space-all');
      const firstRun = r;
      while (r + 1 < runs.length && !runs[r + 1].atomic && runFonts[r + 1].font.key === font.key)
        r++;
      const group = runs.slice(firstRun, r + 1);
      const text = group.map((part) => part.text).join('');
      let hyphenWidth: number | undefined;
      if (text.includes('\u00ad')) {
        sandbox.textContent = '-';
        const hyphenRange = document.createRange();
        hyphenRange.selectNodeContents(sandbox);
        hyphenWidth = hyphenRange.getBoundingClientRect().width;
      }
      // One write per shaped group avoids forced layout for every old KP wrapper.
      // Bindings still point into each original run, including its local offsets.
      sandbox.textContent = text;
      const textNode = sandbox.firstChild!;
      const range = document.createRange();
      range.setStart(textNode, 0);
      let previousWidth = 0;
      let contextLength = 0;
      for (const sourceRun of group) {
        for (const part of segmenter.segment(sourceRun.text)) {
          const end = part.index + part.segment.length;
          range.setEnd(textNode, contextLength + end);
          const prefixWidth = range.getBoundingClientRect().width;
          const natural = Math.max(0, prefixWidth - previousWidth);
          previousWidth = prefixWidth;
          const metrics = canvas?.measureText(part.segment);
          const space = /^[ \t]+$/.test(part.segment);
          bindings.push({
            node: sourceRun.node,
            offset: part.index,
            endOffset: end,
            from,
            to: from + part.segment.length,
            atomic: false,
            width: natural,
          });
          units.push({
            kind: space ? 'glue' : part.segment === '\u00ad' ? 'penalty' : 'box',
            from,
            to: from + part.segment.length,
            text: part.segment,
            width: part.segment === '\u00ad' ? 0 : natural,
            fontSize: size,
            ascent: Math.max(size * 0.8, metrics?.actualBoundingBoxAscent ?? 0),
            descent: Math.max(size * 0.2, metrics?.actualBoundingBoxDescent ?? 0),
            leadingSpace: Math.max(0, -(metrics?.actualBoundingBoxLeft ?? 0)),
            trailingSpace: Math.max(0, natural - (metrics?.actualBoundingBoxRight ?? natural)),
            breakWidth: part.segment === '\u00ad' ? hyphenWidth : undefined,
            stretch: space ? size * 0.75 : 0,
            shrink: space ? natural * 0.4 : 0,
            weight: 2,
            discardable: space || part.segment === '\u00ad',
          });
          from += part.segment.length;
          const now = performance.now();
          if (now > deadline) return null;
          if (now - elapsed > 8) {
            yield;
            elapsed = performance.now();
            if (elapsed > deadline) return null;
          }
        }
        contextLength += sourceRun.text.length;
      }
    }
  } finally {
    sandbox.remove();
  }
  if (!units.length || performance.now() > deadline) return null;
  const input = createParagraphInput(units, width, lineHeight, options, forced);
  input.firstLineIndent = parseFloat(style.textIndent) || 0;
  const checkbox = element.querySelector<HTMLElement>('.task-checkbox-widget');
  if (checkbox) {
    const checkboxStyle = getComputedStyle(checkbox);
    const scale = element.getBoundingClientRect().width / (element.offsetWidth || 1) || 1;
    input.firstLineIndent +=
      checkbox.getBoundingClientRect().width / scale +
      (parseFloat(checkboxStyle.marginLeft) || 0) +
      (parseFloat(checkboxStyle.marginRight) || 0);
  }
  if (performance.now() > deadline) return null;
  return { input, bindings, text: runs.map((run) => run.text).join('') };
}

/** Visual coordinates retain both sides of zero-width source boundaries. */
export function bindingAt(
  bindings: DomBinding[],
  offset: number,
  affinity: 'before' | 'after' = 'after',
): DomBinding | undefined {
  return affinity === 'before'
    ? [...bindings].reverse().find((binding) => binding.from < offset && binding.to >= offset)
    : (bindings.find((binding) => binding.from <= offset && binding.to > offset) ??
        bindings.at(-1));
}
