import type { MarkdownSessionController } from './controller';
import type { MarkdownDocumentPort } from './port';
import { DocumentConflictError, type DocumentSelection } from './protocol';
import { SEARCH_PAGE_SIZE, type SearchIdentity, type SearchMatch, type SearchOptions,
  type SearchPage, type SearchProgress, type SearchQuery } from './searchProtocol';

/** 监听先于启动；按 requestId 接收早于启动响应的终态。任务对象不持有正文。 */
class SearchTask {
  identity?: SearchIdentity;
  private unlisten?: () => void;
  private disposed = false;
  private starting?: Promise<SearchIdentity>;
  private releasing?: Promise<void>;
  private committing = false;
  private settled = false;
  private resolve!: (progress: SearchProgress) => void;
  private reject!: (reason: unknown) => void;
  private readonly completion = new Promise<SearchProgress>((resolve, reject) => {
    this.resolve = resolve; this.reject = reject;
  });
  private readonly abort = () => { if (!this.committing) void this.dispose(); };
  constructor(private readonly port: MarkdownDocumentPort, readonly query: SearchQuery,
    private readonly options: SearchOptions) {
    // 启动失败/取消可先于调用者开始等待完成，避免短暂的未处理拒绝。
    void this.completion.catch(() => {});
  }
  private accept = (progress: SearchProgress) => {
    if (this.disposed || progress.requestId !== this.query.requestId ||
      progress.documentId !== this.query.documentId || progress.baseRevision !== this.query.baseRevision ||
      (this.identity && progress.taskId !== this.identity.taskId)) return;
    this.options.onProgress?.(progress);
    if (this.settled) return;
    if (progress.state === 'ready' || progress.state === 'prepared') {
      this.settled = true; this.resolve(progress);
    } else if (['cancelled', 'conflict', 'failed'].includes(progress.state)) {
      this.settled = true;
      this.reject(progress.state === 'cancelled' ? new DOMException('搜索任务已取消', 'AbortError') :
        Object.assign(new Error(progress.error?.message ?? '搜索任务失败'), { code: progress.error?.code }));
    }
  };
  async start(start: () => Promise<SearchIdentity>) {
    this.options.signal.throwIfAborted();
    this.unlisten = await this.port.listenSearch(this.accept);
    this.options.signal.addEventListener('abort', this.abort, { once: true });
    try {
      this.options.signal.throwIfAborted();
      if (this.disposed) throw new DOMException('搜索任务已取消', 'AbortError');
      this.starting = start();
      this.identity = await this.starting;
      if (this.disposed || this.options.signal.aborted) throw new DOMException('搜索任务已取消', 'AbortError');
      // 元数据补读覆盖丢失事件；没有正文和命中数组，也不持续轮询。
      this.accept(await this.port.searchTaskProgress(this.identity.taskId));
      return await this.completion;
    } catch (error) { await this.dispose(); throw error; }
  }
  beginCommit() { if (!this.committing) this.options.signal.throwIfAborted(); this.committing = true; }
  async dispose() {
    this.disposed = true;
    this.options.signal.removeEventListener('abort', this.abort);
    this.unlisten?.(); this.unlisten = undefined;
    if (!this.settled) { this.settled = true; this.reject(new DOMException('搜索任务已取消', 'AbortError')); }
    // 等待迟到的启动确认再释放，避免旧启动在新查询之后到达并撤销新任务。
    this.releasing ??= (async () => {
      const identity = this.identity ?? await this.starting?.catch(() => undefined);
      if (identity) await this.port.releaseTask(identity.taskId);
    })();
    await this.releasing;
  }
}

/** 只驻留当前页和相邻两页，异步页面即使迟到也不能扩展缓存。 */
export class MarkdownSearchResults {
  private readonly pages = new Map<number, SearchPage>();
  private readonly loading = new Map<number, Promise<SearchPage>>();
  private center = 0;
  private disposed = false;
  constructor(private readonly port: MarkdownDocumentPort, private readonly task: SearchTask,
    readonly totalCount: number, private readonly release: () => void) {}
  get snapshotRevision() { return this.task.query.baseRevision; }
  get residentMatches() { return [...this.pages.values()].reduce((sum, page) => sum + page.matches.length, 0); }
  private keep(start: number) { return Math.abs(start - this.center) <= SEARCH_PAGE_SIZE; }
  private async read(start: number): Promise<SearchPage> {
    if (this.disposed) throw new DOMException('搜索结果已释放', 'AbortError');
    const cached = this.pages.get(start); if (cached) return cached;
    const previous = this.loading.get(start); if (previous) return previous;
    if (this.loading.size >= 3) {
      await Promise.race(this.loading.values()).catch(() => {});
      if (!this.keep(start) || this.disposed) throw new DOMException('分页导航已更新', 'AbortError');
      return this.read(start);
    }
    const identity = this.task.identity!;
    const pending = this.port.readSearchPage(identity.taskId, start, SEARCH_PAGE_SIZE).then(page => {
      if (this.disposed) throw new DOMException('搜索结果已释放', 'AbortError');
      if (page.taskId !== identity.taskId || page.requestId !== identity.requestId ||
        page.documentId !== identity.documentId || page.baseRevision !== identity.baseRevision ||
        page.totalCount !== this.totalCount || page.fromIndex !== start || page.matches.length > SEARCH_PAGE_SIZE)
        throw new DocumentConflictError('搜索分页返回了过期结果。');
      if (this.keep(start)) this.pages.set(start, page);
      return page;
    }).finally(() => this.loading.delete(start));
    this.loading.set(start, pending); return pending;
  }
  async page(fromIndex: number): Promise<SearchPage> {
    this.center = Math.floor(Math.max(0, fromIndex) / SEARCH_PAGE_SIZE) * SEARCH_PAGE_SIZE;
    for (const key of this.pages.keys()) if (!this.keep(key)) this.pages.delete(key);
    const page = await this.read(this.center);
    for (const start of [this.center - SEARCH_PAGE_SIZE, this.center + SEARCH_PAGE_SIZE])
      if (start >= 0 && start < this.totalCount) void this.read(start).catch(() => {});
    return page;
  }
  async match(index: number): Promise<SearchMatch | undefined> {
    if (index < 0 || index >= this.totalCount) return undefined;
    return (await this.page(index)).matches.find(match => match.index === index);
  }
  async dispose() {
    if (this.disposed) return;
    this.disposed = true; this.pages.clear(); this.loading.clear(); this.release();
    await this.task.dispose();
  }
}

