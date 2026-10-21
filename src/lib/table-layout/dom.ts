import {
  allocateTableWidths,
  tableConstraints,
  TABLE_WIDTH_POLICY,
  type CellMetrics,
  type ColumnConstraint,
} from './widths';

export const TABLE_LAYOUT_EVENT = 'nomo:table-layout';
export const TABLE_LAYOUT_READY_EVENT = 'nomo:table-layout-ready';
export interface TableLayoutSnapshot {
  /** 同一内容版本块组中的表格序号，允许只保存已完成测量的表格。 */
  index: number;
  signature: string;
  assets: string;
  widths: number[];
  constraints: ColumnConstraint[];
  width: string;
  availableWidth: number;
}
const assetSignature = (table: HTMLElement) => JSON.stringify(
  Array.from(table.querySelectorAll('img'), image =>
    [image.currentSrc || image.src, image.complete, image.naturalWidth, image.naturalHeight]),
);
// WebView2 在字体变体属性存在时可能返回空 font 简写，必须核对实际几何分量。
const fontSignature = (style: CSSStyleDeclaration) => JSON.stringify([
  style.fontFamily, style.fontSize, style.fontWeight, style.fontStyle, style.fontStretch,
  style.fontVariant, style.lineHeight, style.letterSpacing, style.wordSpacing,
]);
export interface TableCellSource {
  key: object;
  element: HTMLTableCellElement;
  column: number;
  span: number;
  header: boolean;
  explicit?: readonly number[] | null;
  /** Fresh semantic DOM. Never return the live editable content. */
  content(): DocumentFragment;
}
export interface TableSource {
  table: HTMLTableElement;
  wrapper: HTMLElement;
  colgroup: HTMLTableColElement;
  columns(): number;
  cells(): Iterable<TableCellSource>;
}
interface Entry {
  source: TableSource;
  version: number;
  dirty: boolean;
  due: number;
  assetsRevision: number;
  constraints?: ColumnConstraint[];
  signature?: string;
  widths?: number[];
  availableWidth?: number;
  error?: Error;
  job?: { version: number; iterator: Iterator<TableCellSource>; metrics: CellMetrics[] };
}
interface CachedMetric {
  natural: number;
  atomic: number;
  chrome: number;
  signature: string;
}

/** Strip only presentation artifacts; explicit Markdown breaks survive. */
export function semanticCellContent(cell: HTMLElement): DocumentFragment {
  const clone = cell.cloneNode(true) as HTMLElement;
  clone
    .querySelectorAll(
      '.ProseMirror-widget,[data-kp-owned],.kp-break,.pm-inline-source-marker,button,input,textarea',
    )
    .forEach((node) => node.remove());
  clone.querySelectorAll<HTMLElement>('[data-kp-layout]').forEach((node) => {
    const original = node.getAttribute('data-kp-original-style');
    if (original) node.setAttribute('style', original);
    else node.removeAttribute('style');
    for (const attr of [...node.attributes])
      if (attr.name.startsWith('data-kp-')) node.removeAttribute(attr.name);
  });
  clone.querySelectorAll<HTMLElement>('.kp-unit').forEach((node) => {
    if (node.hasAttribute('data-kp-unit-original-style')) {
      node.style.cssText = node.getAttribute('data-kp-unit-original-style') ?? '';
      node.classList.remove('kp-unit');
    } else node.replaceWith(...node.childNodes);
  });
  const fragment = cell.ownerDocument.createDocumentFragment();
  fragment.append(...clone.childNodes);
  return fragment;
}

