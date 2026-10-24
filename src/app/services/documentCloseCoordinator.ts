export type DocumentOperationKind = 'save' | 'export' | 'copy' | 'edit';
export interface DocumentSaveResult {
  status: 'saved' | 'cancelled' | 'failed';
  revision?: number;
  path?: string;
  error?: string;
}
export interface DocumentOperation {
  readonly tabId: string;
  readonly kind: DocumentOperationKind;
  phase: 'queued' | 'running' | 'snapshot' | 'output' | 'finished';
  revision?: number;
  targetPath?: string;
  sourceRequired: boolean;
  saveResult?: DocumentSaveResult;
  promise: Promise<unknown>;
  snapshot(revision: number): void;
  releaseSource(): void;
}

/** 操作先登记再执行；输出任务可在源标签消失后独立持有快照。 */
export class DocumentOperationRegistry {
  private readonly operations = new Set<DocumentOperation>();
  private readonly observers = new Set<{ ids: Set<string>; operations: Set<DocumentOperation> }>();
  private readonly blocked = new Set<string>();
  private readonly listeners = new Set<() => void>();
  private readonly saveQueues = new Map<string, Promise<unknown>>();
  private changed() { for (const listener of [...this.listeners]) listener(); }
  block(ids: readonly string[], blocked: boolean) {
    for (const id of ids) if (blocked) this.blocked.add(id); else this.blocked.delete(id);
  }
  isBlocked(id: string) { return this.blocked.has(id); }
  run<T>(tabId: string, kind: DocumentOperationKind, run: (operation: DocumentOperation) => Promise<T>, allowClosing = false): Promise<T> {
    if (this.blocked.has(tabId) && !allowClosing) return Promise.reject(new Error('文档正在关闭。'));
    const previous = kind === 'save' ? this.saveQueues.get(tabId) : undefined;
    const operation: DocumentOperation = {
      tabId, kind, phase: 'queued', sourceRequired: true, promise: Promise.resolve(),
      snapshot: revision => { operation.revision = revision; operation.phase = 'snapshot'; this.changed(); },
      releaseSource: () => { operation.sourceRequired = false; operation.phase = 'output'; this.changed(); },
    };
    // 微任务开始前，关闭协调器即可看见完整操作归属。
    const pending = Promise.resolve(previous).catch(() => undefined).then(() => {
      operation.phase = 'running';
      return run(operation);
    });
    operation.promise = pending;
    this.operations.add(operation);
    for (const observer of this.observers) if (observer.ids.has(tabId)) observer.operations.add(operation);
    if (kind === 'save') this.saveQueues.set(tabId, pending);
    const finish = () => {
      operation.phase = 'finished'; operation.sourceRequired = false;
      this.operations.delete(operation);
      if (this.saveQueues.get(tabId) === pending) this.saveQueues.delete(tabId);
      this.changed();
    };
    void pending.then(finish, finish);
    return pending;
  }
  coversRevision(id: string, revision: number) {
    return [...this.operations].some(operation => operation.tabId === id && operation.kind === 'save' && operation.revision === revision);
  }
  savingPaths(id: string) { return [...this.operations].filter(operation => operation.tabId === id && operation.kind === 'save' && operation.targetPath).map(operation => operation.targetPath!); }
  async awaitSaveCapture(ids: readonly string[]) {
    while ([...this.operations].some(operation => ids.includes(operation.tabId) && operation.kind === 'save' && operation.revision === undefined))
      await new Promise<void>(resolve => {
        const changed = () => { this.listeners.delete(changed); resolve(); }; this.listeners.add(changed);
      });
  }
  observe(ids: readonly string[]) {
    const observer = { ids: new Set(ids), operations: new Set([...this.operations].filter(operation => ids.includes(operation.tabId))) };
    this.observers.add(observer);
    return { operations: observer.operations, dispose: () => this.observers.delete(observer) };
  }
  async awaitSource(ids: readonly string[], kinds?: readonly DocumentOperationKind[]) {
    for (;;) {
      const pending = [...this.operations].filter(operation => ids.includes(operation.tabId) && operation.sourceRequired && (!kinds || kinds.includes(operation.kind)));
      if (!pending.length) return;
      await new Promise<void>(resolve => {
        const changed = () => { this.listeners.delete(changed); resolve(); };
        this.listeners.add(changed);
      });
    }
  }
  async awaitAll() {
    while (this.operations.size) await Promise.allSettled([...this.operations].map(operation => operation.promise));
  }
}

