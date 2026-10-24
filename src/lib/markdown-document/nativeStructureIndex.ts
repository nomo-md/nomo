import { MarkdownStructureIndex, type MarkdownOutlineItem } from './structureIndex';
import { NativeStructureClient } from './nativeStructureClient';
import { emptyEnhancements, addEnhancements, enhancementTotal } from './enhancements';
import type { MarkdownStructureBlock, MarkdownReference } from './structure';
import type { DocumentIndexProgress } from './protocol';
import type { StructureChanges, StructureCursor, StructureHeading, StructureReference, StructureResponse } from './structureProtocol';

type Geometry = { fromByte: number; toByte: number; layoutUnits: number; blockCount: number;
  enhancements: MarkdownStructureBlock['enhancements']; metricsComplete: boolean };
type Detail = { blocks: MarkdownStructureBlock[]; headings: MarkdownOutlineItem[]; references: Record<string, MarkdownReference> };
const MAX_REGIONS = 128;
const MAX_DETAIL_BLOCKS = 256;
const MAX_RANGE_BLOCKS = 128;

/** 宿主兼容视图：数组仅含粗几何区域和当前挂载范围，绝非全文块/标题数组。 */
export class NativeMarkdownStructureIndex extends MarkdownStructureIndex {
  private regions: Geometry[] = [];
  private parts = new Map<object, Detail>();
  private projected: MarkdownStructureBlock[] = [];
  private localHeadings: MarkdownOutlineItem[] = [];
  private status: StructureResponse | null = null;
  private detailGeneration = 0;
  private detailLease = Symbol('visible-structure');
  private geometryLease = Symbol('structure-geometry');
  geometryGeneration = 0;
  changes: StructureChanges | null = null;
  /** 仅迁移原生确认的共享片段；正文和依赖发生变化的块仍走正式范围查询。 */
  async adopt(previous: NativeMarkdownStructureIndex, signal: AbortSignal) {
    const response = await this.client.query<StructureChanges>({ kind: 'changes', observedGeneration: previous.client.identity.indexGeneration }, signal);
    const changes = response.items[0]; this.changes = changes ?? null;
    if (!changes || changes.invalidateAll || changes.fromGeneration !== previous.client.identity.indexGeneration) return;
    for (const [owner, part] of previous.parts) {
      const blocks: MarkdownStructureBlock[] = []; const headings: MarkdownOutlineItem[] = [];
      for (const block of part.blocks) {
        const mapping = changes.mappings.find(m => block.fromByte >= m.fromByte && block.toByte <= m.toByte);
        if (!mapping) continue;
        const fromByte = block.fromByte + mapping.byteDelta, toByte = block.toByte + mapping.byteDelta;
        if (changes.ranges.some(r => r.fromByte < toByte && r.toByte > fromByte) ||
          (changes.outlineChanged && block.headings.length)) continue;
        blocks.push({ ...block, version: this.revision, fromByte, toByte,
          fromLine: block.fromLine + mapping.lineDelta, toLine: block.toLine + mapping.lineDelta,
          headings: block.headings.map(h => ({ ...h, byte: h.byte + mapping.byteDelta, line: h.line + mapping.lineDelta })) });
        headings.push(...part.headings.filter(h => h.blockId === block.id).map(h => ({ ...h, byte: h.byte + mapping.byteDelta, line: h.line + mapping.lineDelta })));
      }
      const keys = new Set(blocks.flatMap(block => block.references));
      const references = Object.fromEntries(Object.entries(part.references).filter(([key]) => keys.has(key)));
      if (blocks.length) this.parts.set(owner, { blocks, headings, references });
    }
    // 转移租约后再计入新投影，避免将同一有界详情暂时计费两次。
    previous.parts.clear(); previous.client.releaseRetained(previous.detailLease);
    this.rebuild();
  }
  constructor(readonly client: NativeStructureClient, totalBytes: number) { super(client.identity.revision, totalBytes); }
  override get blocks() { return this.projected; }
  override get outline() { return this.localHeadings; }
  override get progress(): DocumentIndexProgress {
    const enhancements = emptyEnhancements(); let layoutUnits = 0;
    for (const region of this.regions) { addEnhancements(enhancements, region.enhancements); layoutUnits += region.layoutUnits; }
    return { revision: this.revision, state: this.status?.state === 'ready' ? 'ready' : this.regions.length ? 'partial' : 'building',
      indexedBytes: this.status?.indexedBytes ?? 0, totalBytes: this.totalBytes, layoutUnits,
      enhancements, enhancementObjects: enhancementTotal(enhancements), metricsComplete: this.status?.state === 'ready' && this.regions.every(r => r.metricsComplete) };
  }
  async refresh(signal: AbortSignal): Promise<boolean> {
    // 已发布前缀不可变；只重读末个聚合区（可能包含继续增长的源码块），避免每次进度轮询扫描全文。
    const regions: Geometry[] = this.regions.slice(0, -1);
    const fromByte = this.regions.at(-1)?.fromByte ?? 0;
    let cursor: StructureCursor | undefined;
    let response: StructureResponse<Geometry>;
    do {
      response = await this.client.query<Geometry>({ kind: 'geometry', fromByte, toByte: this.totalBytes,
        groupSize: Math.max(1, Math.ceil(this.totalBytes / 64 / MAX_REGIONS)), cursor }, signal);
      regions.push(...response.items);
      while (regions.length > MAX_REGIONS) {
        for (let i = 0; i + 1 < regions.length; i += 2) {
          const left = regions[i], right = regions[i + 1];
          const enhancements = { ...left.enhancements }; addEnhancements(enhancements, right.enhancements);
          regions.splice(i, 2, { ...left, toByte: right.toByte, blockCount: left.blockCount + right.blockCount,
            layoutUnits: left.layoutUnits + right.layoutUnits, enhancements, metricsComplete: left.metricsComplete && right.metricsComplete });
        }
      }
      cursor = response.items.length ? response.next ?? undefined : undefined;
      signal.throwIfAborted();
      // 构建期间优先发布已有首屏；不能沿持续增长的尾部追读到全篇结束才通知宿主。
      if (response.state !== 'ready') break;
    } while (cursor);
    const changed = this.status?.state !== response!.state || this.status?.indexedBytes !== response!.indexedBytes;
    // 文末定义与全局 slug 在完成时才稳定；旧的局部详情不能跨越这个发布点。
    if (this.status?.state !== 'ready' && response!.state === 'ready') {
      this.parts.clear(); this.detailGeneration++;
    }
    this.client.retain(this.geometryLease, regions, true);
    this.status = response!; this.regions = regions;
    if (changed || !this.projected.length) this.rebuild();
    return this.status.state === 'ready';
  }
  override blockAt(byte: number): MarkdownStructureBlock | undefined {
    let low = 0, high = this.projected.length;
    while (low < high) { const mid = (low + high) >>> 1; if (this.projected[mid].toByte <= byte) low = mid + 1; else high = mid; }
    return this.projected[low] ?? (byte === this.totalBytes ? this.projected.at(-1) : undefined);
  }
  async refine(owner: object, ranges: Array<{ fromByte: number; toByte: number }>, signal: AbortSignal): Promise<void> {
    const generation = this.detailGeneration;
    const blocks = new Map<string, MarkdownStructureBlock>();
    for (const range of ranges) {
      let rangeBlocks = 0;
      let cursor: StructureCursor | undefined;
      do {
        const page = await this.client.blocks(range.fromByte, range.toByte, cursor, signal);
        for (const block of page.items) {
          if (blocks.has(block.id)) continue;
          if (rangeBlocks >= MAX_RANGE_BLOCKS || blocks.size >= MAX_DETAIL_BLOCKS) break;
          blocks.set(block.id, block); rangeBlocks++;
        }
        cursor = page.next ?? undefined;
        if (!page.items.length || (cursor && cursor.ordinal >= range.toByte)) break;
        // 密集短标题不能按源码字节预取整篇；未取详情的区域继续用几何摘要表示。
        if (rangeBlocks >= MAX_RANGE_BLOCKS || blocks.size >= MAX_DETAIL_BLOCKS) break;
      } while (cursor);
      if (blocks.size >= MAX_DETAIL_BLOCKS) break;
    }
    const ordered = [...blocks.values()].sort((a, b) => a.fromByte - b.fromByte);
    const headings: StructureHeading[] = [];
    const detailRanges: Array<{fromByte: number; toByte: number}> = [];
    for (const block of ordered) {
      const previous = detailRanges.at(-1);
      if (previous && previous.toByte === block.fromByte) previous.toByte = block.toByte;
      else detailRanges.push({fromByte: block.fromByte, toByte: block.toByte});
    }
    for (const range of detailRanges) {
      let cursor: StructureCursor | undefined;
      do {
        const page = await this.client.query<StructureHeading>({ kind: 'headings', ...range, cursor }, signal);
        headings.push(...page.items); cursor = page.next ?? undefined;
      } while (cursor);
    }
    const keys = [...new Set(ordered.flatMap(block => block.references))];
    const references: Record<string, MarkdownReference> = Object.create(null);
    const refs = async (keys: string[]): Promise<void> => {
      try { for (const item of (await this.client.query<StructureReference>({ kind: 'references', keys }, signal)).items) references[item.key] = item.definition; }
      catch (error) {
        if ((error as { code?: string })?.code !== 'structure-page-limit' || keys.length < 2) throw error;
        const half = Math.ceil(keys.length / 2); await refs(keys.slice(0, half)); await refs(keys.slice(half));
      }
    };
    for (let i = 0; i < keys.length; i += 128) await refs(keys.slice(i, i + 128));
    const detail = { blocks: ordered, headings, references };
    if (JSON.stringify(detail).length * 2 > 2 * 1024 * 1024) throw new Error('可见结构详情超过 2 MiB，保留原文。');
    signal.throwIfAborted();
    if (generation !== this.detailGeneration) throw new DOMException('结构详情已被完整索引替代。', 'AbortError');
    const previous = this.parts.get(owner);
    this.parts.set(owner, detail);
    try { this.rebuild(); }
    catch (error) { if (previous) this.parts.set(owner, previous); else this.parts.delete(owner); throw error; }
  }
  release(owner: object) { if (this.parts.delete(owner)) this.rebuild(); }
  private rebuild() {
    const detail = new Map<string, MarkdownStructureBlock>(); const headings = new Map<string, MarkdownOutlineItem>();
    const references: Record<string, MarkdownReference> = Object.create(null);
    for (const part of this.parts.values()) {
      for (const block of part.blocks) detail.set(block.id, block);
      for (const heading of part.headings) headings.set(`${heading.blockId}:${heading.blockHeadingIndex}`, heading);
      Object.assign(references, part.references);
    }
    const ordered = [...detail.values()].sort((a, b) => a.fromByte - b.fromByte);
    const result: MarkdownStructureBlock[] = []; const added = new Set<string>(); let i = 0;
    for (const region of this.regions) {
      let from = region.fromByte;
      const coarse = (to: number) => {
        if (to <= from) return;
        result.push({ id: `region:${from}:${to}`, version: this.revision, fromByte: from, toByte: to,
          fromLine: 1, toLine: 1, kind: 'native_region', mode: 'semantic',
          layoutUnits: Math.max(1, region.layoutUnits * (to - from) / Math.max(1, region.toByte - region.fromByte)),
          enhancements: emptyEnhancements(), enhancementObjects: 0, metricsComplete: false, headings: [], references: [] });
      };
      while (i < ordered.length && ordered[i].toByte <= region.fromByte) i++;
      for (let j = i; j < ordered.length && ordered[j].fromByte < region.toByte; j++) {
        const block = ordered[j]; coarse(Math.min(region.toByte, block.fromByte));
        if (!added.has(block.id)) { result.push(block); added.add(block.id); }
        from = Math.max(from, block.toByte);
      }
      coarse(region.toByte);
    }
    this.client.retain(this.detailLease, { parts: [...this.parts.values()], projected: result, references });
    const outline = [...headings.values()].sort((a, b) => a.byte - b.byte);
    // 两个宿主可重复细化同一范围。事实未变时保持数组身份，避免彼此作废正在读取的计划。
    const sameBlocks = result.length === this.projected.length && result.every((block, i) =>
      block === this.projected[i] || JSON.stringify(block) === JSON.stringify(this.projected[i]));
    if (sameBlocks && JSON.stringify(outline) === JSON.stringify(this.localHeadings) &&
      JSON.stringify(references) === JSON.stringify(this.references)) return;
    this.projected = result; this.references = references;
    this.localHeadings = outline; this.geometryGeneration++;
  }
  dispose() { this.parts.clear(); this.regions = []; this.projected = []; this.localHeadings = []; this.references = Object.create(null);
    this.client.releaseRetained(this.detailLease); this.client.releaseRetained(this.geometryLease); this.client.dispose(); }
}
