import type { OutlineItem } from '../../lib/outline/outlineService';
import type { DocumentTaskState } from '../../lib/markdown-document/protocol';

export interface OutlineListAnchor {
  id: string;
  key?: string;
  index: number;
  offset: number;
}

export type OutlineIndexPhase = DocumentTaskState;

/**
 * 随文档保留的纯数据。没有 DOM、订阅、定时器或组件回调；任务由可见面板拥有。
 * 仅保留当前修订：三个数值索引约 24B/标题，另有 ID 映射与折叠状态；不复制标题或正文。
 * 修订替换重用/清空派生容器，完整索引删除失效状态，关闭文档 dispose 全部释放。
 */
export class OutlineProjection {
  headings: readonly OutlineItem[] = [];
  revision = -1;
  generation = 0;
  phase: OutlineIndexPhase = 'building';
  failure: unknown = null;
  anchor: OutlineListAnchor | null = null;
  activeId = '';
  processed = 0;
  readonly visible: number[] = [];
  readonly parents: number[] = [];
  readonly positions: number[] = [];
  readonly collapsedIds = new Set<string>();
  readonly ids = new Map<string, number>();
  private keys = new Map<string, number>();
  expandableCount = 0;
  collapsedExpandableCount = 0;
  private stack: number[] = [];
  private count = 0;
  private overrides = new Map<string, boolean>();
  private pruning: Iterator<string> | null = null;
  private disposed = false;

  private defaultLevel: number;
  constructor(readonly documentId: string, private preferredLevel = 6) { this.defaultLevel = preferredLevel; }

  get complete() { return this.processed === this.headings.length && !this.pruning; }
  get ready() { return !this.disposed && this.phase === 'ready' && this.complete && !this.failure; }

  update(headings: readonly OutlineItem[], revision: number, phase: OutlineIndexPhase, failure: unknown = null) {
    if (this.disposed || revision < this.revision) return;
    const reset = revision !== this.revision || headings !== this.headings || headings.length < this.count;
    const changed = reset || headings.length !== this.count || phase !== this.phase || failure !== this.failure;
    this.headings = headings;
    this.revision = revision;
    this.phase = phase;
    this.failure = failure;
    this.count = headings.length;
    if (reset) this.reset();
    else if (changed) this.generation++;
    if (changed && phase === 'ready') this.pruning = this.overrides.keys();
  }

  private reset() {
    this.generation++;
    this.processed = 0;
    this.visible.length = this.parents.length = this.positions.length = this.stack.length = 0;
    this.ids.clear();
    this.keys.clear();
    this.collapsedIds.clear();
    this.expandableCount = this.collapsedExpandableCount = 0;
    this.pruning = this.phase === 'ready' ? this.overrides.keys() : null;
  }

  identity(item: OutlineItem) {
    // 结构索引在未受编辑影响的前缀/尾部保留 blockId；重复标题的 slug 会重新编号。
    return 'blockId' in item && typeof item.blockId === 'string'
      ? `${item.blockId}:${'blockHeadingIndex' in item ? item.blockHeadingIndex : 0}` : item.id;
  }

  isCollapsed(item: OutlineItem) {
    return this.overrides.get(this.identity(item)) ?? item.level >= this.defaultLevel;
  }

  toggle(item: OutlineItem) {
    this.overrides.set(this.identity(item), !this.isCollapsed(item));
    this.reset();
  }

  setDefaultLevel(level: number) {
    this.defaultLevel = level;
    this.overrides.clear();
    this.reset();
  }

  resetExpansion(level = this.preferredLevel) {
    this.preferredLevel = level;
    this.setDefaultLevel(level);
  }

  /** 每片最多 128 项 / 4ms；追加时保留栈，只处理新标题和相邻边界。 */
  advance(generation = this.generation, now = () => performance.now()) {
    if (this.disposed || generation !== this.generation) return false;
    const started = now();
    let work = 0;
    while (this.processed < this.headings.length && work < 128 && now() - started < 4) {
      const index = this.processed++;
      const item = this.headings[index];
      const previous = this.headings[index - 1];
      if (previous && previous.level < item.level) {
        this.expandableCount++;
        if (this.isCollapsed(previous)) this.collapsedExpandableCount++;
      }
      while (this.stack.length && this.headings[this.stack.at(-1)!].level >= item.level) this.stack.pop();
      const parent = this.stack.at(-1) ?? -1;
      this.parents.push(parent);
      const visible = parent < 0 || (this.positions[parent] >= 0 && !this.isCollapsed(this.headings[parent]));
      this.positions.push(visible ? this.visible.length : -1);
      if (visible) this.visible.push(index);
      if (this.isCollapsed(item)) this.collapsedIds.add(item.id);
      this.ids.set(item.id, index);
      this.keys.set(this.identity(item), index);
      this.stack.push(index);
      work++;
    }
    // 只在同修订的完整索引上清理已删除标题的用户状态；partial 不代表标题已删除。
    while (this.processed === this.headings.length && this.pruning && work < 128 && now() - started < 4) {
      const next = this.pruning.next();
      if (next.done) { this.pruning = null; break; }
      if (!this.keys.has(next.value)) this.overrides.delete(next.value);
      work++;
    }
    return !this.complete;
  }

  positionFor(anchor: OutlineListAnchor) {
    let index = anchor.key ? this.keys.get(anchor.key) : this.ids.get(anchor.id);
    if (index === undefined) {
      if (!this.ready) return null;
      index = Math.min(anchor.index, this.processed - 1);
    }
    while (index >= 0 && this.positions[index] < 0) index = this.parents[index];
    return index >= 0 ? this.positions[index] : 0;
  }

  dispose() {
    this.disposed = true;
    this.reset();
    this.headings = [];
    this.overrides.clear();
    this.anchor = null;
    this.pruning = null;
  }
}

export interface OutlineWindowRow {
  item: OutlineItem;
  index: number;
  position: number;
}

/** 固定行距虚拟窗口；仅复制视口附近的引用，额外保留至多一个键盘焦点行。 */
export function getOutlineWindow(projection: OutlineProjection, top: number, height: number, pitch: number,
  buffered: boolean, focusedId = ''): OutlineWindowRow[] {
  const buffer = buffered ? height : 0;
  const start = Math.max(0, Math.floor((top - buffer) / pitch));
  const end = Math.min(projection.visible.length, Math.ceil((top + height + buffer) / pitch));
  const positions: number[] = [];
  for (let position = start; position < end; position++) positions.push(position);
  const focusedIndex = projection.ids.get(focusedId);
  const focusedPosition = focusedIndex === undefined ? -1 : projection.positions[focusedIndex];
  if (focusedPosition >= 0 && (focusedPosition < start || focusedPosition >= end)) {
    positions.push(focusedPosition);
    positions.sort((a, b) => a - b);
  }
  return positions.map(position => {
    const index = projection.visible[position];
    return { index, position, item: projection.headings[index] };
  });
}
