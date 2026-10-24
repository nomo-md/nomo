import { NativeStructureClient } from './nativeStructureClient';
import { assertStructureIdentity, type StructureOutlineRow, type StructureOutlineView } from './structureProtocol';

/** 保存最后可读窗口；折叠树、全量标题和可见行投影由 Rust 管理。 */
export class NativeOutlineView {
  private viewId: string | null = null;
  private opening: Promise<void> | null = null;
  private disposed = false;
  private generation = 0;
  private request = 0;
  private epoch: number | undefined;
  private fromIndex = 0;
  private current: StructureOutlineView | null = null;
  private lease = Symbol('outline-window');
  private pending = new AbortController();
  failure: unknown = null;
  constructor(private client: NativeStructureClient, private level = 6) {}
  get snapshot(): StructureOutlineView | null { return this.current; }
  bind(client: NativeStructureClient): void {
    if (client.identity.documentId !== this.client.identity.documentId) throw new Error('大纲视图不能绑定到另一文档。');
    if (client.identity.indexGeneration === this.client.identity.indexGeneration) { this.client = client; return; }
    this.pending.abort(); this.pending = new AbortController(); this.client = client;
    this.epoch = undefined; this.generation++; this.failure = null;
  }
  private async open(): Promise<void> {
    if (this.viewId) return;
    if (this.opening) return this.opening;
    const client = this.client;
    // 创建句柄的 RPC 必须收回结果：即使面板同时关闭，也能释放迟到的句柄。
    this.opening = (async () => {
      const result = await client.port.queryStructure<{ viewId: string }>(client.identity, { kind: 'openOutline', defaultLevel: this.level });
      assertStructureIdentity(client.identity, result);
      const viewId = result.items[0]?.viewId;
      if (!viewId) throw new Error('原生大纲没有返回视图句柄。');
      this.viewId = viewId;
      if (this.disposed) await this.release();
    })().finally(() => { this.opening = null; });
    return this.opening;
  }
  async load(fromIndex = this.fromIndex): Promise<StructureOutlineView | null> {
    if (this.disposed) return this.current;
    const generation = this.generation;
    const request = ++this.request;
    const signal = this.pending.signal;
    try {
      try { await this.open(); }
      catch (error) {
        // 较早修订的创建可能在后端二次校验时过期；新修订的请求重新创建一次。
        if (generation !== this.generation || signal.aborted) return this.current;
        if ((error as { code?: string })?.code !== 'revision-conflict') throw error;
        await this.open();
      }
      signal.throwIfAborted();
      const response = await this.client.query<StructureOutlineView>({ kind: 'outlineView', viewId: this.viewId!, fromIndex: Math.max(0, fromIndex), viewGeneration: this.epoch }, signal);
      if (this.disposed || generation !== this.generation || request !== this.request) return this.current;
      const view = response.items[0];
      if (!view || view.viewId !== this.viewId) throw new Error('原生大纲返回了另一视图。');
      this.epoch = view.viewGeneration; this.fromIndex = fromIndex; this.failure = null;
      // 新修订还没有可见行时保留上一窗口，首次加载才显示空加载态。
      if (view.rows.length || view.projectionComplete || !this.current) {
        this.client.retain(this.lease, view); this.current = view;
      }
      return this.current;
    } catch (error) {
      if (signal.aborted || this.disposed || generation !== this.generation || request !== this.request) return this.current;
      if (this.epoch !== undefined && (error as { code?: string })?.code === 'outline-cursor-expired') {
        this.epoch = undefined; return this.load(fromIndex);
      }
      this.failure = error; throw error;
    }
  }
  async toggle(row: StructureOutlineRow): Promise<StructureOutlineView | null> {
    if (this.disposed || !this.viewId || !row.hasChildren) return this.current;
    await this.client.query({ kind: 'setOutlineFold', viewId: this.viewId, headingId: `${row.blockId}:${row.blockHeadingIndex}`, collapsed: !row.collapsed }, this.pending.signal);
    this.epoch = undefined; return this.load();
  }
  async locate(headingId: string, byte: number, parent = false): Promise<number> {
    const generation = this.generation, signal = this.pending.signal;
    await this.open();
    for (;;) {
      signal.throwIfAborted();
      const response = await this.client.query<{position: number | null; complete: boolean}>({
        kind: 'locateOutline', viewId: this.viewId!, headingId, byte, parent,
      }, signal);
      if (generation !== this.generation || this.disposed) throw new DOMException('大纲定位已过期。', 'AbortError');
      const result = response.items[0];
      if (!result) throw new Error('大纲定位没有返回状态。');
      if (result.complete) return result.position ?? 0;
      await new Promise(resolve => setTimeout(resolve, 16));
    }
  }
  async setLevel(level: number): Promise<StructureOutlineView | null> {
    this.level = level;
    if (!this.viewId) return this.current;
    await this.client.query({ kind: 'setOutlineLevel', viewId: this.viewId, level }, this.pending.signal);
    this.epoch = undefined; return this.load();
  }
  private async release(): Promise<void> {
    const id = this.viewId;
    if (!id) return;
    if (!this.client.port.closeStructureOutline) throw new Error('原生端口缺少大纲句柄释放能力。');
    try { await this.client.port.closeStructureOutline(this.client.identity.documentId, id); }
    catch (error) {
      // 关闭和移交已在原生释放文档全部句柄；其它失败保留身份以便重试。
      if (!['document-closed', 'document-owner-conflict'].includes((error as { code?: string })?.code ?? '')) throw error;
    }
    if (this.viewId === id) this.viewId = null;
  }
  async dispose(): Promise<void> {
    this.disposed = true; this.pending.abort(); this.generation++;
    await this.opening?.catch(() => undefined);
    await this.release(); this.current = null; this.client.releaseRetained(this.lease);
  }
}
