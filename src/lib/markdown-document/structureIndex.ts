import StructureWorker from './structure.worker?worker&inline';
import type { StructureWorkerRequest } from './structure.worker';
import type {
  MarkdownStructureBatch,
  MarkdownStructureBlock,
  MarkdownReference,
  MarkdownStructureScanner,
} from './structure';
import type { MarkdownDocumentPort, SnapshotReference } from './port';
import type { DocumentIndexProgress, SourceEdit } from './protocol';
import { utf8Length } from './utf8Text';
import { createHeadingIdAllocator, normalizeHeadingTitle } from '../toc/tocService';
import type { OutlineItem } from '../outline/outlineService';
import { emptyEnhancements, addEnhancements, enhancementTotal } from './enhancements';

export interface MarkdownOutlineItem extends OutlineItem {
  byte: number;
  blockId: string;
  blockHeadingIndex: number;
}

type Request = StructureWorkerRequest extends infer T
  ? T extends { id: number }
    ? Omit<T, 'id'>
    : never
  : never;
let worker: Worker | null = null;
let sequence = 0;
let clients = 0;
const requests = new Map<
  number,
  { resolve(value: MarkdownStructureBatch): void; reject(error: unknown): void }
>();

function acquireWorker() {
  if (worker) {
    clients++;
    return;
  }
  const created = new StructureWorker();
  worker = created;
  clients++;
  worker.onmessage = (
    event: MessageEvent<{ id: number; error?: string; result: MarkdownStructureBatch }>,
  ) => {
    const request = requests.get(event.data.id);
    if (!request) return;
    requests.delete(event.data.id);
    if (event.data.error) request.reject(new Error(event.data.error));
    else request.resolve(event.data.result);
  };
  worker.onerror = (event) => {
    for (const request of requests.values())
      request.reject(new Error(event.message || '结构解析 Worker 意外退出。'));
    requests.clear();
    worker?.terminate();
    worker = null;
  };
}
function releaseWorker() {
  clients--;
  if (!clients) {
    worker?.terminate();
    worker = null;
    for (const request of requests.values())
      request.reject(new DOMException('结构任务已结束。', 'AbortError'));
    requests.clear();
  }
}
function call(request: Request): Promise<MarkdownStructureBatch> {
  return new Promise((resolve, reject) => {
    if (!worker) {
      reject(new Error('结构解析 Worker 不可用。'));
      return;
    }
    const id = ++sequence;
    requests.set(id, { resolve, reject });
    try {
      worker.postMessage({ ...request, id });
    } catch (error) {
      requests.delete(id);
      reject(error);
    }
  });
}

/** 每个应用窗口按需共用一个纯解析 Worker；视图不持有 Worker，最后一个任务结束即释放。 */
export async function scanMarkdownSnapshot(
  port: MarkdownDocumentPort,
  snapshot: SnapshotReference,
  options: {
    signal: AbortSignal;
    onBatch(batch: MarkdownStructureBatch): void | boolean | Promise<void | boolean>;
    checkpoint?: ConstructorParameters<typeof MarkdownStructureScanner>[1];
    shouldYield?: () => Promise<void>;
  },
) {
  options.signal.throwIfAborted();
  const job = globalThis.crypto.randomUUID();
  acquireWorker();
  const abortable = <T>(task: Promise<T>) =>
    new Promise<T>((resolve, reject) => {
      const aborted = () =>
        reject(options.signal.reason ?? new DOMException('结构任务已取消。', 'AbortError'));
      if (options.signal.aborted) {
        aborted();
        void task.catch(() => {});
        return;
      }
      options.signal.addEventListener('abort', aborted, { once: true });
      void task
        .then(resolve, reject)
        .finally(() => options.signal.removeEventListener('abort', aborted));
    });
  try {
    await abortable(
      call({ type: 'start', job, revision: snapshot.revision, options: options.checkpoint }),
    );
    let from = options.checkpoint?.fromByte ?? 0;
    if (from === snapshot.byteLength)
      await options.onBatch(
        await abortable(call({ type: 'chunk', job, text: '', complete: true })),
      );
    while (from < snapshot.byteLength) {
      options.signal.throwIfAborted();
      const window = await abortable(
        port.readSnapshot(snapshot, from, Math.min(snapshot.byteLength, from + 16 * 1024)),
      );
      options.signal.throwIfAborted();
      if (
        window.revision !== snapshot.revision ||
        window.fromByte !== from ||
        window.toByte <= from
      )
        throw new Error('结构索引读取到不连续的文档快照。');
      from = window.toByte;
      const batch = await abortable(
        call({ type: 'chunk', job, text: window.text, complete: from === snapshot.byteLength }),
      );
      options.signal.throwIfAborted();
      if ((await options.onBatch(batch)) === false) return;
      await abortable(
        options.shouldYield?.() ?? new Promise<void>((resolve) => setTimeout(resolve, 0)),
      );
    }
  } finally {
    // 取消不等待 Worker 处理旧批次；其它文档继续工作，迟到结果由 RPC 身份丢弃。
    if (worker) void call({ type: 'cancel', job }).catch(() => {});
    releaseWorker();
  }
}