export type DocumentCloseStage = 'preparing' | 'detached' | 'finalizing' | 'closed';
export interface DocumentCloseHooks {
  prepare(): Promise<void>;
  decide(): Promise<boolean>;
  persist(phase: 'pending' | 'readyToRelease' | 'removed'): Promise<void>;
  detach(): void;
  releaseViews(): Promise<void>;
  validate(): void;
  isDiscarding?(tabId: string): boolean;
  release(): Promise<void>;
  restore(error?: unknown): Promise<void>;
  resume(): void;
  report(error: unknown): void;
  mark?(stage: string): void;
}
interface CloseTransaction {
  ids: readonly string[];
  stage: DocumentCloseStage;
  cancelled: boolean;
  removed: Promise<boolean>;
  finished: Promise<void>;
  restore: (error?: unknown) => Promise<void>;
}

/** 标签可见性与文档所有权分开；保护任务结束前绝不销毁原生正文。 */
export class DocumentCloseCoordinator {
  private readonly transactions = new Map<string, CloseTransaction>();
  constructor(readonly operations = new DocumentOperationRegistry()) {}
  isClosing(id: string) { return this.transactions.has(id); }
  get pending() { return new Set(this.transactions.values()).size; }
  request(ids: readonly string[], hooks: DocumentCloseHooks): Promise<boolean> {
    const existing = ids.map(id => this.transactions.get(id)).find(Boolean);
    if (existing) return existing.removed;
    const watched = this.operations.observe(ids);
    const transaction: CloseTransaction = {
      ids, stage: 'preparing', cancelled: false, removed: Promise.resolve(false), finished: Promise.resolve(), restore: async () => {},
    };
    for (const id of ids) this.transactions.set(id, transaction);
    const forget = () => {
      watched.dispose(); this.operations.block(ids, false);
      for (const id of ids) if (this.transactions.get(id) === transaction) this.transactions.delete(id);
    };
    let restoring: Promise<void> | undefined;
    transaction.restore = error => restoring ??= (async () => {
      transaction.cancelled = true;
      await hooks.restore(error);
      // 先恢复可见引用再删除关闭记录；持久化失败仍保留恢复入口。
      await hooks.persist('removed');
      hooks.resume(); forget();
    })();
    transaction.removed = (async () => {
      try {
        hooks.mark?.('requested');
        this.operations.block(ids, true);
        await hooks.prepare(); hooks.mark?.('committed');
        if (!await hooks.decide()) { hooks.resume(); forget(); return false; }
        hooks.mark?.('confirmed');
        await hooks.persist('pending');
        hooks.detach(); transaction.stage = 'detached'; hooks.mark?.('detached');
        const views = hooks.releaseViews();
        transaction.finished = (async () => {
          try {
            await this.operations.awaitSource(ids);
            const saves = [...watched.operations].filter(operation => operation.kind === 'save');
            const results = await Promise.allSettled(saves.map(operation => operation.promise));
            if (transaction.cancelled) return;
            const failed = saves.find((operation, index) => !hooks.isDiscarding?.(operation.tabId) &&
              (operation.saveResult?.status === 'failed' || operation.saveResult?.status === 'cancelled' || results[index].status === 'rejected' || (results[index].status === 'fulfilled' && results[index].value === false)));
            if (failed) throw new Error(failed.saveResult?.error ?? '保存未完成，已恢复关闭的文档。');
            hooks.mark?.('operationsFinished');
            await views; hooks.mark?.('viewsReleased');
            if (transaction.cancelled) return;
            hooks.validate();
            await hooks.persist('readyToRelease');
            if (transaction.cancelled) return;
            transaction.stage = 'finalizing';
            await hooks.release();
            transaction.stage = 'closed'; hooks.mark?.('released');
            await hooks.persist('removed'); forget();
          } catch (error) {
            if (transaction.stage === 'closed') { hooks.report(error); forget(); return; }
            try { await views; await transaction.restore(error); }
            catch (restoreError) { hooks.report(restoreError); }
          }
        })();
        return true;
      } catch (error) {
        hooks.resume(); forget(); hooks.report(error); return false;
      }
    })();
    return transaction.removed;
  }
  async reopen(id: string) {
    const transaction = this.transactions.get(id);
    if (!transaction) return false;
    await transaction.removed;
    if (this.transactions.get(id) !== transaction) return false;
    if (transaction.stage === 'finalizing' || transaction.stage === 'closed') { await transaction.finished; return false; }
    await transaction.restore(); return true;
  }
  async awaitSettled() {
    while (this.transactions.size) {
      const transactions = [...new Set(this.transactions.values())];
      await Promise.all(transactions.map(transaction => transaction.removed));
      await Promise.all(transactions.map(transaction => transaction.finished));
      if (transactions.some(transaction => this.transactions.get(transaction.ids[0]) === transaction))
        throw new Error('待关闭文档仍有未解决的恢复错误。');
    }
  }
}
