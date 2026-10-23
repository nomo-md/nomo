import { markdownWorkScheduler, MarkdownTaskPriority } from '../../lib/markdown-document/scheduler';
import { getOutlineWindow, type OutlineProjection, type OutlineWindowRow } from './outlineProjection';

export interface OutlineViewportSnapshot {
  rows: OutlineWindowRow[];
  height: number;
  pitch: number;
  loading: boolean;
  focusedId: string;
  generation: number;
}

/** 一个可见面板一个控制器。文档投影不反向持有这里的 DOM 或 publish 闭包。 */
export class OutlineViewportController {
  private frame = 0;
  private disposed = false;
  private buffered = false;
  private pendingRestore = true;
  private generation = -1;
  private pitch = 32;
  private listTop = 54;
  private height = 320;
  private rows: OutlineWindowRow[] = [];
  private focusedId = '';
  private pendingFocus = '';
  private resize: ResizeObserver;
  private lastScrollTop = 0;
  private restoreOffset = -1;

  constructor(private panel: HTMLElement, private projection: OutlineProjection,
    private publish: (snapshot: OutlineViewportSnapshot) => void) {
    this.resize = new ResizeObserver(this.schedule);
    this.resize.observe(panel);
    const measure = panel.querySelector<HTMLElement>('[data-outline-measure]');
    if (measure) this.resize.observe(measure);
    panel.addEventListener('scroll', this.onScroll, { passive: true });
    panel.addEventListener('wheel', this.onIntent, { passive: true });
    panel.addEventListener('pointerdown', this.onIntent);
    panel.addEventListener('focusin', this.onFocus);
    document.fonts?.addEventListener('loadingdone', this.schedule);
    this.update();
  }

  update() {
    if (this.disposed || this.generation === this.projection.generation) return;
    // 折叠/编辑重建之前的行仍是旧窗口，用其 ID 保存锚点，不能读取已清空的新索引。
    if (this.generation >= 0) this.capture();
    this.generation = this.projection.generation;
    this.pendingRestore = true;
    this.enqueue(!this.rows.length);
    this.schedule();
  }

  private enqueue(urgent = false) {
    if (this.disposed || this.projection.complete) return;
    const generation = this.projection.generation;
    markdownWorkScheduler.enqueue(this, 'outline-projection', urgent ? MarkdownTaskPriority.NearViewport : MarkdownTaskPriority.Index, () => {
      if (this.disposed || generation !== this.projection.generation) return;
      const more = this.projection.advance(generation);
      this.schedule();
      if (more) this.enqueue();
    });
  }

  private schedule = () => {
    if (!this.disposed && !this.frame) this.frame = requestAnimationFrame(this.render);
  };

  private render = () => {
    this.frame = 0;
    if (this.disposed) return;
    const list = this.panel.querySelector<HTMLElement>('.content-outline-list');
    const measure = this.panel.querySelector<HTMLElement>('[data-outline-measure]');
    const rect = this.panel.getBoundingClientRect();
    // 只测一个永久测量行与面板，随后统一发布窗口，避免逐行读写布局。
    if (measure?.offsetHeight) this.pitch = measure.offsetHeight + 8;
    if (list) this.listTop = list.getBoundingClientRect().top - rect.top + this.panel.scrollTop - this.panel.clientTop;
    this.height = Math.max(this.pitch, this.panel.clientHeight - Math.max(0, this.listTop - this.panel.scrollTop));
    const anchor = this.projection.anchor;
    if (this.pendingRestore) {
      const position = anchor ? this.projection.positionFor(anchor) : 0;
      if (position !== null) {
        this.restoreOffset = anchor ? this.listTop + position * this.pitch + Math.min(anchor.offset, this.pitch - 1) : 0;
        this.pendingRestore = false;
      }
    }
    const scrollTop = this.restoreOffset >= 0 ? this.restoreOffset : this.panel.scrollTop;
    this.restoreOffset = -1;
    const top = Math.max(0, scrollTop - this.listTop);
    this.rows = getOutlineWindow(this.projection, top, this.height, this.pitch, this.buffered, this.focusedId);
    this.publish({ rows: this.rows, height: Math.max(0, this.projection.visible.length * this.pitch - 8),
      pitch: this.pitch, loading: !this.projection.ready, focusedId: this.focusedId, generation: this.projection.generation });
    // 等 Svelte 提交新的占位高度后恢复，防止浏览器先把深处 scrollTop 截断。
    queueMicrotask(() => {
      if (this.disposed) return;
      if (!this.pendingRestore && Math.abs(this.panel.scrollTop - scrollTop) > 0.5) this.panel.scrollTop = scrollTop;
      this.lastScrollTop = this.panel.scrollTop;
      this.capture();
      if (this.pendingFocus) {
        const target = Array.from(this.panel.querySelectorAll<HTMLElement>('[data-outline-id]'))
          .find(row => row.dataset.outlineId === this.pendingFocus)?.querySelector<HTMLButtonElement>('.outline-link');
        if (target) { target.focus({ preventScroll: true }); this.pendingFocus = ''; }
      }
    });
    if (!this.buffered) {
      markdownWorkScheduler.enqueue(this, 'outline-buffer', MarkdownTaskPriority.Prefetch, () => {
        this.buffered = true; this.schedule();
      });
    }
    // 调度器共享 4ms 预算；可处理多片，但 DOM 最多每帧发布一次。
    this.enqueue(this.pendingRestore || this.projection.visible.length * this.pitch < top + this.height);
  };

