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

interface PreparedRun {
  node: Node;
  text: string;
  atomic: boolean;
  soft?: boolean;
  hard?: boolean;
  font: ReturnType<typeof snapshotFont>;
  hyphenatable: boolean;
  object?: {
    projection: HTMLElement;
    width: number;
    ascent: number;
    descent: number;
    sourceSize: number;
    code: boolean;
  };
}

/** 目标 CSS 仍生效时取得的标量快照；异步测量不再读取原段落的样式或几何。 */
export interface PreparedParagraphMeasurement {
  width: number;
  lineHeight: number;
  rootFont: number;
  firstLineIndent: number;
  runs: readonly PreparedRun[];
  text: string;
  hasEnglishWords: boolean;
}

export interface TypographyMeasureCache {
  clear(): void;
}

export interface ParagraphMeasurementContext {
  prepared?: PreparedParagraphMeasurement | null;
  cache?: TypographyMeasureCache;
}

interface GraphemeMetrics {
  prefixWidth: number;
  ascent: number;
  descent: number;
  left: number;
  right: number;
}

interface ShapedGroup {
  metrics: readonly GraphemeMetrics[];
  hyphenWidth?: number;
}

interface MeasureCacheState {
  entries: Map<string, ShapedGroup>;
  glyphs: number;
  keyLength: number;
  version: number;
}

const MAX_CACHE_GROUPS = 128;
const MAX_CACHE_GLYPHS = 32_768;
const MAX_CACHE_KEY_LENGTH = 262_144;
const measureCaches = new WeakMap<TypographyMeasureCache, MeasureCacheState>();

/** 每个编辑器持有独立的有界 LRU；只缓存文字度量，不缓存 DOM binding 或原子框。 */
export function createTypographyMeasureCache(): TypographyMeasureCache {
  const state: MeasureCacheState = {
    entries: new Map(),
    glyphs: 0,
    keyLength: 0,
    version: 0,
  };
  const cache: TypographyMeasureCache = {
    clear() {
      state.entries.clear();
      state.glyphs = 0;
      state.keyLength = 0;
      state.version++;
    },
  };
  measureCaches.set(cache, state);
  return cache;
}

function cachedGroup(state: MeasureCacheState | undefined, key: string): ShapedGroup | undefined {
  const group = state?.entries.get(key);
  if (group) {
    state!.entries.delete(key);
    state!.entries.set(key, group);
  }
  return group;
}

function cacheGroup(state: MeasureCacheState, key: string, group: ShapedGroup) {
  if (group.metrics.length > MAX_CACHE_GLYPHS || key.length > MAX_CACHE_KEY_LENGTH) return;
  const previous = state.entries.get(key);
  if (previous) {
    state.glyphs -= previous.metrics.length;
    state.keyLength -= key.length;
    state.entries.delete(key);
  }
  state.entries.set(key, group);
  state.glyphs += group.metrics.length;
  state.keyLength += key.length;
  while (
    state.entries.size > MAX_CACHE_GROUPS ||
    state.glyphs > MAX_CACHE_GLYPHS ||
    state.keyLength > MAX_CACHE_KEY_LENGTH
  ) {
    const oldest = state.entries.keys().next().value!;
    state.glyphs -= state.entries.get(oldest)!.metrics.length;
    state.keyLength -= oldest.length;
    state.entries.delete(oldest);
  }
}

const SKIP = '.ProseMirror-widget,.ProseMirror-trailingBreak,[data-kp-owned],script,style';
const ATOMIC =
  '.math-inline,.katex,.footnote-ref,.image-node,img,code,.pm-inline-source-code,input[type="checkbox"]';
const KP_TEXT_STYLES = new Set(['margin-left', 'margin-right', 'line-height', 'padding-right']);

