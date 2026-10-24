import { assertStructureIdentity, STRUCTURE_SYNTAX_VERSION, type StructureIdentity, type StructureQuery,
  type StructureQueryPort, type StructureResponse, type StructureCursor, type StructureBlock } from './structureProtocol';

const WINDOW_BYTES = 8 * 1024 * 1024;
const GEOMETRY_BYTES = 1024 * 1024;
const entries = new Map<string, { value: StructureResponse; bytes: number; geometry: boolean }>();
const retained = new Map<symbol, { documentId: string; bytes: number; geometry: boolean }>();
let usedBytes = 0;
let geometryBytes = 0;
function cost(value: unknown): number {
  if (typeof value === 'string') return 24 + value.length * 2;
  if (value === null || typeof value !== 'object') return 8;
  if (Array.isArray(value)) return 32 + value.reduce((total, item) => total + 8 + cost(item), 0);
  return 64 + Object.entries(value).reduce((total, [key, item]) => total + 16 + cost(key) + cost(item), 0);
}
function remove(key: string): void {
  const entry = entries.get(key);
  if (!entry) return;
  entries.delete(key); usedBytes -= entry.bytes;
  if (entry.geometry) geometryBytes -= entry.bytes;
}
function remember(key: string, value: StructureResponse, geometry: boolean): void {
  const bytes = cost(value) + cost(key);
  if (bytes > WINDOW_BYTES || (geometry && bytes > GEOMETRY_BYTES)) return;
  remove(key);
  while (usedBytes + bytes > WINDOW_BYTES || (geometry && geometryBytes + bytes > GEOMETRY_BYTES)) {
    let oldest: string | undefined;
    for (const [candidate, entry] of entries) {
      if (!geometry || geometryBytes + bytes <= GEOMETRY_BYTES || entry.geometry) { oldest = candidate; break; }
    }
    if (!oldest) return;
    remove(oldest);
  }
  entries.set(key, { value, bytes, geometry }); usedBytes += bytes;
  if (geometry) geometryBytes += bytes;
}
function abortable<T>(pending: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason ?? new DOMException('结构查询已取消。', 'AbortError'));
    if (signal.aborted) { abort(); void pending.catch(() => {}); return; }
    signal.addEventListener('abort', abort, { once: true });
    void pending.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}

