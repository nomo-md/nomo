import { describe, expect, it } from 'vitest';
import { MarkdownHeightIndex } from './viewport';

describe('Markdown 视口高度与挂载', () => {
  it.each(['append', 'rebuild'] as const)('%s 后局部高度变化及追加定位与独立线性基准一致', method => {
    const index = new MarkdownHeightIndex();
    const values: number[] = [];
    for (let i = 0; i < 1_000; i++) {
      const height = 20 + (i % 17);
      values.push(height);
      if (method === 'append') index.append(height);
    }
    if (method === 'rebuild') index.rebuild(values.length, i => values[i]);
    values.push(31); index.append(31);
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
  it('批量高度无效时保留原索引，空批次可以清空', () => {
    const index = new MarkdownHeightIndex(); index.append(30);
    expect(() => index.rebuild(3, i => i === 2 ? NaN : 20)).toThrow(RangeError);
    expect(index.totalHeight).toBe(30); expect(index.length).toBe(1);
    index.rebuild(0, () => 10);
    expect(index.totalHeight).toBe(0); expect(index.indexAt(10)).toBe(-1);
  });
});