/** One scheduler, observer set and temporary measurement surface per mounted editor/export. */
export class TableLayoutController {
  snapshot(): TableLayoutSnapshot[] | undefined {
    const result: TableLayoutSnapshot[] = [];
    let index = 0;
    for (const entry of this.entries) {
      const tableIndex = index++;
      if (entry.dirty || !entry.widths || !entry.constraints || !entry.signature ||
        entry.availableWidth !== entry.source.wrapper.clientWidth) continue;
      const font = getComputedStyle(entry.source.table);
      if (entry.signature !== fontSignature(font)) continue;
      result.push({ index: tableIndex, signature: entry.signature, assets: assetSignature(entry.source.table),
        widths: [...entry.widths], constraints: entry.constraints.map(value => ({ ...value })),
        width: entry.source.table.style.width, availableWidth: entry.availableWidth });
    }
    return result.length || !this.entries.size ? result : undefined;
  }
  restore(snapshots: readonly TableLayoutSnapshot[]) {
    const entries = [...this.entries];
    const usable = snapshots.filter(saved => entries[saved.index]?.source.columns() === saved.widths.length &&
      assetSignature(entries[saved.index].source.table) === saved.assets);
    if (!usable.length) return false;
    usable.forEach(saved => {
      const entry = entries[saved.index];
      entry.signature = saved.signature;
      entry.constraints = saved.constraints.map(value => ({ ...value }));
      entry.widths = [...saved.widths];
      entry.availableWidth = saved.availableWidth;
      entry.dirty = false;
      entry.job = undefined;
      const { table, colgroup } = entry.source;
      colgroup.replaceChildren(...saved.widths.map(width => {
        const col = table.ownerDocument.createElement('col');
        col.style.width = `${width}px`;
        return col;
      }));
      table.style.width = saved.width;
      table.style.tableLayout = 'fixed';
      table.dataset.tableLayout = 'ready';
    });
    return true;
  }
  get empty() {
    return this.entries.size === 0;
  }
  private entries = new Set<Entry>();
  private cache = new Map<number, CachedMetric>();
  private keys = new WeakMap<object, number>();
  private nextKey = 0;
  private bytes = 0;
  private frame = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private destroyed = false;
  private composing = false;
  private sandbox?: HTMLElement;
  private printing = false;
  private observer?: ResizeObserver;
  private visibility?: IntersectionObserver;
  private visible = new Set<Element>();
  private sizes = new WeakMap<Element, string>();
  private assets = new Set<Element>();
  private assetsDirty = true;

  constructor(private root: HTMLElement) {
    if (typeof ResizeObserver !== 'undefined') {
      this.observer = new ResizeObserver((entries) => {
        let widthChanged = false;
        const changedAssets = new Set<Element>();
        for (const item of entries) {
          const object = this.assets.has(item.target);
          const size = object
            ? `${item.contentRect.width}:${item.contentRect.height}`
            : `${item.contentRect.width}`;
          const old = this.sizes.get(item.target);
          this.sizes.set(item.target, size);
          if (old !== undefined && old !== size) {
            if (object) changedAssets.add(item.target);
            else widthChanged = true;
          }
        }
        for (const entry of this.entries)
          if ([...changedAssets].some((asset) => entry.source.table.contains(asset))) {
            entry.assetsRevision++;
            this.change(entry);
          }
        if (widthChanged) this.schedule();
      });
      this.observer.observe(root);
    }
    this.observeViewport(null, 300);
    root.addEventListener('compositionstart', this.compositionStart);
    root.addEventListener('compositionend', this.compositionEnd);
    root.addEventListener('load', this.loaded, true);
    root.ownerDocument.fonts?.addEventListener('loadingdone', this.invalidate);
  }

  /** 指定实际滚动根；只扩大浏览器视口的 rootMargin 仍会被滚动容器裁剪。 */
  observeViewport(root: Element | null, margin: number) {
    if (typeof IntersectionObserver === 'undefined') return;
    this.visibility?.disconnect();
    this.visible.clear();
    this.visibility = new IntersectionObserver(entries => {
      for (const item of entries) {
        if (item.isIntersecting) this.visible.add(item.target);
        else this.visible.delete(item.target);
      }
      this.schedule();
    }, { root, rootMargin: `${Math.ceil(margin)}px` });
    for (const entry of this.entries) this.visibility.observe(entry.source.wrapper);
  }

  private compositionStart = () => {
    this.composing = true;
  };
  private compositionEnd = () => {
    this.composing = false;
    this.schedule();
  };
  private loaded = (event: Event) => {
    for (const entry of this.entries)
      if (event.target instanceof Node && entry.source.table.contains(event.target)) {
        entry.assetsRevision++;
        this.change(entry);
      }
  };