/** 每个 WebView 共享有界纯数据缓存；不持有全文正文、组件回调或 DOM。 */
export class NativeStructureClient {
  private abort = new AbortController();
  private constructor(readonly port: StructureQueryPort, readonly identity: StructureIdentity) {}
  static async open(port: StructureQueryPort, documentId: string, revision: number, signal: AbortSignal): Promise<NativeStructureClient> {
    const identity = await abortable(port.startStructure(documentId, revision), signal);
    signal.throwIfAborted();
    if (identity.documentId !== documentId || identity.revision !== revision || identity.syntaxVersion !== STRUCTURE_SYNTAX_VERSION)
      throw new Error('原生结构索引返回了错误的文档身份。');
    return new NativeStructureClient(port, identity);
  }
  private key(query: StructureQuery): string {
    return `${this.identity.documentId}:${this.identity.indexGeneration}:${JSON.stringify(query)}`;
  }
  async query<T>(query: StructureQuery, signal: AbortSignal = this.abort.signal): Promise<StructureResponse<T>> {
    this.abort.signal.throwIfAborted(); signal.throwIfAborted();
    const key = this.key(query);
    const cached = entries.get(key);
    if (cached) { entries.delete(key); entries.set(key, cached); return cached.value as StructureResponse<T>; }
    const response = await abortable(abortable(this.port.queryStructure<T>(this.identity, query), signal), this.abort.signal);
    this.abort.signal.throwIfAborted(); signal.throwIfAborted();
    assertStructureIdentity(this.identity, response);
    if (response.state === 'failed') throw new Error(response.failure ?? '原生结构索引构建失败。');
    if (response.items.length > 256 || new TextEncoder().encode(JSON.stringify(response)).length > 256 * 1024)
      throw new Error('原生结构查询超过协议单页预算。');
    if (response.next && response.next.generation !== this.identity.indexGeneration)
      throw new Error('原生结构查询返回了过期游标。');
    if (response.state === 'ready' && !response.fragment &&
        ['blocks', 'geometry', 'references', 'outline', 'headings'].includes(query.kind))
      remember(key, response, query.kind === 'geometry');
    return response;
  }
  /** 普通页最多 256 项；仅超长单项续取，绝不自动下载余下全文元数据。 */
  async blocks(fromByte: number, toByte: number, cursor?: StructureCursor, signal?: AbortSignal): Promise<StructureResponse<StructureBlock>> {
    let response = await this.query<StructureBlock>({ kind: 'blocks', fromByte, toByte, cursor }, signal);
    if (response.fragment === null) return response;
    let combined = '';
    const ordinal = cursor?.ordinal ?? response.next?.ordinal;
    for (;;) {
      if (response.fragment === null) throw new Error('结构单项续取被中断。');
      combined += response.fragment;
      if (combined.length * 2 > WINDOW_BYTES) throw new Error('单项结构详情超过窗口预算，保留原文显示。');
      if (!response.next || response.next.offset === 0) break;
      if (ordinal !== undefined && response.next.ordinal !== ordinal) throw new Error('结构单项续取身份不连续。');
      response = await this.query<StructureBlock>({ kind: 'blocks', fromByte, toByte, cursor: response.next }, signal);
    }
    const block = JSON.parse(combined) as StructureBlock;
    if (!block || typeof block.id !== 'string' || block.version !== this.identity.revision || block.fromByte >= block.toByte)
      throw new Error('结构单项续取内容无效。');
    return { ...response, items: [block], fragment: null };
  }
  dispose(): void { this.abort.abort(); }
  /** 活跃投影与查询缓存共用预算；账本只保存令牌和大小，不持有视图或 DOM。 */
  retain(token: symbol, value: unknown, geometry = false): void {
    const bytes = cost(value);
    const previous = retained.get(token);
    const previousGeometry = previous?.geometry ? previous.bytes : 0;
    if (bytes > WINDOW_BYTES || (geometry && bytes > GEOMETRY_BYTES))
      throw new Error('结构投影超过窗口驻留预算。');
    while (usedBytes - (previous?.bytes ?? 0) + bytes > WINDOW_BYTES ||
      geometryBytes - previousGeometry + (geometry ? bytes : 0) > GEOMETRY_BYTES) {
      const oldest = [...entries].find(([, entry]) =>
        geometryBytes - previousGeometry + (geometry ? bytes : 0) <= GEOMETRY_BYTES || entry.geometry)?.[0];
      if (!oldest) throw new Error('结构投影超过窗口共享预算，请缩小可见范围。');
      remove(oldest);
    }
    this.releaseRetained(token);
    retained.set(token, { documentId: this.identity.documentId, bytes, geometry });
    usedBytes += bytes; if (geometry) geometryBytes += bytes;
  }
  releaseRetained(token: symbol): void {
    const previous = retained.get(token);
    if (!previous) return;
    retained.delete(token); usedBytes -= previous.bytes;
    if (previous.geometry) geometryBytes -= previous.bytes;
  }
  static releaseDocument(documentId: string): void {
    for (const key of entries.keys()) if (key.startsWith(`${documentId}:`)) remove(key);
    for (const [token, entry] of retained) if (entry.documentId === documentId) {
      retained.delete(token); usedBytes -= entry.bytes;
      if (entry.geometry) geometryBytes -= entry.bytes;
    }
  }
  static cacheUsage() { return { usedBytes, geometryBytes, capacityBytes: WINDOW_BYTES, geometryCapacityBytes: GEOMETRY_BYTES }; }
}
