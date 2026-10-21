import type { TableLayoutSnapshot } from '../table-layout/dom';

export interface MarkdownLayoutResult {
  height?: number;
  tables?: readonly TableLayoutSnapshot[];
}

/** 宿主级纯数据 LRU；两个上限同时生效，容量计算包含键和数值数组。 */
export class MarkdownLayoutCache {
  private entries = new Map<string, { value: MarkdownLayoutResult; bytes: number }>();
  private bytes = 0;
  hits = 0;
  misses = 0;
  constructor(readonly maxEntries = 2048, readonly maxBytes = 2 * 1024 * 1024) {}
  get size() { return this.entries.size; }
  get estimatedBytes() { return this.bytes; }
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
}