function isPlainKpTextWrapper(node: HTMLElement): boolean {
  return (
    node.tagName === 'SPAN' &&
    node.classList.length === 1 &&
    node.classList.contains('kp-unit') &&
    Array.from(node.attributes).every(({ name }) => name === 'class' || name === 'style') &&
    node.style.getPropertyValue('margin-left') !== '' &&
    node.style.getPropertyValue('margin-right') !== '' &&
    node.style.getPropertyValue('line-height') !== '' &&
    Array.from(node.style).every((property) => KP_TEXT_STYLES.has(property)) &&
    node.childNodes.length > 0 &&
    Array.from(node.childNodes).every((child) => child.nodeType === Node.TEXT_NODE)
  );
}

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
    fontKerning: style.fontKerning,
    fontVariationSettings: style.fontVariationSettings,
    fontOpticalSizing: style.getPropertyValue('font-optical-sizing'),
    fontVariantCaps: style.fontVariantCaps,
    fontVariantNumeric: style.fontVariantNumeric,
    fontVariantEastAsian: style.fontVariantEastAsian,
    fontVariantPosition: style.getPropertyValue('font-variant-position'),
    fontVariantAlternates: style.getPropertyValue('font-variant-alternates'),
    fontSizeAdjust: style.getPropertyValue('font-size-adjust'),
    fontLanguageOverride: style.getPropertyValue('font-language-override'),
    fontSynthesis: style.getPropertyValue('font-synthesis'),
    textRendering: style.textRendering,
    direction: style.direction,
  };
  // Text measurement below always disables ligatures. Key the cache by that
  // actual shaping configuration, so installing KP's identical CSS does not
  // turn the first zoom into a cold measurement of every glyph.
  const shaping = { ...font, fontFeatureSettings: '"liga" 0', fontVariantLigatures: 'none' };
  return { ...font, key: JSON.stringify(Object.values(shaping).slice(1)) };
}

function applyFont(sandbox: HTMLElement, font: ReturnType<typeof snapshotFont>) {
  sandbox.style.font = font.font;
  sandbox.style.fontFamily = font.fontFamily;
  sandbox.style.fontSize = font.fontSize;
  sandbox.style.fontWeight = font.fontWeight;
  sandbox.style.fontStyle = font.fontStyle;
  sandbox.style.fontStretch = font.fontStretch;
  sandbox.style.letterSpacing = font.letterSpacing;
  sandbox.style.wordSpacing = font.wordSpacing;
  sandbox.style.fontFeatureSettings = font.fontFeatureSettings;
  sandbox.style.fontVariantLigatures = font.fontVariantLigatures;
  sandbox.style.fontKerning = font.fontKerning;
  sandbox.style.fontVariationSettings = font.fontVariationSettings;
  sandbox.style.setProperty('font-optical-sizing', font.fontOpticalSizing);
  sandbox.style.fontVariantCaps = font.fontVariantCaps;
  sandbox.style.fontVariantNumeric = font.fontVariantNumeric;
  sandbox.style.fontVariantEastAsian = font.fontVariantEastAsian;
  sandbox.style.setProperty('font-variant-position', font.fontVariantPosition);
  sandbox.style.setProperty('font-variant-alternates', font.fontVariantAlternates);
  sandbox.style.setProperty('font-size-adjust', font.fontSizeAdjust);
  sandbox.style.setProperty('font-language-override', font.fontLanguageOverride);
  sandbox.style.setProperty('font-synthesis', font.fontSynthesis);
  sandbox.style.textRendering = font.textRendering;
  sandbox.style.direction = font.direction;
}

function createSandbox() {
  const sandbox = document.createElement('span');
  sandbox.dataset.kpOwned = 'measurement';
  sandbox.style.cssText =
    'position:fixed;left:-100000px;top:0;visibility:hidden;white-space:pre;pointer-events:none;';
  return sandbox;
}

/** DOM-free search uses real browser measurements collected here in bounded batches. */
export async function measureParagraph(
  element: HTMLElement,
  options: TypographyOptions,
  shouldCancel?: () => boolean,
  context?: ParagraphMeasurementContext,
): Promise<MeasuredParagraph | null> {
  const batches = measureParagraphBatches(element, options, 1_500, shouldCancel, context);
  try {
    let step = batches.next();
    while (!step.done) {
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      if (shouldCancel?.()) return null;
      step = batches.next();
    }
    return step.value;
  } finally {
    batches.return(null);
  }
}

