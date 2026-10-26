import type { MarkdownStructureBlock } from './structure';
import type { MarkdownMountRange } from './viewport';
import viewportPolicy from './viewportPolicy.json';

export const MARKDOWN_VIEWPORT_POLICY = Object.freeze(viewportPolicy);

export interface MarkdownBlockGroup extends MarkdownMountRange {
  id: string;
  blockIds: readonly string[];
}

/** 宿主已核对正文、修订和完整结构边界的现有窗口。 */
export interface MarkdownRetainedGroup { id: string; from: number; to: number; }

/** 已发布组封口；追加索引不会扩展旧组，编辑只重组失配区域。 */
export class MarkdownStableGroups {
  private groups: MarkdownBlockGroup[] = [];
  private serial = 0;
  private indexedBlocks: readonly MarkdownStructureBlock[] | undefined;
  private indexedLength = 0;
  private lastBlock: MarkdownStructureBlock | undefined;
  update(blocks: readonly MarkdownStructureBlock[], retained: readonly MarkdownRetainedGroup[] = []): readonly MarkdownBlockGroup[] {
    const preserved = [...retained].sort((a, b) => a.from - b.from);
    for (let i = 0; i < preserved.length; i++) {
      const range = preserved[i];
      if (range.from < 0 || range.to <= range.from || range.to > blocks.length ||
        (i > 0 && preserved[i - 1].to > range.from)) throw new Error('保留窗口的结构范围无效。');
    }
    // 同一索引只会追加或延长尾块；已发布组仍封口，不再扫描已确认前缀。
    if (!preserved.length && blocks === this.indexedBlocks && blocks.length >= this.indexedLength) {
      const tail = this.groups.at(-1);
      if (this.indexedLength && blocks[this.indexedLength - 1]?.id !== this.lastBlock?.id) {
        // 身份改变由完整匹配路径处理，不能用追加假设掩盖索引替换。
      } else {
        if (tail && blocks[this.indexedLength - 1] !== this.lastBlock)
          this.groups[this.groups.length - 1] = { ...tail };
        this.append(blocks, this.indexedLength);
        this.remember(blocks);
        return this.groups;
      }
    }
    const old = new Map(this.groups.map(group => [group.blockIds[0], group]));
    const next: MarkdownBlockGroup[] = [];
    let retainedIndex = 0;
    for (let from = 0; from < blocks.length;) {
      const keep = preserved[retainedIndex];
      if (keep?.from === from) {
        next.push({ ...keep, visible: false, blockIds: blocks.slice(from, keep.to).map(block => block.id) });
        from = keep.to;
        retainedIndex++;
        continue;
      }
      const end = keep?.from ?? blocks.length;
      const previous = old.get(blocks[from].id);
      const matches = previous && from + previous.blockIds.length <= end &&
        previous.blockIds.length <= MARKDOWN_VIEWPORT_POLICY.groupBlocks &&
        (previous.blockIds.length === 1 || blocks[from + previous.blockIds.length - 1]?.toByte - blocks[from].fromByte <= MARKDOWN_VIEWPORT_POLICY.groupBytes) &&
        previous.blockIds.every((id, i) => blocks[from + i]?.id === id);
      if (matches) {
        next.push({ ...previous, from, to: from + previous.blockIds.length });
        from += previous.blockIds.length;
        continue;
      }
      let to = from + 1;
      while (to < end && blocks[from].kind !== 'native_region' && blocks[to].kind !== 'native_region' && blocks[from].mode === 'semantic' &&
        blocks[to].mode === 'semantic' && !old.has(blocks[to].id) &&
        to - from < MARKDOWN_VIEWPORT_POLICY.groupBlocks &&
        blocks[to].toByte - blocks[from].fromByte <= MARKDOWN_VIEWPORT_POLICY.groupBytes) to++;
      next.push({ id: `group-${++this.serial}`, from, to, visible: false,
        blockIds: blocks.slice(from, to).map(block => block.id) });
      from = to;
    }
    this.groups = next;
    this.remember(blocks);
    return this.groups;
  }
  private append(blocks: readonly MarkdownStructureBlock[], start: number) {
    for (let from = start; from < blocks.length;) {
      let to = from + 1;
      while (to < blocks.length && blocks[from].kind !== 'native_region' && blocks[to].kind !== 'native_region' && blocks[from].mode === 'semantic' && blocks[to].mode === 'semantic' &&
        to - from < MARKDOWN_VIEWPORT_POLICY.groupBlocks &&
        blocks[to].toByte - blocks[from].fromByte <= MARKDOWN_VIEWPORT_POLICY.groupBytes) to++;
      this.groups.push({ id: `group-${++this.serial}`, from, to, visible: false,
        blockIds: blocks.slice(from, to).map(block => block.id) });
      from = to;
    }
  }
  private remember(blocks: readonly MarkdownStructureBlock[]) {
    this.indexedBlocks = blocks; this.indexedLength = blocks.length; this.lastBlock = blocks.at(-1);
  }
  clear() { this.groups = []; this.indexedBlocks = undefined; this.indexedLength = 0; this.lastBlock = undefined; }
}

/** 只保存标量和最近 32 次耗时；程序定位及大幅离散跳转不参与速度预测。 */
export class MarkdownScrollDemand {
  private previous?: { top: number; time: number };
  private velocity = 0;
  private before = 3;
  private after = 3;
  private samples: number[] = [];
  lastScroll = -Infinity;
  sample(top: number, height: number, now: number) {
    const previous = this.previous;
    this.previous = { top, time: now };
    this.lastScroll = now;
    if (!previous || now - previous.time > 300 || Math.abs(top - previous.top) > height * 6) {
      this.velocity = 0;
      return;
    }
    const speed = (top - previous.top) / Math.max(1, now - previous.time);
    this.velocity = this.velocity * .65 + speed * .35;
  }
  reset(top: number, now: number) { this.previous = { top, time: now }; this.velocity = 0; }
  prepared(milliseconds: number) {
    this.samples.push(Math.max(0, milliseconds));
    if (this.samples.length > 32) this.samples.shift();
  }
  get preparationMs() {
    if (!this.samples.length) return 300;
    const sorted = [...this.samples].sort((a, b) => a - b);
    return sorted[Math.ceil(sorted.length * .95) - 1] + 150;
  }
  range(height: number, now: number) {
    const scrolling = now - this.lastScroll < MARKDOWN_VIEWPORT_POLICY.quietMs;
    if (!scrolling) { this.velocity = 0; this.before = this.after = 3; }
    const screens = Math.max(3, Math.min(6, Math.abs(this.velocity) * this.preparationMs / Math.max(1, height)));
    if (this.velocity < 0) this.before = Math.max(this.before, screens);
    else this.after = Math.max(this.after, screens);
    return { before: this.before * height, after: this.after * height,
      retainBefore: (this.before + 3) * height, retainAfter: (this.after + 3) * height, scrolling };
  }
}