/** 结构元数据不持有原文、PM 节点或 DOM。旧修订发布必须由调用方拒绝。 */
export class MarkdownStructureIndex {
  private entries: MarkdownStructureBlock[] = [];
  references: Record<string, MarkdownReference> = Object.create(null);
  private indexedBytes = 0;
  private complete = false;
  private layoutUnits = 0;
  private enhancements = emptyEnhancements();
  private incompleteMetrics = 0;
  private headings: MarkdownOutlineItem[] = [];
  private allocateHeadingId = createHeadingIdAllocator();
  constructor(
    readonly revision: number,
    readonly totalBytes: number,
  ) {}
  get blocks(): readonly MarkdownStructureBlock[] {
    return this.entries;
  }
  /** 结果与 progress 使用同一版本；partial 时只代表已确认结构，不能当全文目录写回。 */
  get outline(): readonly MarkdownOutlineItem[] {
    return this.headings;
  }
  private appendHeadings(block: MarkdownStructureBlock) {
    for (const [blockHeadingIndex, heading] of block.headings.entries()) {
      const title = normalizeHeadingTitle(heading.text) || heading.text.trim();
      this.headings.push({
        id: this.allocateHeadingId(title, this.headings.length + 1),
        level: heading.level as OutlineItem['level'],
        title,
        line: heading.line,
        byte: heading.byte,
        blockId: block.id,
        blockHeadingIndex,
      });
    }
  }
  get progress(): DocumentIndexProgress {
    return {
      revision: this.revision,
      state: this.complete ? 'ready' : this.indexedBytes ? 'partial' : 'building',
      indexedBytes: this.indexedBytes,
      totalBytes: this.totalBytes,
      layoutUnits: this.layoutUnits,
      enhancementObjects: enhancementTotal(this.enhancements),
      enhancements: { ...this.enhancements },
      metricsComplete: this.complete && this.incompleteMetrics === 0,
    };
  }
  publish(batch: MarkdownStructureBatch) {
    let rebuildHeadings = false;
    for (const block of batch.blocks) {
      if (block.version !== this.revision) throw new Error('不能发布另一修订的结构索引。');
      const last = this.entries.at(-1);
      if (last?.id === block.id) {
        this.layoutUnits -= last.layoutUnits;
        addEnhancements(this.enhancements, last.enhancements, -1);
        this.incompleteMetrics -= Number(!last.metricsComplete);
        this.entries[this.entries.length - 1] = block;
        // 巨型原文块可延长，但不重复分配标题 ID。真正替换了标题才重建派生目录。
        if (last.headings.length || block.headings.length) rebuildHeadings = true;
      } else {
        if (block.fromByte !== (last?.toByte ?? 0)) throw new Error('结构索引出现空洞或重叠。');
        this.entries.push(block);
        if (!rebuildHeadings) this.appendHeadings(block);
      }
      this.layoutUnits += block.layoutUnits;
      addEnhancements(this.enhancements, block.enhancements);
      this.incompleteMetrics += Number(!block.metricsComplete);
    }
    if (rebuildHeadings) {
      this.headings = [];
      this.allocateHeadingId = createHeadingIdAllocator();
      for (const block of this.entries) this.appendHeadings(block);
    }
    this.references = batch.references;
    this.indexedBytes = batch.consumedBytes;
    this.complete = batch.complete;
  }
  /** 仅在安全结构起点之前继承元数据，正文始终属于独立不可变快照。 */
  forkPrefix(revision: number, totalBytes: number, beforeByte: number) {
    const next = new MarkdownStructureIndex(revision, totalBytes);
    const blocks = this.entries
      .filter((block) => block.toByte <= beforeByte)
      .map((block) => ({ ...block, version: revision }));
    const references: Record<string, MarkdownReference> = Object.create(null);
    for (const block of blocks)
      for (const [key, value] of Object.entries(block.definitions ?? {})) references[key] ??= value;
    next.publish({
      blocks,
      references,
      consumedBytes: blocks.at(-1)?.toByte ?? 0,
      complete: false,
    });
    return next;
  }
  /** 调用方已核对收敛块的实际源文；从该块之后复用结构，不复用旧 DOM 或 PM 节点。 */
  appendUnchangedTail(
    previous: MarkdownStructureIndex,
    afterByte: number,
    byteDelta: number,
    lineDelta: number,
  ) {
    if (previous.progress.state !== 'ready') throw new Error('尚未完整的旧索引不能作为收敛后缀。');
    const blocks = previous.blocks
      .filter((block) => block.fromByte >= afterByte)
      .map((block) => ({
        ...block,
        version: this.revision,
        fromByte: block.fromByte + byteDelta,
        toByte: block.toByte + byteDelta,
        fromLine: block.fromLine + lineDelta,
        toLine: block.toLine + lineDelta,
        headings: block.headings.map((heading) => ({
          ...heading,
          byte: heading.byte + byteDelta,
          line: heading.line + lineDelta,
        })),
      }));
    const references: Record<string, MarkdownReference> = Object.assign(
      Object.create(null),
      this.references,
    );
    for (const block of blocks)
      for (const [key, value] of Object.entries(block.definitions ?? {})) references[key] ??= value;
    this.publish({ blocks, references, consumedBytes: this.totalBytes, complete: true });
  }
  blockAt(byte: number): MarkdownStructureBlock | undefined {
    let from = 0;
    let to = this.entries.length;
    while (from < to) {
      const middle = (from + to) >>> 1;
      if (this.entries[middle].toByte <= byte) from = middle + 1;
      else to = middle;
    }
    return this.entries[from] ?? (byte === this.totalBytes ? this.entries.at(-1) : undefined);
  }
  /** 重扫从变化前的安全结构起点开始；返回旧块供后续版本/状态一致性比较。 */
  checkpointFor(edits: readonly SourceEdit[]) {
    const first = edits.reduce((from, edit) => Math.min(from, edit.fromByte), this.totalBytes);
    const block = this.blockAt(Math.max(0, first - 1));
    const shift = edits.reduce(
      (sum, edit) => sum + utf8Length(edit.insertedText) - (edit.toByte - edit.fromByte),
      0,
    );
    return { fromByte: block?.fromByte ?? 0, fromLine: block?.fromLine ?? 1, shift };
  }
}