export class MarkdownSearchController {
  private generation = 0;
  private active = new Set<() => Promise<void>>();
  private releases = new Set<Promise<void>>();
  private preparations = new Map<AbortController, Promise<unknown>>();
  private readonly unsubscribe: () => void;
  constructor(private readonly session: MarkdownSessionController) {
    // base 尚在提交草稿/物化目录时，变化属于即将捕获的新版本。只撤销已启动任务；
    // 显式隐藏、关闭或后续请求仍通过 generation 撤销整个准备过程。
    this.unsubscribe = session.subscribeChanges(() => { void this.releaseTasks().catch(() => {}); });
  }
  private async base(options: SearchOptions) {
    const generation = ++this.generation;
    for (const controller of this.preparations.keys()) controller.abort();
    const abort = new AbortController();
    const cancel = () => abort.abort();
    options.signal.addEventListener('abort', cancel, { once: true });
    if (options.signal.aborted) cancel();
    const operation = this.session.withSnapshotTask(async () => {
      await this.releaseTasks();
      abort.signal.throwIfAborted();
      await this.session.flushEdits(); abort.signal.throwIfAborted();
      await this.session.beforeSnapshot?.(abort.signal); abort.signal.throwIfAborted();
      await this.session.awaitTransactions();
      if (generation !== this.generation || this.session.state.closing) throw new DOMException('搜索任务已取消', 'AbortError');
      return { revision: this.session.state.committed.revision, generation: this.session.state.viewGeneration, requestGeneration: generation };
    });
    this.preparations.set(abort, operation);
    try { return await operation; }
    finally { options.signal.removeEventListener('abort', cancel); this.preparations.delete(abort); }
  }
  private query(query: string, revision: number, options: SearchOptions): SearchQuery {
    return { documentId: this.session.documentId, baseRevision: revision, requestId: crypto.randomUUID(),
      query, caseSensitive: options.caseSensitive, wholeWord: options.wholeWord };
  }
  async search(query: string, options: SearchOptions) {
    const base = await this.base(options), request = this.query(query, base.revision, options);
    const task = new SearchTask(this.session.port, request, options);
    const dispose = () => task.dispose(); this.active.add(dispose);
    try {
      const progress = await task.start(() => this.session.port.startSearch(request));
      if (base.requestGeneration !== this.generation) throw new DOMException('搜索任务已取消', 'AbortError');
      if (base.generation !== this.session.state.viewGeneration) throw new DocumentConflictError();
      this.active.delete(dispose);
      const result = new MarkdownSearchResults(this.session.port, task, progress.matchCount, () => this.active.delete(release));
      const release = () => result.dispose(); this.active.add(release);
      return result;
    } catch (error) { this.active.delete(dispose); await task.dispose(); throw error; }
  }
  async replaceAll(query: string, replacement: string, options: SearchOptions,
    positions: () => { selection?: DocumentSelection; anchors?: number[] } = () => ({}),
    mapped?: (anchors: number[]) => void) {
    const base = await this.base(options), request = { ...this.query(query, base.revision, options),
      replacement, operationId: crypto.randomUUID(), ...positions() };
    const task = new SearchTask(this.session.port, request, options);
    const dispose = () => task.dispose(); this.active.add(dispose);
    let published = false;
    try {
      await task.start(() => this.session.port.prepareReplaceAll(request));
      const result = await this.session.commitNativeReplacement(base, options.signal, async () => {
        // 发布期间取消不能抢先把成功显示为取消；由同一 operationId 查询确定结果。
        task.beginCommit(); this.active.delete(dispose);
        return this.session.port.commitReplaceAll(request.documentId, task.identity!.taskId, request.operationId);
      }, result => mapped?.(result.anchors));
      published = true; return result;
    } finally {
      this.active.delete(dispose);
      // 释放失败不能把已经发布的正文报告成取消/替换失败；窗口销毁还有原生兜底。
      if (published) await task.dispose().catch(error => console.warn('已完成替换，释放搜索任务失败', error));
      else await task.dispose();
    }
  }
  async cancel() {
    this.generation++;
    const preparations = [...this.preparations];
    for (const [abort] of preparations) abort.abort();
    await this.releaseTasks();
    await Promise.allSettled(preparations.map(([, operation]) => operation));
  }
  private async releaseTasks() {
    const work = [...this.active]; this.active.clear();
    for (const release of work) {
      const pending = release(); this.releases.add(pending);
      void pending.finally(() => this.releases.delete(pending)).catch(() => {});
    }
    await Promise.all([...this.releases]);
  }
  async dispose() { this.unsubscribe(); await this.cancel(); }
}
