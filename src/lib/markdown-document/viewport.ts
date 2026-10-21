import type { MarkdownStructureBlock } from './structure';

/** 支持追加和局部高度修正的 Fenwick 索引；滚动路径不遍历全部正文或同步测量 DOM。 */
export class MarkdownHeightIndex {
  private heights: number[] = [];
  private tree: number[] = [0];
  get length() {
    return this.heights.length;
  }
  get totalHeight() {
    return this.prefix(this.length);
  }
  private prefix(count: number) {
    let height = 0;
    for (let index = Math.min(count, this.length); index > 0; index -= index & -index)
      height += this.tree[index];
    return height;
  }
  top(index: number) {
    return this.prefix(index);
  }
  height(index: number) {
    return this.heights[index] ?? 0;
  }
  append(height: number) {
    if (!Number.isFinite(height) || height < 1) throw new RangeError('排版高度必须是有限正数。');
    const index = this.length + 1;
    const value = height + this.prefix(index - 1) - this.prefix(index - (index & -index));
    this.heights.push(height);
    this.tree.push(value);
  }
  update(index: number, height: number) {
    if (!Number.isFinite(height) || height < 1 || index < 0 || index >= this.length)
      throw new RangeError('排版单元或高度无效。');
    const delta = height - this.heights[index];
    this.heights[index] = height;
    for (let current = index + 1; current <= this.length; current += current & -current)
      this.tree[current] += delta;
  }
  indexAt(height: number) {
    if (!this.length) return -1;
    let index = 0;
    let accumulated = 0;
    let bit = 1;
    while (bit * 2 <= this.length) bit *= 2;
    for (; bit; bit >>= 1) {
      const next = index + bit;
      if (next <= this.length && accumulated + this.tree[next] <= height) {
        index = next;
        accumulated += this.tree[next];
      }
    }
    return Math.min(index, this.length - 1);
  }
}

export interface MarkdownMountRange {
  from: number;
  to: number;
  visible: boolean;
}

export function estimateMarkdownBlockHeight(
  block: MarkdownStructureBlock,
  charactersPerLine = 80,
  lineHeight = 24,
) {
  const sourceLines = Math.max(1, block.toLine - block.fromLine);
  const wrappedLines = Math.ceil(
    (block.toByte - block.fromByte) / (Math.max(20, charactersPerLine) * 2),
  );
  return Math.max(
    lineHeight,
    Math.max(sourceLines, wrappedLines) * lineHeight + Math.max(1, block.layoutUnits) * 8,
  );
}