  capture() {
    if (this.pendingRestore || !this.rows.length) return;
    const top = Math.max(0, this.panel.scrollTop - this.listTop);
    const position = Math.floor(top / this.pitch);
    const row = this.rows.find(row => row.position === position);
    if (row) this.projection.anchor = { id: row.item.id, key: this.projection.identity(row.item), index: row.index, offset: top - position * this.pitch };
  }

  private onScroll = () => {
    if (this.panel.scrollTop !== this.lastScrollTop) this.capture();
    this.lastScrollTop = this.panel.scrollTop;
    markdownWorkScheduler.interacting();
    this.schedule();
  };

  private onIntent = () => {
    this.pendingRestore = false;
    markdownWorkScheduler.interacting();
  };

  private onFocus = (event: FocusEvent) => {
    const row = (event.target as HTMLElement)?.closest<HTMLElement>('[data-outline-id]');
    this.focusedId = row?.dataset.outlineId ?? '';
    this.schedule();
  };

  keydown(event: KeyboardEvent, toggle: (item: OutlineWindowRow['item']) => void) {
    const row = (event.target as HTMLElement)?.closest<HTMLElement>('[data-outline-id]');
    if (!row || event.altKey || event.ctrlKey || event.metaKey) return;
    const model = this.projection;
    let position = Number(row.dataset.outlinePosition);
    const index = model.visible[position];
    const item = model.headings[index];
    if (!item) return;
    let folded = false;
    switch (event.key) {
      case 'ArrowDown': position++; break;
      case 'ArrowUp': position--; break;
      case 'PageDown': position += Math.max(1, Math.floor(this.height / this.pitch)); break;
      case 'PageUp': position -= Math.max(1, Math.floor(this.height / this.pitch)); break;
      case 'Home': position = 0; break;
      case 'End': position = model.visible.length - 1; break;
      case 'ArrowRight':
        if (model.headings[index + 1]?.level > item.level) {
          if (model.isCollapsed(item)) { this.capture(); toggle(item); this.pendingFocus = item.id; this.update(); folded = true; }
          else position++;
        }
        break;
      case 'ArrowLeft':
        if (model.headings[index + 1]?.level > item.level && !model.isCollapsed(item)) {
          this.capture(); toggle(item); this.pendingFocus = item.id; this.update();
          folded = true;
        } else if (model.parents[index] >= 0) position = model.positions[model.parents[index]];
        break;
      default: return;
    }
    event.preventDefault();
    if (folded) { this.schedule(); return; }
    this.onIntent();
    if (!model.complete) { this.schedule(); return; }
    position = Math.max(0, Math.min(model.visible.length - 1, position));
    this.focusedId = model.headings[model.visible[position]]?.id ?? '';
    this.pendingFocus = this.focusedId;
    const top = this.listTop + position * this.pitch;
    if (top < this.panel.scrollTop) this.panel.scrollTop = top;
    else if (top + this.pitch > this.panel.scrollTop + this.panel.clientHeight)
      this.panel.scrollTop = top + this.pitch - this.panel.clientHeight;
    this.schedule();
  }

  destroy() {
    if (this.disposed) return;
    this.capture();
    this.disposed = true;
    markdownWorkScheduler.cancel(this);
    if (this.frame) cancelAnimationFrame(this.frame);
    this.resize.disconnect();
    this.panel.removeEventListener('scroll', this.onScroll);
    this.panel.removeEventListener('wheel', this.onIntent);
    this.panel.removeEventListener('pointerdown', this.onIntent);
    this.panel.removeEventListener('focusin', this.onFocus);
    document.fonts?.removeEventListener('loadingdone', this.schedule);
    this.rows = [];
  }
}
