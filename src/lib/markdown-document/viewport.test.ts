import { describe, expect, it } from 'vitest';
import { MarkdownHeightIndex } from './viewport';

describe('Markdown 视口高度与挂载', () => {
  it('局部高度变化后定位与独立线性基准一致', () => {
    const index = new MarkdownHeightIndex();
    const values: number[] = [];
    for (let i = 0; i < 1_000; i++) {
      const height = 20 + (i % 17);
      values.push(height);
      index.append(height);
    }
    for (let i = 0; i < values.length; i += 7) {
      values[i] += 55;
      index.update(i, values[i]);
    }
    expect(index.totalHeight).toBe(values.reduce((sum, value) => sum + value, 0));
    for (let i = 0; i < values.length; i += 13) {
      const height = values.slice(0, i).reduce((sum, value) => sum + value, 0);
      expect(index.top(i)).toBe(height);
      expect(index.indexAt(height + 0.5)).toBe(i);
    }
  });
});