/** Local editing layout can finish in the current frame within a bounded budget. */
export function measureParagraphSync(
  element: HTMLElement,
  options: TypographyOptions,
  budgetMs = 24,
  context?: ParagraphMeasurementContext,
): MeasuredParagraph | null {
  const budget = Number.isFinite(budgetMs) ? budgetMs : 24;
  if (budget <= 0) return null;
  const batches = measureParagraphBatches(element, options, budget, undefined, context);
  try {
    let step = batches.next();
    while (!step.done) step = batches.next();
    return step.value;
  } finally {
    batches.return(null);
  }
}

export function prepareParagraphMeasurement(
  element: HTMLElement,
  shouldCancel?: () => boolean,
): PreparedParagraphMeasurement | null {
  return prepareParagraphSnapshot(element, shouldCancel, performance.now() + 1_500);
}

function prepareParagraphSnapshot(
  element: HTMLElement,
  shouldCancel: (() => boolean) | undefined,
  deadline: number,
): PreparedParagraphMeasurement | null {
  if (shouldCancel?.()) return null;
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
  const lineHeightCss = style.lineHeight;
  const lineHeight = parseFloat(lineHeightCss) || rootFont * 1.75;
  let firstLineIndent = parseFloat(style.textIndent) || 0;
  const checkbox = element.querySelector<HTMLElement>('.task-checkbox-widget');
  if (checkbox) {
    const checkboxStyle = getComputedStyle(checkbox);
    const scale = element.getBoundingClientRect().width / (element.offsetWidth || 1) || 1;
    firstLineIndent +=
      checkbox.getBoundingClientRect().width / scale +
      (parseFloat(checkboxStyle.marginLeft) || 0) +
      (parseFloat(checkboxStyle.marginRight) || 0);
  }
  const computedStyles = new Map<HTMLElement, CSSStyleDeclaration>([[element, style]]);
  const canReuseKpFont = element.dataset.kpLayout === 'ready' && !!element.closest('.ProseMirror');
  const semanticParents = new Map<HTMLElement, HTMLElement>();
  const semanticParent = (node: HTMLElement) => {
    if (!canReuseKpFont) return node;
    let parent = semanticParents.get(node);
    if (!parent) {
      // Generated text units only add spacing/line height. Marks and hidden
      // source markers remain semantic parents and retain their own style reads.
      parent = isPlainKpTextWrapper(node) ? (node.parentElement ?? node) : node;
      semanticParents.set(node, parent);
    }
    return parent;
  };
  const readStyle = (node: HTMLElement) => {
    const parent = semanticParent(node);
    let computed = computedStyles.get(parent);
    if (!computed) {
      computed = getComputedStyle(parent);
      computedStyles.set(parent, computed);
    }
    return computed;
  };
  const runs: { node: Node; text: string; atomic: boolean; soft?: boolean; hard?: boolean }[] = [];
  let timedOut = false;
  const walk = (node: Node) => {
    if (shouldCancel?.() || performance.now() > deadline) {
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
    hyphenatable: boolean;
    sourceSize: number;
    code: boolean;
  }[] = [];
  for (const run of runs) {
    if (shouldCancel?.() || performance.now() > deadline) return null;
    const parent =
      run.node.nodeType === Node.TEXT_NODE ? run.node.parentElement! : (run.node as HTMLElement);
    const computed = readStyle(parent);
    const fontParent = semanticParent(parent);
    let font = fontSnapshots.get(fontParent);
    if (!font) {
      font = snapshotFont(computed);
      fontSnapshots.set(fontParent, font);
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
    const language = parent
      .closest('p[lang],li[lang],td[lang],th[lang],span[lang],a[lang]')
      ?.getAttribute('lang');
    const code =
      run.atomic && !run.soft && !run.hard && parent.matches('code,.pm-inline-source-code');
    runFonts.push({
      parent,
      font,
      atomicStyles,
      hyphenatable:
        !parent.closest('a,code,.pm-inline-source-code') &&
        (!language || /^en(?:-|$)/i.test(language)),
      sourceSize: code ? (parent.textContent?.length ?? 1) : 1,
      code,
    });
  }
  if (shouldCancel?.() || performance.now() > deadline) return null;
  const preparedRuns: PreparedRun[] = [];
  let sandbox: HTMLElement | undefined;
  try {
    for (let r = 0; r < runs.length; r++) {
      if (shouldCancel?.() || performance.now() > deadline) return null;
      const run = runs[r];
      const captured = runFonts[r];
      const preparedRun: PreparedRun = {
        ...run,
        font: captured.font,
        hyphenatable: captured.hyphenatable,
      };
      if (run.atomic && !run.soft && !run.hard) {
        sandbox ??= createSandbox();
        if (!sandbox.isConnected) document.body.append(sandbox);
        applyFont(sandbox, captured.font);
        sandbox.style.lineHeight = lineHeightCss;
        sandbox.style.width = width + 'px';
        const copy = captured.parent.cloneNode(true) as HTMLElement;
        const original = copy.getAttribute('data-kp-unit-original-style');
        if (original !== null) copy.style.cssText = original;
        // The projection is outside .ProseMirror; remove hidden source markers
        // and previous KP wrappers before capturing the target object's geometry.
        copy
          .querySelectorAll('.pm-inline-source-marker.is-hidden')
          .forEach((node) => node.remove());
        copy.classList.remove('kp-unit');
        copy.querySelectorAll('.kp-unit').forEach((node) => node.replaceWith(...node.childNodes));
        for (const [property, value] of captured.atomicStyles)
          copy.style.setProperty(property, value);
        const marker = document.createElement('span');
        marker.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline;';
        sandbox.replaceChildren(copy, marker);
        const rect = copy.getBoundingClientRect();
        const baseline = marker.getBoundingClientRect().top;
        const copyStyle = getComputedStyle(copy);
        preparedRun.object = {
          projection: copy,
          width:
            rect.width +
            (parseFloat(copyStyle.marginLeft) || 0) +
            (parseFloat(copyStyle.marginRight) || 0),
          ascent: Math.max(0, baseline - rect.top),
          descent: Math.max(0, rect.bottom - baseline),
          sourceSize: captured.sourceSize,
          code: captured.code,
        };
        copy.remove();
      }
      preparedRuns.push(preparedRun);
    }
  } finally {
    sandbox?.remove();
  }
  if (shouldCancel?.() || performance.now() > deadline) return null;
  const text = runs.map((run) => run.text).join('');
  return {
    width,
    lineHeight,
    rootFont,
    firstLineIndent,
    runs: preparedRuns,
    text,
    hasEnglishWords: /[A-Za-z]{8,}/.test(text),
  };
}

function* measureParagraphBatches(
  element: HTMLElement,
  options: TypographyOptions,
  budgetMs: number,
  shouldCancel?: () => boolean,
  context?: ParagraphMeasurementContext,
): Generator<void, MeasuredParagraph | null, void> {
  if (shouldCancel?.()) return null;
  let elapsed = performance.now();
  const deadline = elapsed + budgetMs;
  const prepared =
    context && 'prepared' in context
      ? context.prepared
      : prepareParagraphSnapshot(element, shouldCancel, deadline);
  if (!prepared || !segmenter || shouldCancel?.() || performance.now() > deadline) return null;
  const { runs, width, lineHeight, rootFont } = prepared;
  const cacheState = context?.cache ? measureCaches.get(context.cache) : undefined;
  const cacheVersion = cacheState?.version;
  const units: LayoutItem[] = [];
  const bindings: DomBinding[] = [];
  const forced = new Set<number>();
  const sandbox = createSandbox();
  let from = 0;
  try {
    if (!cacheState) document.body.append(sandbox);
    const canvas = document.createElement('canvas').getContext('2d');
    for (let r = 0; r < runs.length; r++) {
      const run = runs[r];
      if (shouldCancel?.() || performance.now() > deadline) return null;
      const font = run.font;
      const size = parseFloat(font.fontSize) || rootFont;
      if (run.atomic) {
        let text = run.text;
        if (run.soft) text = softBreakText(runs[r - 1]?.text ?? '', runs[r + 1]?.text ?? '');
        const breakNode = run.soft || run.hard;
        if (canvas)
          canvas.font = [font.fontStyle, font.fontWeight, font.fontSize, font.fontFamily].join(' ');
        const natural = breakNode
          ? text === ' '
            ? (canvas?.measureText(' ').width ?? size * 0.25)
            : 0
          : (run.object?.width ?? 0);
        // Atomic means an unbroken layout box; its original source still owns
        // every caret position, including code delimiters hidden in the projection.
        const sourceSize = run.object?.sourceSize ?? 1;
        const to = from + sourceSize;
        bindings.push({
          node: run.node,
          offset: 0,
          endOffset: 1,
          from,
          to,
          atomic: true,
          width: natural,
          sourceSize,
        });
        units.push({
          kind: breakNode ? 'glue' : 'box',
          from,
          to,
          text,
          width: natural,
          fontSize: size,
          ascent: breakNode ? 0 : Math.max(size * 0.8, run.object?.ascent ?? 0),
          descent: breakNode ? 0 : Math.max(size * 0.2, run.object?.descent ?? 0),
          stretch: run.soft && text ? natural * 0.5 : 0,
          shrink: run.soft ? natural / 3 : 0,
          comfortStretch: run.soft && text ? natural * 0.25 : 0,
          comfortShrink: run.soft ? natural * 0.25 : 0,
          code: run.object?.code ?? false,
          discardable: !!breakNode,
        });
        from = to;
        if (run.hard) forced.add(from);
        continue;
      }
      const firstRun = r;
      while (r + 1 < runs.length && !runs[r + 1].atomic && runs[r + 1].font.key === font.key) r++;
      const group = runs.slice(firstRun, r + 1);
      const text = group.map((part) => part.text).join('');
      const segments: {
        run: PreparedRun;
        text: string;
        offset: number;
        end: number;
        groupEnd: number;
      }[] = [];
      let contextLength = 0;
      for (const sourceRun of group) {
        for (const part of segmenter.segment(sourceRun.text)) {
          if (shouldCancel?.() || performance.now() > deadline) return null;
          const end = part.index + part.segment.length;
          segments.push({
            run: sourceRun,
            text: part.segment,
            offset: part.index,
            end,
            groupEnd: contextLength + end,
          });
          const now = performance.now();
          if (now - elapsed > 8) {
            yield;
            elapsed = performance.now();
            if (shouldCancel?.() || elapsed > deadline) return null;
          }
        }
        contextLength += sourceRun.text.length;
      }
      const needsHyphen =
        text.includes('\u00ad') || (prepared.hasEnglishWords && /[A-Za-z]/u.test(text));
      // Run boundaries may split a grapheme even when the joined text is equal.
      // Including the segmentation keeps cache hits independent of old DOM nodes.
      const cacheKey = cacheState
        ? JSON.stringify([font.key, text, needsHyphen, segments.map((part) => part.groupEnd)])
        : '';
      const cached =
        cacheState?.version === cacheVersion ? cachedGroup(cacheState, cacheKey) : undefined;
      const metrics: GraphemeMetrics[] = [];
      let hyphenWidth = cached?.hyphenWidth;
      let range: Range | undefined;
      if (!cached) {
        // Cache hits only bind current text nodes; inserting an unused sandbox
        // would invalidate page layout before the next paragraph's geometry read.
        if (!sandbox.isConnected) document.body.append(sandbox);
        applyFont(sandbox, font);
        // Match ProseMirror's editable shaping in every output surface.
        sandbox.style.fontFeatureSettings = '"liga" 0';
        sandbox.style.fontVariantLigatures = 'none';
        sandbox.style.letterSpacing = font.letterSpacing;
        sandbox.style.wordSpacing = font.wordSpacing;
        sandbox.style.setProperty('text-autospace', 'no-autospace');
        sandbox.style.setProperty('text-spacing-trim', 'space-all');
        if (canvas)
          canvas.font = [font.fontStyle, font.fontWeight, font.fontSize, font.fontFamily].join(' ');
        if (needsHyphen) {
          sandbox.textContent = '-';
          const hyphenRange = document.createRange();
          hyphenRange.selectNodeContents(sandbox);
          hyphenWidth = hyphenRange.getBoundingClientRect().width;
        }
        sandbox.textContent = text;
        range = document.createRange();
        range.setStart(sandbox.firstChild!, 0);
      }
      let previousWidth = 0;
      for (let index = 0; index < segments.length; index++) {
        if (shouldCancel?.() || performance.now() > deadline) return null;
        const part = segments[index];
        let metric = cached?.metrics[index];
        if (!metric) {
          range!.setEnd(sandbox.firstChild!, part.groupEnd);
          const prefixWidth = range!.getBoundingClientRect().width;
          const natural = Math.max(0, prefixWidth - previousWidth);
          const measured = canvas?.measureText(part.text);
          metric = {
            prefixWidth,
            ascent: measured?.actualBoundingBoxAscent ?? 0,
            descent: measured?.actualBoundingBoxDescent ?? 0,
            left: measured?.actualBoundingBoxLeft ?? 0,
            right: measured?.actualBoundingBoxRight ?? natural,
          };
          metrics.push(metric);
        }
        const natural = Math.max(0, metric.prefixWidth - previousWidth);
        previousWidth = metric.prefixWidth;
        const space = /^[ \t]+$/.test(part.text);
        const to = from + part.text.length;
        bindings.push({
          node: part.run.node,
          offset: part.offset,
          endOffset: part.end,
          from,
          to,
          atomic: false,
          width: natural,
        });
        units.push({
          kind: space ? 'glue' : part.text === '\u00ad' ? 'penalty' : 'box',
          from,
          to,
          text: part.text,
          width: part.text === '\u00ad' ? 0 : natural,
          fontSize: size,
          ascent: Math.max(size * 0.8, metric.ascent),
          descent: Math.max(size * 0.2, metric.descent),
          leadingSpace: Math.max(0, -metric.left),
          trailingSpace: Math.max(0, natural - metric.right),
          breakWidth: /^[A-Za-z\u00ad]$/u.test(part.text) ? hyphenWidth : undefined,
          hyphenatable: part.run.hyphenatable,
          stretch: space ? natural * 0.5 : 0,
          shrink: space ? natural / 3 : 0,
          comfortStretch: space ? natural * 0.25 : 0,
          comfortShrink: space ? natural * 0.25 : 0,
          weight: 1,
          discardable: space || part.text === '\u00ad',
          sourceWhitespace: space,
        });
        from = to;
        const now = performance.now();
        if (shouldCancel?.() || now > deadline) return null;
        if (now - elapsed > 8) {
          yield;
          elapsed = performance.now();
          if (shouldCancel?.() || elapsed > deadline) return null;
        }
      }
      if (!cached && cacheState && cacheState.version === cacheVersion) {
        // Only complete groups enter the cache; cancellation cannot retain a
        // partially measured prefix or old DOM binding.
        cacheGroup(cacheState, cacheKey, { metrics, hyphenWidth });
      }
    }
  } finally {
    sandbox.remove();
  }
  if (!units.length || shouldCancel?.() || performance.now() > deadline) return null;
  const input = createParagraphInput(units, width, lineHeight, options, forced, rootFont);
  input.firstLineIndent = prepared.firstLineIndent;
  if (shouldCancel?.() || performance.now() > deadline) return null;
  return { input, bindings, text: prepared.text };
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
