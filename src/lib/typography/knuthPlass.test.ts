import { describe, expect, it } from 'vitest';
import { evaluateLine, layoutParagraph } from './knuthPlass';
import { createParagraphInput, softBreakText } from './rules';
import { createPositionMap, sourceToVisual, visualToSource } from './positions';
import { DEFAULT_TYPOGRAPHY, type LayoutItem, type ParagraphInput, type LineLayout } from './types';

function box(width = 10, from = 0): LayoutItem {
  return {
    kind: 'box',
    from,
    to: from + 1,
    text: '字',
    width,
    fontSize: 10,
    ascent: 8,
    descent: 2,
  };
}
function input(widths: number[], width: number): ParagraphInput {
  return {
    items: widths.map((w, i) => ({ ...box(w, i), stretch: 7, shrink: 2 })),
    breaks: widths.map((_, i) => ({ at: i + 1, penalty: 0, required: i === widths.length - 1 })),
    width,
    minLineHeight: 14,
    hanging: false,
    timeBudgetMs: 10_000,
  };
}

/** Enumerate all partitions independently of the DP state-merging algorithm. */
function exhaustive(problem: ParagraphInput) {
  let best = Infinity;
  function visit(start: number, lines: LineLayout[], cost: number, flagged: boolean) {
    for (const point of problem.breaks) {
      if (point.at <= start) continue;
      if (point.penalty >= 10_000 && !point.required) continue;
      const line = evaluateLine(problem, start, point, lines.length);
      if (line) {
        const previous = lines.at(-1);
        let next = cost + (10 + line.badness) ** 2;
        if (!point.required) next += Math.sign(point.penalty) * point.penalty ** 2;
        if (previous && Math.abs(previous.fitness - line.fitness) > 1) next += 3000;
        if (flagged && point.flagged) next += 3000;
        if (line.last && flagged) next += 1500;
        if (point.at === problem.items.length && previous && line.naturalWidth < line.width * 0.2)
          next += 1000 * (1 - line.naturalWidth / (line.width * 0.2));
        if (point.at === problem.items.length) best = Math.min(best, next);
        else visit(point.at, [...lines, line], next, !!point.flagged);
      }
      if (point.required) break;
    }
  }
  visit(0, [], 0, false);
  return best;
}

describe('Knuth–Plass behaviour', () => {
  it('maps both sides of a visual break without splitting a grapheme', () => {
    const problem = input([10, 10, 10, 10], 20);
    const map = createPositionMap(problem, layoutParagraph(problem));
    expect(sourceToVisual(map, 2, 'before')).toMatchObject({ line: 0, x: 20 });
    expect(sourceToVisual(map, 2, 'after')).toMatchObject({ line: 1, x: 0 });
    expect(visualToSource(map, 1, 0)).toEqual({ offset: 2, affinity: 'after' });
  });
  it('matches exhaustive partitions with fitness, forced breaks, penalties and line widths', () => {
    for (let seed = 0; seed < 100; seed++) {
      const problem = input(
        Array.from({ length: 7 }, (_, i) => 5 + ((seed * 11 + i * 3) % 15)),
        26 + (seed % 10),
      );
      problem.lineWidths = [22 + (seed % 8), 35];
      problem.firstLineIndent = seed % 3;
      problem.breaks[2].required = seed % 3 === 0;
      problem.breaks[1].penalty = seed % 5 === 0 ? 10000 : -10;
      problem.breaks[3].flagged = true;
      problem.breaks[3].width = 3;
      problem.breaks[4].flagged = true;
      const expected = exhaustive(problem);
      const actual = layoutParagraph(problem);
      if (expected === Infinity) expect(actual.status).toBe('fallback');
      else {
        expect(actual.status).toBe('ready');
        if (actual.status === 'ready') expect(actual.demerits).toBeCloseTo(expected, 7);
      }
    }
  });
  it('honours mandatory and prohibited breakpoints', () => {
    const p = input([10, 10, 10, 10], 40);
    p.breaks[0].penalty = 10000;
    p.breaks[1].required = true;
    const result = layoutParagraph(p);
    expect(result.status).toBe('ready');
    if (result.status === 'ready') expect(result.lines.map((l) => l.end)).toEqual([2, 4]);
  });
  it('handles zero flexibility, impossible objects and explicit budget failure', () => {
    const p = input([20], 10);
    p.items[0].shrink = 0;
    expect(layoutParagraph(p)).toMatchObject({ status: 'fallback', reason: 'no-solution' });
    p.width = 25;
    p.items[0].stretch = 0;
    expect(layoutParagraph(p)).toMatchObject({ status: 'ready' });
    p.maxCandidates = 0;
    expect(layoutParagraph(p)).toMatchObject({ status: 'fallback', reason: 'budget-exceeded' });
    p.width = NaN;
    expect(layoutParagraph(p)).toMatchObject({ status: 'fallback', reason: 'invalid-metrics' });
  });
  it('includes compression, hanging, break width and tall content before fitting', () => {
    const p = input([10, 10], 14);
    p.items.forEach((i) => {
      i.shrink = 0;
    });
    p.items[1].trimEnd = 3;
    p.items[1].hang = 3;
    expect(evaluateLine(p, 0, p.breaks[1], 0)).toBeNull();
    p.hanging = true;
    p.items[1].ascent = 30;
    p.items[1].descent = 8;
    expect(evaluateLine(p, 0, p.breaks[1], 0)).toMatchObject({
      naturalWidth: 14,
      height: 38,
      hang: 3,
    });
    p.breaks[1].width = 2;
    expect(evaluateLine(p, 0, p.breaks[1], 0)).toBeNull();
  });
  it('allocates whitespace within class capacities and never stretches the last line', () => {
    const p = input([10, 10, 10], 25);
    const line = evaluateLine(p, 0, p.breaks[1], 0)!;
    expect(line.adjustments.reduce((a, b) => a + b, 0)).toBeCloseTo(5);
    expect(evaluateLine(p, 2, p.breaks[2], 1)?.adjustments).toEqual([0]);
  });
});

