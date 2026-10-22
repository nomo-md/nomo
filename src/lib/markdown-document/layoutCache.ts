import type { TableLayoutSnapshot } from '../table-layout/dom';

export interface MarkdownLayoutResult {
  height?: number;
  tables?: readonly TableLayoutSnapshot[];
}

/** 纯数据 LRU；两个上限同时生效，容量计算包含键和数值数组。 */
export class MarkdownLayoutCache {
  private entries = new Map<string, { value: MarkdownLayoutResult; bytes: number }>();
  private bytes = 0;
  hits = 0;
  misses = 0;
  constructor(readonly maxEntries = 2048, readonly maxBytes = 2 * 1024 * 1024) {}
  get size() { return this.entries.size; }
  get estimatedBytes() { return this.bytes; }
  delete(key: string) {
    const entry = this.entries.get(key);
    if (!entry) return false;
    this.bytes -= entry.bytes;
    return this.entries.delete(key);
  }
  get(key: string) {
    const entry = this.entries.get(key);
    if (!entry) { this.misses++; return undefined; }
    this.hits++;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }
  put(key: string, value: MarkdownLayoutResult) {
    const bytes = 64 + key.length * 2 + (value.tables?.reduce((sum, table) =>
      sum + 128 + (table.signature.length + table.assets.length + table.width.length) * 2 + table.widths.length * 8 + table.constraints.length * 40, 0) ?? 0);
    const previous = this.entries.get(key);
    if (previous) { this.bytes -= previous.bytes; this.entries.delete(key); }
    if (bytes > this.maxBytes) return;
    this.entries.set(key, { value, bytes });
    this.bytes += bytes;
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes;
      this.entries.delete(oldest);
    }
  }
  clear() { this.entries.clear(); this.bytes = 0; }
  scope(prefix: string) {
    const cache = this;
    return {
      hits: 0, misses: 0,
      get size() { return [...cache.entries.keys()].filter(key => key.startsWith(prefix)).length; },
      get estimatedBytes() {
        let bytes = 0;
        for (const [key, entry] of cache.entries) if (key.startsWith(prefix)) bytes += entry.bytes;
        return bytes;
      },
      get(key: string) {
        const value = cache.get(prefix + key);
        if (value) this.hits++; else this.misses++;
        return value;
      },
      put(key: string, value: MarkdownLayoutResult) { cache.put(prefix + key, value); },
      delete(key: string) { return cache.delete(prefix + key); },
      clear() {
        for (const key of cache.entries.keys()) if (key.startsWith(prefix)) cache.delete(key);
      },
    };
  }
}

// 同一 WebView 的所有文档/模式共用 2 MiB，而非每个后台标签增加一份容量。
// 只跨视图保存高度、列宽等数值；关闭文档即删除其分区，不持有 DOM 或视图回调。
const sharedLayouts = new MarkdownLayoutCache();
let ownerSequence = 0;
let owners = 0;
const invalidateFonts = () => sharedLayouts.clear();
export function createMarkdownDocumentLayouts() {
  const prefix = `${++ownerSequence}:`;
  const semantic = sharedLayouts.scope(`${prefix}semantic:`);
  const source = sharedLayouts.scope(`${prefix}source:`);
  if (owners++ === 0 && typeof document !== 'undefined')
    document.fonts?.addEventListener('loadingdone', invalidateFonts);
  let disposed = false;
  return { semantic, source, dispose() {
    if (disposed) return;
    disposed = true;
    semantic.clear(); source.clear();
    if (--owners === 0 && typeof document !== 'undefined')
      document.fonts?.removeEventListener('loadingdone', invalidateFonts);
  } };
}
