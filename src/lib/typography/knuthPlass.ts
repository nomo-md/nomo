import type { BreakPoint, LineLayout, ParagraphInput, ParagraphLayout } from './types';

const EPSILON = 0.02;
const FORBIDDEN = 10_000;

/** Allocates space by class preference, with each unit's capacity as a hard bound. */
function allocate(input: ParagraphInput, start: number, end: number, delta: number): number[] {
  const result = Array(end - start).fill(0) as number[];
  const growing = delta >= 0;
  let remaining = Math.abs(delta);
  const active = input.items
    .slice(start, end)
    .map((item, i) => ({
      i,
      capacity: growing ? (item.stretch ?? 0) : (item.shrink ?? 0),
      weight: item.weight ?? 1,
    }))
    .filter((item) => item.capacity > 0);
  while (remaining > EPSILON && active.length) {
    const total = active.reduce((n, item) => n + item.capacity * item.weight, 0);
    let consumed = 0;
    for (let i = active.length - 1; i >= 0; i--) {
      const item = active[i];
      const amount = Math.min(item.capacity, (remaining * item.capacity * item.weight) / total);
      result[item.i] += growing ? amount : -amount;
      item.capacity -= amount;
      consumed += amount;
      if (item.capacity < EPSILON) active.splice(i, 1);
    }
    if (consumed < EPSILON) break;
    remaining -= consumed;
  }
  return result;
}

/** Edge compression and protrusion are part of fitting, not post-layout transforms. */
export function evaluateLine(
  input: ParagraphInput,
  start: number,
  point: BreakPoint,
  lineIndex: number,
): LineLayout | null {
  const { items } = input;
  const end = point.at;
  let first = start;
  let last = end;
  while (first < last && items[first].discardable) first++;
  while (last > first && items[last - 1].discardable) last--;
  const width =
    (input.lineWidths?.[lineIndex] ?? input.width) -
    (lineIndex === 0 ? (input.firstLineIndent ?? 0) : 0);
  const final = end === items.length || !!point.required;
  const trimStart = first < last ? (items[first]?.trimStart ?? 0) : 0;
  const trimEnd = last > first ? (items[last - 1].trimEnd ?? 0) : 0;
  const hang =
    input.hanging && last > first
      ? Math.min(items[last - 1].hang ?? 0, Math.max(0, items[last - 1].width - trimEnd))
      : 0;
  let naturalWidth = (point.width ?? 0) - trimStart - trimEnd - hang;
  let stretch = 0;
  let shrink = 0;
  let ascent = input.minLineHeight * 0.75;
  let descent = input.minLineHeight * 0.25;
  for (let i = first; i < last; i++) {
    const item = items[i];
    naturalWidth += item.width;
    stretch += item.stretch ?? 0;
    shrink += item.shrink ?? 0;
    ascent = Math.max(ascent, item.ascent);
    descent = Math.max(descent, item.descent);
  }
  const delta = width - naturalWidth;
  const ragged = input.alignment === 'ragged';
  let ratio = 0;
  if (!((final || ragged) && delta >= -EPSILON)) {
    const capacity = delta >= 0 ? stretch : shrink;
    if (Math.abs(delta) > EPSILON && capacity <= 0) return null;
    ratio = capacity > 0 ? delta / capacity : 0;
    if (ratio < -1 - EPSILON || ratio > 1 + EPSILON) return null;
  }
  if (width <= 0 || naturalWidth < -EPSILON) return null;
  const adjustments = Array(end - start).fill(0) as number[];
  const inner = allocate(input, first, last, (final || ragged) && delta >= 0 ? 0 : delta);
  for (let i = first; i < last; i++) adjustments[i - start] = inner[i - first];
  for (let i = start; i < first; i++) adjustments[i - start] = -items[i].width;
  for (let i = last; i < end; i++) adjustments[i - start] = -items[i].width;
  let badness = 100 * Math.abs(ratio) ** 3;
  for (let i = first; i < last; i++) {
    const adjustment = adjustments[i - start];
    const comfort = adjustment >= 0 ? items[i].comfortStretch : items[i].comfortShrink;
    if (comfort !== undefined && Math.abs(adjustment) > EPSILON) {
      // A few large holes must not hide behind the paragraph's total glue capacity.
      badness = Math.max(badness, 100 * (Math.abs(adjustment) / Math.max(EPSILON, comfort)) ** 3);
    }
  }
  if (!ragged && badness > (input.tolerance ?? 10_000)) return null;
  if (ragged && !final && delta > 0) badness += 100 * (delta / width) ** 2;
  const height = Math.max(input.minLineHeight, ascent + descent);
  return {
    start,
    end,
    visibleStart: first,
    visibleEnd: last,
    width,
    naturalWidth,
    ratio,
    badness: Math.min(10_000, badness),
    fitness: ratio < -0.5 ? 0 : ratio <= 0.5 ? 1 : ratio <= 0.8 ? 2 : 3,
    adjustments,
    trimStart,
    trimEnd,
    hang,
    hyphenWidth: point.width ?? 0,
    ascent,
    descent,
    height,
    baseline: ascent,
    last: final,
  };
}

interface State {
  at: number;
  count: number;
  fitness: number;
  flagged: boolean;
  cost: number;
  previous: State | null;
  line: LineLayout | null;
}

