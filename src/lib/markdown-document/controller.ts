import type { DocumentSelection, DocumentTransaction, SourceEdit } from './protocol';
import { DocumentConflictError } from './protocol';
import type { MarkdownDocumentPort, MarkdownSessionStatus, SnapshotReference } from './port';
import type { ReplaceResult } from './searchProtocol';

export interface MarkdownControllerState {
  committed: MarkdownSessionStatus;
  /** 视图世代包含尚未确认的输入，不能冒充原生修订号。 */
  viewGeneration: number;
  pendingTransactions: number;
  failure: unknown | null;
  historyBusy: boolean;
  nativeCommitBusy: boolean;
  closing: boolean;
}
export interface MarkdownEditInput {
  derived?: boolean;
  viewGeneration: number;
  edits: readonly SourceEdit[];
  historyGroupId: string;
  selectionBefore?: DocumentSelection;
  selectionAfter?: DocumentSelection;
}
export interface MarkdownControllerChange {
  derived?: boolean;
  generation: number;
  /** 历史恢复没有局部补丁时，视图必须重新读取，不得猜测位置。 */
  edits?: readonly SourceEdit[];
}
interface Pending {
  input: MarkdownEditInput;
  operationId: string;
  transaction?: DocumentTransaction;
  resolve: (status: MarkdownSessionStatus) => void;
  reject: (error: unknown) => void;
}
export interface MountedMarkdownEdits {
  awaitCompositionEnd(): Promise<void>;
  commitPendingEdits(): void | Promise<void>;
  hasUnconfirmedDraft?(): boolean;
  hasUnsettledEdits?(): boolean;
}

/**
 * 前端会话协调器只持有元数据及尚未确认的补丁。PM/CM 可乐观显示输入；确认失败时
 * 补丁和视图均保留，保存/移交必须等待同一队列成功，不能退回旧正文后报告成功。
 */
export class MarkdownSessionController {
  private committed: MarkdownSessionStatus;
  private generation = 0;
  private sequence = 0;
  private readonly clientId = globalThis.crypto.randomUUID();
  private pending: Pending[] = [];
  private running: Promise<void> | null = null;
  private failure: unknown | null = null;
  private closed = false;
  private historyBusy = false;
  private nativeCommitBusy = false;
  private nativeCommit: Promise<ReplaceResult> | null = null;
  private nativeRecovery: (() => Promise<ReplaceResult>) | null = null;
  private closing = false;
  private mounted = new Set<MountedMarkdownEdits>();
  private listeners = new Set<(state: MarkdownControllerState) => void>();
  private changeListeners = new Set<(change: MarkdownControllerChange) => void>();
  private snapshotTasks = new Set<Promise<unknown>>();
  // 保存、搜索准备和索引共用释放确认。失败句柄不能丢失，也不再继续积累新快照。
  private snapshotReleases = new Map<SnapshotReference, Promise<void> | null>();
  beforeSnapshot?: (signal?: AbortSignal) => Promise<void>;

  constructor(
    readonly port: MarkdownDocumentPort,
    status: MarkdownSessionStatus,
  ) {
    this.committed = status;
  }
  get documentId() {
    return this.committed.documentId;
  }
  get state(): MarkdownControllerState {
    return {
      committed: this.committed,
      viewGeneration: this.generation,
      pendingTransactions: this.pending.length,
      failure: this.failure,
      historyBusy: this.historyBusy,
      nativeCommitBusy: this.nativeCommitBusy,
      closing: this.closing,
    };
  }
  private assertOpen() {
    if (this.closed) throw new Error('文档会话已关闭。');
  }
  private emit() {
    for (const listener of this.listeners) {
      try {
        listener(this.state);
      } catch (error) {
        queueMicrotask(() => {
          throw error;
        });
      }
    }
  }
  subscribe(listener: (state: MarkdownControllerState) => void) {
    this.assertOpen();
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }
  subscribeChanges(listener: (change: MarkdownControllerChange) => void) {
    this.assertOpen();
    this.changeListeners.add(listener);
    return () => this.changeListeners.delete(listener);
  }
  private changed(edits?: readonly SourceEdit[], derived = false) {
    for (const listener of this.changeListeners) {
      try {
        listener({ generation: this.generation, edits, derived });
      } catch (error) {
        queueMicrotask(() => {
          throw error;
        });
      }
    }
  }
  attachEdits(view: MountedMarkdownEdits) {
    this.assertOpen();
    this.mounted.add(view);
    return () => this.mounted.delete(view);
  }
  /** 派生目录不能穿过尚未进入会话队列的输入或中文组字。 */
  get hasUnsettledEdits() {
    return [...this.mounted].some(view => view.hasUnsettledEdits?.());
  }