  register(source: TableSource) {
    const entry: Entry = { source, version: 0, dirty: true, due: 0, assetsRevision: 0 };
    this.entries.add(entry);
    this.assetsDirty = true;
    this.observer?.observe(source.wrapper);
    this.visibility?.observe(source.wrapper);
    this.schedule();
    return {
      update: () => this.change(entry),
      destroy: () => {
        this.entries.delete(entry);
        this.visible.delete(source.wrapper);
        this.observer?.unobserve(source.wrapper);
        this.visibility?.unobserve(source.wrapper);
        this.syncAssets();
      },
    };
  }

  private change(entry: Entry) {
    this.assetsDirty = true;
    entry.version++;
    entry.job = undefined;
    entry.dirty = true;
    entry.error = undefined;
    entry.due = performance.now() + TABLE_WIDTH_POLICY.debounceMs;
    this.schedule();
  }

  invalidate = () => {
    this.cache.clear();
    this.bytes = 0;
    for (const entry of this.entries) {
      this.change(entry);
      entry.due = 0;
    }
    this.schedule();
  };

  private schedule = () => {
    if (this.destroyed || this.composing || this.frame) return;
    clearTimeout(this.timer);
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.run(false);
    });
  };

  private syncAssets() {
    for (const element of this.assets)
      if (!this.root.contains(element)) {
        this.observer?.unobserve(element);
        this.assets.delete(element);
      }
    for (const entry of this.entries)
      for (const element of entry.source.table.querySelectorAll('img,.math-inline')) {
        if (this.assets.has(element)) continue;
        this.assets.add(element);
        this.observer?.observe(element);
      }
  }

  private cachePut(id: number, value: CachedMetric) {
    const cost = (entry: CachedMetric) => 64 + entry.signature.length * 2;
    const previous = this.cache.get(id);
    if (previous) {
      this.bytes -= cost(previous);
      this.cache.delete(id);
    }
    this.cache.set(id, value);
    this.bytes += cost(value);
    while (
      this.cache.size > TABLE_WIDTH_POLICY.cacheEntries ||
      this.bytes > TABLE_WIDTH_POLICY.cacheBytes
    ) {
      const oldest = this.cache.keys().next().value!;
      this.bytes -= cost(this.cache.get(oldest)!);
      this.cache.delete(oldest);
    }
  }

  private prepare(cell: TableCellSource, signature: string) {
    let id = this.keys.get(cell.key);
    if (id === undefined) {
      id = ++this.nextKey;
      this.keys.set(cell.key, id);
    }
    const cached = this.cache.get(id);
    if (cached?.signature === signature) {
      this.cachePut(id, cached);
      return { cell, metric: cached };
    }
    const style = getComputedStyle(cell.element);
    const doc = this.root.ownerDocument;
    const box = doc.createElement('div');
    box.style.cssText = `font:${style.font};font-family:${style.fontFamily};font-size:${style.fontSize};font-weight:${style.fontWeight};letter-spacing:${style.letterSpacing};line-height:${style.lineHeight};width:max-content;max-width:none;white-space:pre;`;
    box.append(cell.content());
    for (const tag of ['strong', 'em', 'a', 'mark', 'del', 'u', 'sup', 'sub']) {
      const live = cell.element.querySelector(tag);
      if (!live) continue;
      const font = getComputedStyle(live);
      box.querySelectorAll<HTMLElement>(tag).forEach((node) => {
        node.style.font = font.font;
        node.style.letterSpacing = font.letterSpacing;
      });
    }
    box.querySelectorAll<HTMLElement>('p').forEach((p) => {
      p.style.cssText += ';margin:0;width:max-content;max-width:none;white-space:pre;';
    });
    box.querySelectorAll<HTMLElement>('code').forEach((code) => {
      const live = cell.element.querySelector('code,.pm-inline-source-code');
      if (live) {
        const font = getComputedStyle(live);
        code.style.font = font.font;
        code.style.padding = font.padding;
      }
      code.style.whiteSpace = 'pre';
    });
    box.querySelectorAll<HTMLElement>('.math-inline').forEach((math) => {
      const live = [...cell.element.querySelectorAll<HTMLElement>('.math-inline')].find(
        (item) => item.dataset.tex === math.dataset.tex,
      );
      const rendered = live?.querySelector('.katex');
      if (rendered) math.replaceChildren(rendered.cloneNode(true));
    });
    box.querySelectorAll<HTMLImageElement>('img').forEach((image) => {
      const width = image.getAttribute('width') || image.style.width;
      const px =
        width && /^\d+(\.\d+)?(px)?$/.test(width)
          ? parseFloat(width)
          : 12 * (parseFloat(style.fontSize) || 14);
      image.style.cssText = `width:${px}px;height:auto;max-width:none;`;
    });
    const chrome =
      (parseFloat(style.paddingLeft) || 0) +
      (parseFloat(style.paddingRight) || 0) +
      (parseFloat(style.borderRightWidth) || 0);
    return { cell, box, id, signature, chrome };
  }

  private run(force: boolean) {
    if (this.destroyed || this.composing) return;
    const start = performance.now();
    let nextDue = Infinity;
    const entries = [...this.entries].sort(
      (a, b) =>
        Number(this.visible.has(b.source.wrapper)) - Number(this.visible.has(a.source.wrapper)),
    );
    for (const entry of entries) {
      if (!force && this.visibility && !this.visible.has(entry.source.wrapper)) continue;
      if (!entry.source.wrapper.isConnected || entry.source.wrapper.clientWidth <= 0) continue;
      const font = getComputedStyle(entry.source.table);
      const signature = fontSignature(font);
      if (entry.signature !== signature) {
        entry.signature = signature;
        entry.version++;
        entry.job = undefined;
        entry.dirty = true;
        entry.error = undefined;
      }
      if (entry.error) continue;
      if (entry.dirty && entry.due > performance.now() && !force) {
        nextDue = Math.min(nextDue, entry.due);
        continue;
      }
      try {
        if (entry.dirty) {
          entry.job ??= {
            version: entry.version,
            iterator: entry.source.cells()[Symbol.iterator](),
            metrics: [],
          };
          const job = entry.job;
          let done = false;
          do {
            const batch: ReturnType<TableLayoutController['prepare']>[] = [];
            // Batch writes/reads. A large cell is indivisible; the budget is cooperative.
            for (let i = 0; i < 8; i++) {
              const next = job.iterator.next();
              if (next.done) {
                done = true;
                break;
              }
              batch.push(
                this.prepare(
                  next.value,
                  `${signature}|${entry.assetsRevision}|${next.value.header}`,
                ),
              );
              if (performance.now() - start >= TABLE_WIDTH_POLICY.batchMs) break;
            }
            if (batch.some((item) => item.box)) {
              this.sandbox ??= this.root.ownerDocument.createElement('div');
              this.sandbox.style.cssText =
                'position:fixed;left:-100000px;top:0;visibility:hidden;pointer-events:none;width:max-content;contain:layout style;';
              this.root.ownerDocument.body.append(this.sandbox);
              this.sandbox.replaceChildren(
                ...batch.flatMap((item) => (item.box ? [item.box] : [])),
              );
            }
            for (const item of batch) {
              const metric = item.metric ?? {
                natural: item.box!.getBoundingClientRect().width,
                atomic: Math.max(
                  0,
                  ...[...item.box!.querySelectorAll<HTMLElement>('.math-inline')].map(
                    (node) => node.getBoundingClientRect().width,
                  ),
                ),
                chrome: item.chrome!,
                signature: item.signature!,
              };
              if (!item.metric) this.cachePut(item.id!, metric);
              job.metrics.push({
                natural: metric.natural,
                atomic: metric.atomic,
                chrome: metric.chrome,
                column: item.cell.column,
                span: item.cell.span,
                header: item.cell.header,
                explicit: item.cell.explicit,
              });
            }
            this.sandbox?.replaceChildren();
          } while (!done && performance.now() - start < TABLE_WIDTH_POLICY.batchMs);
          if (done && job.version === entry.version) {
            entry.constraints = tableConstraints(
              entry.source.columns(),
              job.metrics,
              parseFloat(font.fontSize) || 14,
            );
            entry.dirty = false;
            entry.job = undefined;
          }
        }
        if (!entry.dirty && entry.constraints) this.apply(entry);
      } catch (error) {
        entry.error = error instanceof Error ? error : new Error(String(error));
        entry.job = undefined;
        entry.source.table.dataset.tableLayoutError = entry.error.message;
      }
      if (entry.dirty && performance.now() - start >= TABLE_WIDTH_POLICY.batchMs) break;
    }
    this.sandbox?.remove();
    if (this.assetsDirty && !entries.some(entry => entry.job)) {
      this.syncAssets();
      this.assetsDirty = false;
    }
    const unfinished = entries.some(
      (entry) =>
        entry.dirty &&
        !entry.error &&
        (force || !this.visibility || this.visible.has(entry.source.wrapper)) &&
        entry.source.wrapper.clientWidth > 0,
    );
    if (unfinished && !force) {
      if (nextDue !== Infinity && nextDue > performance.now())
        this.timer = setTimeout(this.schedule, nextDue - performance.now());
      else this.schedule();
    }
  }

  private apply(entry: Entry) {
    const { table, wrapper, colgroup } = entry.source;
    const style = getComputedStyle(table);
    const border =
      (parseFloat(style.borderLeftWidth) || 0) + (parseFloat(style.borderRightWidth) || 0);
    const available = wrapper.clientWidth - border;
    entry.availableWidth = wrapper.clientWidth;
    const result = allocateTableWidths(entry.constraints!, available);
    const same =
      entry.widths?.length === result.columns.length &&
      entry.widths.every((width, index) => Math.abs(width - result.columns[index]) < 1);
    if (!same) {
      // Synchronous event invalidates existing KP geometry before any width is written.
      table.dispatchEvent(new CustomEvent(TABLE_LAYOUT_EVENT, { bubbles: true }));
      while (colgroup.childElementCount > result.columns.length)
        colgroup.lastElementChild!.remove();
      result.columns.forEach((width, index) => {
        const col =
          (colgroup.children[index] as HTMLElement | undefined) ??
          colgroup.appendChild(table.ownerDocument.createElement('col'));
        col.style.width = `${width}px`;
      });
      table.style.width = `${result.width + border}px`;
      table.style.tableLayout = 'fixed';
      table.dataset.tableLayout = 'ready';
      table.dataset.tableLayoutVersion = String(entry.version);
      delete table.dataset.tableLayoutError;
      entry.widths = result.columns;
      table.dispatchEvent(new CustomEvent(TABLE_LAYOUT_READY_EVENT, { bubbles: true }));
    }
    // zoom participates in print layout, unlike transform which would leave the old height.
    table.style.setProperty(
      'zoom',
      this.printing && result.overflow ? String(available / result.width) : '1',
    );
  }

  async flush(printing = this.printing) {
    this.printing = printing;
    const deadline = performance.now() + 30_000;
    do {
      if (this.destroyed) return;
      if (performance.now() > deadline) throw new Error('Table layout did not settle');
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      this.run(true);
      const error = [...this.entries].find((entry) => entry.error)?.error;
      if (error) throw error;
    } while (
      this.composing ||
      [...this.entries].some((entry) => entry.dirty && entry.source.wrapper.clientWidth > 0)
    );
  }

  destroy() {
    this.destroyed = true;
    cancelAnimationFrame(this.frame);
    clearTimeout(this.timer);
    this.observer?.disconnect();
    this.visibility?.disconnect();
    this.root.removeEventListener('compositionstart', this.compositionStart);
    this.root.removeEventListener('compositionend', this.compositionEnd);
    this.root.removeEventListener('load', this.loaded, true);
    this.root.ownerDocument.fonts?.removeEventListener('loadingdone', this.invalidate);
    this.sandbox?.remove();
    this.sandbox = undefined;
    this.entries.clear();
    this.assets.clear();
    this.visible.clear();
    this.cache.clear();
    this.bytes = 0;
  }
}