/** Pure paragraph optimisation. All paths stop at mandatory breaks. */
export function layoutParagraph(input: ParagraphInput): ParagraphLayout {
  if (input.alignment === 'auto') {
    const now = () => (typeof performance === 'undefined' ? Date.now() : performance.now());
    const started = now();
    const justified = layoutParagraph({ ...input, alignment: 'justify' });
    if (justified.status === 'ready' || justified.reason !== 'no-solution') return justified;
    // Both passes share the original search budget; ragged is a complete KP result.
    const remainingTime = (input.timeBudgetMs ?? 100) - (now() - started);
    const remainingCandidates = (input.maxCandidates ?? 100_000) - justified.candidates;
    if (remainingTime <= 0 || remainingCandidates <= 0)
      return { status: 'fallback', reason: 'budget-exceeded', candidates: justified.candidates };
    const ragged = layoutParagraph({
      ...input,
      alignment: 'ragged',
      timeBudgetMs: remainingTime,
      maxCandidates: remainingCandidates,
    });
    return { ...ragged, candidates: ragged.candidates + justified.candidates };
  }
  if (
    !Number.isFinite(input.width) ||
    input.width <= 0 ||
    !Number.isFinite(input.minLineHeight) ||
    input.minLineHeight <= 0 ||
    input.items.some(
      (item) =>
        !Number.isFinite(item.width) ||
        item.width < 0 ||
        !Number.isFinite(item.ascent + item.descent) ||
        !Number.isFinite((item.stretch ?? 0) + (item.shrink ?? 0)) ||
        (item.stretch ?? 0) < 0 ||
        (item.shrink ?? 0) < 0,
    )
  ) {
    return { status: 'fallback', reason: 'invalid-metrics', candidates: 0 };
  }
  if (!input.items.length) return { status: 'ready', lines: [], demerits: 0, candidates: 0 };
  const points = input.breaks
    .filter((point) => point.at > 0 && point.at <= input.items.length)
    .slice()
    .sort((a, b) => a.at - b.at);
  if (points.at(-1)?.at !== input.items.length)
    points.push({ at: input.items.length, penalty: 0, required: true });
  let states: State[] = [
    { at: 0, count: 0, fitness: 1, flagged: false, cost: 0, previous: null, line: null },
  ];
  const clock = () => (typeof performance === 'undefined' ? Date.now() : performance.now());
  const deadline = clock() + (input.timeBudgetMs ?? 100);
  const budget = input.maxCandidates ?? 100_000;
  // A conservative lower bound permits pruning without dropping a feasible path.
  const minimum = [0];
  let edgeAllowance = 0;
  for (const item of input.items) {
    minimum.push(
      minimum.at(-1)! + (item.discardable ? 0 : Math.max(0, item.width - (item.shrink ?? 0))),
    );
    edgeAllowance = Math.max(
      edgeAllowance,
      (item.trimStart ?? 0) + (item.trimEnd ?? 0) + (item.hang ?? 0),
    );
  }
  const maxWidth =
    Math.max(input.width, ...(input.lineWidths ?? [])) +
    Math.abs(input.firstLineIndent ?? 0) +
    edgeAllowance * 2;
  let candidates = 0;
  for (const point of points) {
    if (point.penalty >= FORBIDDEN && !point.required) continue;
    const best = new Map<string, State>();
    for (const state of states) {
      if (state.at >= point.at) continue;
      if (minimum[point.at] - minimum[state.at] > maxWidth + EPSILON) continue;
      if (++candidates > budget || (candidates % 128 === 0 && clock() > deadline)) {
        return { status: 'fallback', reason: 'budget-exceeded', candidates };
      }
      const line = evaluateLine(input, state.at, point, state.count);
      if (!line) continue;
      let cost = state.cost + (10 + line.badness) ** 2;
      if (!point.required) cost += point.penalty >= 0 ? point.penalty ** 2 : -(point.penalty ** 2);
      if (Math.abs(state.fitness - line.fitness) > 1 && state.count) cost += 3_000;
      if (state.flagged && point.flagged) cost += 3_000;
      if (line.last && state.flagged) cost += 1_500;
      const minimumLastWidth = input.lastLineMinWidth ?? line.width * 0.2;
      if (point.at === input.items.length && state.count && line.naturalWidth < minimumLastWidth) {
        cost += (input.lastLinePenalty ?? 1_000) * (1 - line.naturalWidth / minimumLastWidth);
      }
      const next: State = {
        at: point.at,
        count: state.count + 1,
        fitness: line.fitness,
        flagged: !!point.flagged,
        cost,
        previous: state,
        line,
      };
      const key = `${next.count}:${next.fitness}:${next.flagged}`;
      if (!best.has(key) || best.get(key)!.cost > cost) best.set(key, next);
    }
    if (point.required) states = [];
    states.push(...best.values());
    if (point.required && !states.length)
      return { status: 'fallback', reason: 'no-solution', candidates };
  }
  const finalStates = states.filter((state) => state.at === input.items.length);
  if (!finalStates.length) return { status: 'fallback', reason: 'no-solution', candidates };
  let state = finalStates.reduce((a, b) => (a.cost <= b.cost ? a : b));
  const cost = state.cost;
  const lines: LineLayout[] = [];
  while (state.line) {
    lines.push(state.line);
    state = state.previous!;
  }
  lines.reverse();
  let top = 0;
  for (const line of lines) {
    line.baseline += top;
    top += line.height;
  }
  return {
    status: 'ready',
    lines,
    demerits: cost,
    candidates,
    alignment: input.alignment === 'ragged' ? 'ragged' : 'justify',
  };
}