describe('Chinese/English projection', () => {
  const project = (text: string) => {
    let offset = 0;
    const units = [...new Intl.Segmenter('zh', { granularity: 'grapheme' }).segment(text)].map(
      ({ segment }) => {
        const unit = { ...box(10, offset), text: segment, to: offset + segment.length };
        offset += segment.length;
        return unit;
      },
    );
    return createParagraphInput(units, 100, 18, DEFAULT_TYPOGRAPHY);
  };
  it('preserves words, NBSP, combining marks, emoji and punctuation pairs', () => {
    for (const text of ['hello', 'a\u00a0b', 'a\u0301', '👩‍👩‍👧‍👦', '——', '……']) {
      const p = project(text);
      expect(p.breaks).toHaveLength(1);
    }
    const p = project('甲（乙），丙');
    const ends = p.breaks.map((b) => p.items[b.at - 1].to);
    expect(ends).not.toContain(2);
    expect(ends).not.toContain(3);
    expect(ends).not.toContain(4);
  });
  it('adds visual glue only at mixed boundaries without existing spaces', () => {
    expect(project('中A').items.filter((i) => i.from === i.to)).toHaveLength(1);
    expect(project('中 A').items.filter((i) => i.from === i.to)).toHaveLength(0);
    expect(softBreakText('o', 'w')).toBe(' ');
    expect(softBreakText('文', '字')).toBe('');
    expect(softBreakText(' ', 'A')).toBe('');
    expect(softBreakText('𠀀', 'word')).toBe('');
    const soft = createParagraphInput(
      [
        { ...box(10, 0), text: '中' },
        { ...box(0, 1), text: '', discardable: true },
        { ...box(10, 2), text: 'A' },
      ],
      100,
      18,
      DEFAULT_TYPOGRAPHY,
    );
    expect(soft.items.filter((item) => item.from === item.to)).toHaveLength(1);
    expect(project('ab\u00adcd').breaks.some((b) => b.flagged && b.width! > 0)).toBe(true);
    const hyphen = createParagraphInput(
      [
        { ...box(), text: '\u00ad', breakWidth: 7.125 },
        { ...box(10, 1), text: 'a' },
      ],
      100,
      18,
      DEFAULT_TYPOGRAPHY,
    );
    expect(hyphen.breaks[0].width).toBe(7.125);
    expect(project('word\u00ad').breaks.at(-1)?.width).toBe(0);
  });
});
