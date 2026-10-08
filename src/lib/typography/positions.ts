import type { ParagraphInput, ParagraphLayout } from './types';

export interface VisualPosition {
  from: number;
  to: number;
  line: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** Logical source intervals and CSS positions, including zero-length visual glue. */
export function createPositionMap(
  input: ParagraphInput,
  layout: ParagraphLayout,
): VisualPosition[] {
  if (layout.status !== 'ready') return [];
  const result: VisualPosition[] = [];
  let top = 0;
  layout.lines.forEach((line, lineIndex) => {
    let x = lineIndex === 0 ? (input.firstLineIndent ?? 0) : 0;
    for (let i = line.start; i < line.end; i++) {
      const item = input.items[i];
      if (i === line.visibleStart) x -= line.trimStart;
      const left = x;
      x += item.width + line.adjustments[i - line.start];
      if (i === line.visibleEnd - 1) x -= line.trimEnd + line.hang;
      result.push({
        from: item.from,
        to: item.to,
        line: lineIndex,
        left,
        right: x,
        top,
        bottom: top + line.height,
      });
    }
    top += line.height;
  });
  return result;
}

export function sourceToVisual(
  map: VisualPosition[],
  offset: number,
  affinity: 'before' | 'after' = 'after',
) {
  const candidates = map.filter((p) => p.from <= offset && p.to >= offset && p.from !== p.to);
  const item = affinity === 'before' ? candidates[0] : candidates.at(-1);
  if (!item) return null;
  return {
    line: item.line,
    x: offset === item.to ? item.right : item.left,
    top: item.top,
    bottom: item.bottom,
  };
}

export function visualToSource(
  map: VisualPosition[],
  line: number,
  x: number,
): { offset: number; affinity: 'before' | 'after' } | null {
  const candidates = map.filter((p) => p.line === line && p.from !== p.to);
  if (!candidates.length) return null;
  let best = {
    offset: candidates[0].from,
    affinity: 'after' as 'before' | 'after',
    distance: Infinity,
  };
  for (const item of candidates) {
    for (const [offset, edge, affinity] of [
      [item.from, item.left, 'after'],
      [item.to, item.right, 'before'],
    ] as const) {
      if (Math.abs(edge - x) < best.distance)
        best = { offset, affinity, distance: Math.abs(edge - x) };
    }
  }
  return { offset: best.offset, affinity: best.affinity };
}
