import type { MeasuredParagraph } from './measure';
import type { ParagraphLayout } from './types';

export interface UnitStyle {
  from: number;
  to: number;
  left: number;
  right: number;
  height: number;
}
export interface VisualBreak {
  at: number;
  hyphen: boolean;
}

/** A shared rendering recipe for ProseMirror decorations and read-only DOM. */
export function createRenderPlan(measured: MeasuredParagraph, result: ParagraphLayout) {
  const styles = new Map<number, UnitStyle>();
  const breaks: VisualBreak[] = [];
  if (result.status !== 'ready') return { styles: [], breaks };
  const bindings = new Map(measured.bindings.map((binding) => [binding.from, binding]));
  const { items } = measured.input;
  for (const line of result.lines) {
    let previous: UnitStyle | undefined;
    for (let i = line.start; i < line.end; i++) {
      const item = items[i];
      if (item.from === item.to) {
        if (previous) previous.right += item.width + line.adjustments[i - line.start];
        continue;
      }
      const binding = bindings.get(item.from);
      const value: UnitStyle = {
        from: item.from,
        to: item.to,
        left: 0,
        right: item.width - (binding?.width ?? item.width) + line.adjustments[i - line.start],
        height: line.height,
      };
      if (i === line.visibleStart) value.left -= line.trimStart;
      if (i === line.visibleEnd - 1) value.right -= line.trimEnd + line.hang;
      styles.set(item.from, value);
      previous = value;
    }
    if (line.end < items.length && items[line.end - 1]?.text !== '\n') {
      breaks.push({ at: items[line.end - 1].to, hyphen: line.hyphenWidth > 0 });
    }
  }
  return { styles: Array.from(styles.values()), breaks };
}

export function unitStyleCss(style: UnitStyle): string {
  return `margin-left:${style.left.toFixed(3)}px;margin-right:${style.right.toFixed(3)}px;line-height:${style.height.toFixed(3)}px;`;
}

export const PARAGRAPH_CSS =
  'white-space:pre;overflow-wrap:normal;word-break:normal;text-align:left;text-align-last:left;text-autospace:no-autospace;text-spacing-trim:space-all;hyphens:none;font-variant-ligatures:none;font-feature-settings:"liga" 0;';