  applyEdits(input: MarkdownEditInput): Promise<MarkdownSessionStatus> {
    this.assertOpen();
    if (this.historyBusy || this.nativeCommitBusy || this.closing)
      return Promise.reject(new Error('正在切换文档状态，请稍后提交输入。'));
    if (input.viewGeneration !== this.generation)
      return Promise.reject(new DocumentConflictError());
    if (!input.edits.length) return Promise.resolve(this.committed);
    const edits = input.edits.map((edit) => ({ ...edit }));
    this.generation++;
    const promise = new Promise<MarkdownSessionStatus>((resolve, reject) =>
      this.pending.push({
        input: {
          ...input,
          edits,
          selectionBefore: input.selectionBefore && { ...input.selectionBefore },
          selectionAfter: input.selectionAfter && { ...input.selectionAfter },
        },
        operationId: `${this.clientId}:${++this.sequence}`,
        resolve,
        reject,
      }),
    );
    this.changed(edits, input.derived);
    this.emit();
    this.startDrain();
    return promise;
  }

  private startDrain() {
    if (this.running || this.failure || !this.pending.length) return;
    this.running = this.drain().finally(() => {
      this.running = null;
      if (!this.failure && this.pending.length) this.startDrain();
    });
  }
  private async drain() {
    while (this.pending.length && !this.failure) {
      const first = this.pending[0];
      // 首次发送才绑定原生修订；重试保留相同 operationId、baseRevision 和内容。
      first.transaction ??= {
        derived: first.input.derived,
        baseRevision: this.committed.revision,
        operationId: first.operationId,
        historyGroupId: first.input.historyGroupId,
        edits: first.input.edits,
        selectionBefore: first.input.selectionBefore,
        selectionAfter: first.input.selectionAfter,
      };
      try {
        const status = await this.port.applyTransaction(this.documentId, first.transaction);
        if (status.documentId !== this.documentId || status.revision < this.committed.revision)
          throw new DocumentConflictError('会话返回了过期确认。');
        this.committed = status;
        this.pending.shift();
        first.resolve(status);
        this.emit();
      } catch (error) {
        this.failure = error;
        first.reject(error);
        this.emit();
      }
    }
  }
  async retryPendingEdits() {
    this.assertOpen();
    await this.running;
    if (this.nativeRecovery) {
      try {
        const result = await this.nativeRecovery();
        this.acceptNativeReplacement(result); this.nativeRecovery = null;
      } catch (error) {
        if ((error as { code?: string })?.code === 'replacement-not-committed') {
          this.nativeRecovery = null; this.failure = null; this.emit();
        }
        throw error;
      }
    }
    this.failure = null;
    this.startDrain();
    await this.flushQueue();
  }
  private async flushQueue() {
    while (this.running || this.pending.length) {
      if (this.failure) throw this.failure;
      this.startDrain();
      await this.running;
    }
    if (this.failure) throw this.failure;
  }
  /** 视口和索引等待已提交事务，不能替用户确认 NodeView 草稿或结束组字。 */
  async awaitTransactions() {
    this.assertOpen();
    await this.flushQueue();
  }
  /** Statistics may wait for composition/transactions, but never commit a view draft. */
  async awaitStatisticsReady(signal: AbortSignal): Promise<boolean> {
    this.assertOpen();
    for (const view of this.mounted) {
      if (signal.aborted || view.hasUnconfirmedDraft?.()) return false;
      const ready = await new Promise<boolean>((resolve, reject) => {
        const cancelled = () => { signal.removeEventListener('abort', cancelled); resolve(false); };
        signal.addEventListener('abort', cancelled, { once: true });
        view.awaitCompositionEnd().then(() => {
          signal.removeEventListener('abort', cancelled); resolve(!signal.aborted);
        }, error => { signal.removeEventListener('abort', cancelled); reject(error); });
      });
      if (!ready || view.hasUnconfirmedDraft?.()) return false;
    }
    await this.awaitTransactions();
    return !signal.aborted && !this.closing;
  }
  async flushEdits(intent: 'explicit' | 'automatic' = 'explicit') {
    this.assertOpen();
    if (this.nativeCommit) await this.nativeCommit;
    for (const view of [...this.mounted]) {
      if (intent === 'automatic' && view.hasUnconfirmedDraft?.())
        throw new Error('自动保存正在等待尚未确认的编辑草稿。');
      await view.awaitCompositionEnd();
      if (this.mounted.has(view)) await view.commitPendingEdits();
    }
    await this.flushQueue();
  }
  async undo(redo = false) {
    await this.flushEdits();
    if (this.historyBusy) throw new Error('另一项历史操作尚未结束。');
    this.historyBusy = true;
    this.emit();
    try {
      // flushEdits 的 await 与历史门禁之间仍可能收到一笔输入。先关门，再排空它。
      await this.flushQueue();
      const status = await (redo ? this.port.redo : this.port.undo)(
        this.documentId,
        this.committed.revision,
      );
      this.committed = status;
      this.generation++;
      this.changed();
      return status;
    } finally {
      this.historyBusy = false;
      this.emit();
    }
  }
  private acceptNativeReplacement(result: ReplaceResult) {
    if (result.status.documentId !== this.documentId || result.status.revision < this.committed.revision)
      throw new DocumentConflictError('原生替换返回了过期确认。');
    const changed = result.status.revision !== this.committed.revision;
    this.committed = result.status;
    if (changed) { this.generation++; this.changed(); }
  }
  /** 准备期间允许输入，最终发布前关闭所有写入口并排空间隙事务。 */
  async commitNativeReplacement(base: { revision: number; generation: number }, signal: AbortSignal,
    commit: () => Promise<ReplaceResult>, mapped?: (result: ReplaceResult) => void): Promise<ReplaceResult> {
    await this.flushEdits(); signal.throwIfAborted();
    if (this.closing || this.historyBusy || this.nativeCommitBusy) throw new DocumentConflictError();
    this.nativeCommitBusy = true; this.emit();
    const operation = (async () => {
      await this.flushQueue(); signal.throwIfAborted();
      if (this.generation !== base.generation || this.committed.revision !== base.revision || this.pending.length)
        throw new DocumentConflictError('正文已有新输入，旧替换已取消。');
      let result: ReplaceResult;
      try { result = await commit(); }
      catch (error) {
        if (error instanceof DocumentConflictError || (error instanceof Error && typeof (error as { code?: unknown }).code === 'string')) throw error;
        // 未知传输失败只能重查同一 operationId，不能重新准备/执行替换。
        try { result = await commit(); }
        catch (failure) {
          this.nativeRecovery = async () => {
            let recovered: ReplaceResult;
            try { recovered = await commit(); }
            catch (error) {
              // 已释放候选且同版本仍在，确认该操作未发布；解除故障门禁，不在新请求中自动重做。
              if ((error as { code?: string })?.code === 'search-expired' &&
                (await this.port.status(this.documentId)).revision === base.revision)
                throw Object.assign(new Error('上次替换未提交，正文已保留，请重新执行替换。'), { code: 'replacement-not-committed' });
              throw error;
            }
            mapped?.(recovered); return recovered;
          };
          this.failure = failure; throw failure;
        }
      }
      try { mapped?.(result); }
      catch (error) { this.failure = error; }
      this.acceptNativeReplacement(result); return result;
    })();
    this.nativeCommit = operation;
    try { return await operation; }
    finally { this.nativeCommit = null; this.nativeCommitBusy = false; this.emit(); }
  }
  async captureSnapshot(intent: 'explicit' | 'automatic' = 'explicit', signal?: AbortSignal): Promise<SnapshotReference> {
    await this.awaitSnapshotReleases();
    await this.flushEdits(intent);
    signal?.throwIfAborted();
    await this.beforeSnapshot?.(signal);
    signal?.throwIfAborted();
    return this.port.captureSnapshot(this.documentId);
  }
  async withSnapshot<T>(
    task: (snapshot: SnapshotReference) => Promise<T>,
    intent: 'explicit' | 'automatic' = 'explicit',
    signal?: AbortSignal,
  ): Promise<T> {
    return this.withSnapshotTask(async () => {
      const snapshot = await this.captureSnapshot(intent, signal);
      try { return await task(snapshot); }
      finally { await this.releaseSnapshot(snapshot); }
    });
  }
  /** 包括搜索的目录准备；登记必须先于任何可能取得快照的异步工作。 */
  async withSnapshotTask<T>(task: () => Promise<T>): Promise<T> {
    this.assertOpen();
    if (this.closing) throw new Error('文档正在关闭。');
    const operation = Promise.resolve().then(task);
    this.snapshotTasks.add(operation);
    try { return await operation; }
    finally { this.snapshotTasks.delete(operation); }
  }
  async awaitSnapshotTasks() {
    while (this.snapshotTasks.size) await Promise.allSettled([...this.snapshotTasks]);
  }
  releaseSnapshot(snapshot: SnapshotReference): Promise<void> {
    const pending = this.snapshotReleases.get(snapshot);
    if (pending) return pending;
    const operation = Promise.resolve().then(() => this.port.releaseSnapshot(snapshot)).then(
      () => { this.snapshotReleases.delete(snapshot); },
      error => { this.snapshotReleases.set(snapshot, null); throw error; },
    );
    this.snapshotReleases.set(snapshot, operation);
    return operation;
  }
  async awaitSnapshotReleases() {
    while (this.snapshotReleases.size)
      await Promise.all([...this.snapshotReleases.keys()].map(snapshot => this.releaseSnapshot(snapshot)));
  }
  async save(path: string, options: { overwriteExternal?: boolean; automatic?: boolean } = {}) {
    return this.withSnapshot(
      (snapshot) => this.saveCaptured(snapshot, path, options.overwriteExternal),
      options.automatic ? 'automatic' : 'explicit',
    );
  }
  async saveCaptured(snapshot: SnapshotReference, path: string, overwriteExternal = false) {
        const status = await this.port.saveSnapshot(snapshot, path, overwriteExternal);
        // 保存期间的输入继续排队。返回的原生状态可能早于刚确认的新输入，需重新取状态。
        this.committed =
          status.revision < this.committed.revision
            ? await this.port.status(this.documentId)
            : status;
        this.emit();
        return this.committed;
  }
  async close(discard = false) {
    await this.prepareClose(discard);
    await this.closePrepared(discard);
  }
  async closePrepared(discard: boolean, closeStorage?: () => Promise<void>) {
    this.assertOpen();
    if (!this.closing) throw new Error('文档尚未准备关闭。');
    try {
      if (closeStorage) await closeStorage();
      else await this.port.close(this.documentId, discard);
    } catch (error) {
      this.cancelClose();
      throw error;
    }
    for (const pending of this.pending)
      pending.reject(new Error('用户已丢弃未保存输入并关闭文档。'));
    this.pending = [];
    this.closed = true;
    this.mounted.clear();
    this.listeners.clear();
    this.changeListeners.clear();
  }
  /** 先阻止新快照和输入，再排空已开始的任务；索引快照由文档运行时随后释放。 */
  async prepareClose(discard = false) {
    this.assertOpen();
    if (this.nativeCommit) {
      if (discard) await this.nativeCommit.catch(() => undefined);
      else await this.nativeCommit;
    }
    if (!discard) await this.flushEdits();
    if (this.closing) throw new Error('文档正在关闭。');
    this.closing = true;
    this.emit();
    try {
      if (!discard) await this.flushQueue();
      else await this.running;
      await this.awaitSnapshotTasks();
      await this.awaitSnapshotReleases();
    } catch (error) {
      this.cancelClose();
      throw error;
    }
  }
  cancelClose() {
    this.closing = false;
    this.emit();
  }
}
