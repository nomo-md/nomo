import { MarkdownSessionController } from './controller';
import { buildMarkdownIndex, type MarkdownIndexedSnapshot } from './incrementalIndex';
import { invalidateMarkdownIndex, type MarkdownIndexInvalidation } from './indexInvalidation';
import { MarkdownMountPolicy, type MarkdownMountMode } from './mountPolicy';
import type { MarkdownDocumentPort, MarkdownSessionStatus, SnapshotReference } from './port';
import { readCompleteSnapshot } from './port';
import { DocumentConflictError, type DocumentWindow, type DocumentIndexProgress } from './protocol';
import { markdownWorkScheduler, MarkdownTaskPriority } from './scheduler';
import type { MarkdownStructureIndex } from './structureIndex';
import { MarkdownGlobalSelection } from './selection';
import { AutomaticMarkdownToc } from './automaticToc';
import {
  searchMarkdownSnapshot,
  replaceMarkdownSnapshotMatches,
  type MarkdownTaskProgress,
} from './tasks';

export interface MarkdownLoadingState {
  status: MarkdownSessionStatus;
  index: MarkdownStructureIndex | null;
  progress: DocumentIndexProgress;
  mountMode: MarkdownMountMode;
  failure: unknown | null;
  timings: {
    captured: number;
    readable?: number;
    editable?: number;
    indexed?: number;
    stable?: number;
  };
}
export interface PreparedMarkdownWindow {
  window: DocumentWindow;
  mode: 'semantic' | 'raw';
  blockIds: string[];
}

/**
 * 文档级运行时只持有会话、索引和可取消任务。视图可以卸载，完整正文和历史不会随之释放。
 * 保留上一个完整索引快照用于增量收敛，最多另持有一个正在构建的快照。
 */
export class ProgressiveMarkdownDocument {
  readonly session: MarkdownSessionController;
  readonly selection: MarkdownGlobalSelection;
  private toc: AutomaticMarkdownToc;
  private completed: MarkdownIndexedSnapshot | null = null;
  private indexingSnapshot: SnapshotReference | undefined;
  private currentIndex: MarkdownStructureIndex | null = null;
  private invalidation?: MarkdownIndexInvalidation;
  private forceFullIndex = false;
  private abort: AbortController | null = null;
  private building: Promise<void> | null = null;
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private idleTimer: ReturnType<typeof setTimeout> | null = null;
  private hidden = false;
  private closed = false;
  private failure: unknown | null = null;
  private policy = new MarkdownMountPolicy();
  private mode: MarkdownMountMode = 'windowed';
  private readonly listeners = new Set<(state: MarkdownLoadingState) => void>();
  private unsubscribe: () => void;
  private unsubscribeChanges: () => void;
  private readonly started: number;
  private readonly timings: MarkdownLoadingState['timings'];
  private latestInteraction = 0;
  private queuedYield: (() => void) | null = null;
  private tocPossible: boolean | null = null;
  private publishedSignature = '';
  private publishedFailure: unknown;

  constructor(
    readonly port: MarkdownDocumentPort,
    status: MarkdownSessionStatus,
    private readonly build: typeof buildMarkdownIndex = buildMarkdownIndex,
    started = performance.now(),
  ) {
    this.started = started;
    this.timings = { captured: performance.now() - this.started };
    this.session = new MarkdownSessionController(port, status);
    this.toc = new AutomaticMarkdownToc(this.session, build);
    this.session.beforeSnapshot = signal => this.materializeToc(signal);
    this.selection = new MarkdownGlobalSelection(this.session);
    this.unsubscribeChanges = this.session.subscribeChanges((change) => {
      this.abort?.abort();
      this.generation++;
      // 编辑发生于首轮渐进索引期间时，保留已经发布的结构身份及对应不可变快照。
      if (change.edits && this.currentIndex && this.indexingSnapshot &&
        this.currentIndex.revision === this.indexingSnapshot.revision &&
        this.completed?.index !== this.currentIndex) {
        const previous = this.completed;
        this.completed = { index: this.currentIndex, snapshot: this.indexingSnapshot };
        this.invalidation = undefined;
        this.forceFullIndex = false;
        if (previous) void this.port.releaseSnapshot(previous.snapshot).catch(error => {
          this.failure = error;
          this.publish();
        });
      }
      if (!change.edits || change.edits.some(edit => edit.insertedText.includes('<') ||
        this.completed?.index.blockAt(edit.fromByte)?.kind === 'html_block')) this.tocPossible = null;
      if (change.edits && !this.forceFullIndex)
        this.invalidation = invalidateMarkdownIndex(this.invalidation, change.edits);
      else {
        this.forceFullIndex = true;
        this.invalidation = undefined;
      }
      this.currentIndex = null;
      this.timings.indexed = undefined;
      this.interacting();
      this.scheduleIndex();
    });
    this.unsubscribe = this.session.subscribe((state) => {
      this.failure = state.failure;
      this.publish();
      if (!state.pendingTransactions && this.currentIndex?.revision !== state.committed.revision)
        this.scheduleIndex();
    });
  }
  static async open(
    port: MarkdownDocumentPort,
    input: Parameters<MarkdownDocumentPort['openDocument']>[0],
  ) {
    const started = performance.now();
    return new ProgressiveMarkdownDocument(
      port,
      await port.openDocument(input),
      buildMarkdownIndex,
      started,
    );
  }
  get state(): MarkdownLoadingState {
    const status = this.session.state.committed;
    const progress =
      this.currentIndex?.revision === status.revision
        ? this.currentIndex.progress
        : {
            revision: status.revision,
            state: this.failure ? ('failed' as const) : ('building' as const),
            indexedBytes: 0,
            totalBytes: status.byteLength,
            layoutUnits: 0,
            enhancementObjects: 0,
            metricsComplete: false,
          };
    return {
      status,
      index: this.currentIndex,
      progress,
      mountMode: this.mode,
      failure: this.failure,
      timings: { ...this.timings },
    };
  }
  subscribe(listener: (state: MarkdownLoadingState) => void) {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }
  private publish() {
    if (this.closed) return;
    this.mode = this.policy.update(
      this.state.progress,
      performance.now(),
      performance.now() - this.latestInteraction < 2000,
    );
    const state = this.state;
    const signature = JSON.stringify([state.status.revision, state.status.dirty, state.status.canUndo, state.status.canRedo,
      state.status.persistenceWarning, this.session.state.viewGeneration, this.session.state.pendingTransactions,
      state.progress.state, state.progress.indexedBytes, state.progress.layoutUnits, state.mountMode, state.timings]);
    // 滚动只更新时间和挂载策略；元数据没有变化时不让应用反复派生大纲、持久化标签。
    if (signature === this.publishedSignature && this.publishedFailure === state.failure) return;
    this.publishedSignature = signature; this.publishedFailure = state.failure;
    for (const listener of this.listeners) listener(state);
  }
  noteVisible(stage: 'readable' | 'editable' | 'stable') {
    if (this.timings[stage] === undefined) {
      this.timings[stage] = performance.now() - this.started;
      this.publish();
    }
  }
  interacting() {
    this.latestInteraction = performance.now();
    markdownWorkScheduler.interacting();
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.publish();
    // 回退到挂满必须连续空闲两秒；再触发一次策略检查，不能依赖用户继续操作。
    this.idleTimer = setTimeout(() => {
      this.idleTimer = null;
      this.publish();
      this.idleTimer = setTimeout(() => {
        this.idleTimer = null;
        this.publish();
      }, 2001);
    }, 2001);
  }
  setHidden(hidden: boolean) {
    this.hidden = hidden;
    markdownWorkScheduler.pause(this, hidden);
    if (!hidden) this.scheduleIndex();
  }
  private scheduleIndex() {
    if (this.closed || this.hidden || this.timer) return;
    this.timer = setTimeout(
      () => {
        this.timer = null;
        markdownWorkScheduler.enqueue(this, 'index', MarkdownTaskPriority.Index, () => {
          void this.refreshIndex();
        });
      },
      this.completed ? 80 : 0,
    );
  }
  private async refreshIndex() {
    if (
      this.closed ||
      this.hidden ||
      this.session.state.failure ||
      this.session.state.pendingTransactions
    )
      return;
    if (this.building) {
      await this.building;
      if (!this.closed) this.scheduleIndex();
      return;
    }
    if (
      this.currentIndex?.revision === this.session.state.committed.revision &&
      this.currentIndex.progress.state === 'ready'
    )
      return;
    const generation = this.generation;
    const abort = (this.abort = new AbortController());
    this.building = (async () => {
      let snapshot: SnapshotReference | undefined;
      try {
        snapshot = await this.port.captureSnapshot(this.session.documentId);
        this.indexingSnapshot = snapshot;
        abort.signal.throwIfAborted();
        const index = await this.build(this.port, snapshot, {
          signal: abort.signal,
          previous: !this.forceFullIndex ? (this.completed ?? undefined) : undefined,
          invalidation: this.invalidation,
          shouldYield: () =>
            new Promise<void>((resolve) => {
              this.queuedYield = resolve;
              markdownWorkScheduler.enqueue(this, 'index-yield', MarkdownTaskPriority.Index, () => {
                this.queuedYield = null;
                resolve();
              });
            }),
          onProgress: (index) => {
            if (
              this.closed ||
              generation !== this.generation ||
              index.revision !== this.session.state.committed.revision
            )
              return;
            this.currentIndex = index;
            this.publish();
          },
        });
        abort.signal.throwIfAborted();
        if (
          generation !== this.generation ||
          snapshot.revision !== this.session.state.committed.revision
        )
          return;
        const previous = this.completed;
        this.completed = { index, snapshot };
        snapshot = undefined;
        this.currentIndex = index;
        this.tocPossible = index.blocks.some(block => block.kind === 'toc_block' || block.kind === 'html_block');
        this.invalidation = undefined;
        this.forceFullIndex = false;
        this.timings.indexed = performance.now() - this.started;
        this.failure = null;
        this.publish();
        if (previous) await this.port.releaseSnapshot(previous.snapshot);
        if (this.completed && !this.hidden) {
          await this.toc.apply(this.completed.index, this.completed.snapshot);
        }
      } catch (error) {
        if (!abort.signal.aborted && !this.closed) {
          this.failure = error;
          this.publish();
        }
      } finally {
        if (snapshot && snapshot !== this.completed?.snapshot) await this.port.releaseSnapshot(snapshot);
        this.indexingSnapshot = undefined;
      }
    })().finally(() => {
      this.building = null;
    });
    await this.building;
  }
  /** 读取的是当前完整会话中的有界范围；未证明结构安全的区域先用原文编辑。 */
  async prepareWindow(
    byte: number,
    requestedMode: 'semantic' | 'source' = 'semantic',
    byteBudget = 16 * 1024,
  ): Promise<PreparedMarkdownWindow> {
    await this.session.awaitTransactions();
    const status = this.session.state.committed;
    const generation = this.session.state.viewGeneration;
    byte = Math.max(0, Math.min(byte, status.byteLength));
    const index = this.currentIndex?.revision === status.revision ? this.currentIndex : null;
    const block = index?.blockAt(byte);
    const semantic =
      requestedMode === 'semantic' &&
      block?.mode === 'semantic' &&
      block.toByte - block.fromByte <= 32 * 1024;
    let from = semantic ? block.fromByte : Math.max(0, byte - Math.floor(byteBudget / 4));
    let to = semantic ? block.toByte : Math.min(status.byteLength, from + byteBudget);
    if (!semantic && block?.mode === 'raw') {
      from = Math.max(block.fromByte, from);
      to = Math.min(block.toByte, to);
    }
    const window = await this.port.readWindow(status.documentId, status.revision, from, to);
    if (generation !== this.session.state.viewGeneration) throw new DocumentConflictError();
    return { window, mode: semantic ? 'semantic' : 'raw', blockIds: block ? [block.id] : [] };
  }
  readMarkdown(signal?: AbortSignal) {
    return this.session.withSnapshot((snapshot) =>
      readCompleteSnapshot(this.port, snapshot, signal),
      'explicit', signal,
    );
  }
  /** 保存、恢复、导出在冻结前物化同版本目录，后台窗口是否挂载不影响完整性。 */
  async materializeToc(signal?: AbortSignal) {
    // 初始版本保持原片段；已证明没有目录的普通文档无需为保存/搜索等待结构任务。
    if (this.session.state.committed.revision === 0 || this.tocPossible === false) return;
    for (;;) {
      signal?.throwIfAborted();
      await this.session.awaitTransactions();
      const snapshot = await this.port.captureSnapshot(this.session.documentId);
      try {
        const index = this.currentIndex?.revision === snapshot.revision && this.currentIndex.progress.state === 'ready'
          ? this.currentIndex : await this.build(this.port, snapshot, {
              signal: signal ?? new AbortController().signal, onProgress() {},
              previous: !this.forceFullIndex ? this.completed ?? undefined : undefined,
              invalidation: this.invalidation,
              shouldYield: () => new Promise(resolve => setTimeout(resolve, 0)),
            });
        this.tocPossible = index.blocks.some(block => block.kind === 'toc_block' || block.kind === 'html_block');
        const changed = await this.toc.apply(index, snapshot);
        if (!changed && snapshot.revision === this.session.state.committed.revision && !this.session.state.pendingTransactions) return;
      } finally { await this.port.releaseSnapshot(snapshot); }
    }
  }
  async search(
    query: string,
    options: {
      caseSensitive?: boolean;
      wholeWord?: boolean;
      signal: AbortSignal;
      onProgress?(progress: MarkdownTaskProgress): void;
    },
  ) {
    return this.session.withSnapshot(async (snapshot) => ({
      snapshotRevision: snapshot.revision,
      matches: await searchMarkdownSnapshot(this.port, snapshot, query, options),
    }), 'explicit', options.signal);
  }
  async replaceAll(
    query: string,
    replacement: string,
    options: {
      caseSensitive?: boolean;
      wholeWord?: boolean;
      signal: AbortSignal;
      onProgress?(progress: MarkdownTaskProgress): void;
    },
  ) {
    return this.session.withSnapshot(async (snapshot) => {
      const generation = this.session.state.viewGeneration;
      const matches = await searchMarkdownSnapshot(this.port, snapshot, query, options);
      options.signal.throwIfAborted();
      await replaceMarkdownSnapshotMatches(
        this.session,
        snapshot,
        matches,
        replacement,
        generation,
      );
      return matches.length;
    }, 'explicit', options.signal);
  }
  async retry() {
    this.failure = null;
    await this.session.retryPendingEdits();
    this.scheduleIndex();
    this.publish();
  }
  async suspendTasks() {
    this.setHidden(true); this.abort?.abort(); this.generation++;
    if (this.timer) { clearTimeout(this.timer); this.timer = null; }
    markdownWorkScheduler.cancel(this); this.queuedYield?.(); this.queuedYield = null;
    await this.building;
    await this.toc.dispose(); this.toc = new AutomaticMarkdownToc(this.session, this.build);
    if (this.completed) { await this.port.releaseSnapshot(this.completed.snapshot); this.completed = null; }
    this.currentIndex = null;
  }
  /** 跨窗归属已提交后只释放本 WebView 的任务与快照，不关闭原生文档。 */
  async detach() {
    if (this.closed) return;
    this.closed = true; this.abort?.abort();
    if (this.timer) clearTimeout(this.timer);
    if (this.idleTimer) clearTimeout(this.idleTimer);
    markdownWorkScheduler.cancel(this); this.queuedYield?.(); this.queuedYield = null;
    await this.building;
    await this.session.awaitSnapshotTasks();
    await this.toc.dispose();
    if (this.completed) { await this.port.releaseSnapshot(this.completed.snapshot); this.completed = null; }
    this.unsubscribe(); this.unsubscribeChanges(); this.selection.dispose(); this.listeners.clear();
    await this.port.detach?.(this.session.documentId);
  }
  /** 卸载使用 setHidden；只有完成关闭/丢弃流程才释放会话和磁盘历史。 */
  async close(discard = false) {
    if (this.closed) return;
    if (!discard) await this.session.flushEdits();
    this.closed = true;
    this.abort?.abort();
    if (this.timer) clearTimeout(this.timer);
    if (this.idleTimer) clearTimeout(this.idleTimer);
    markdownWorkScheduler.cancel(this);
    this.queuedYield?.();
    this.queuedYield = null;
    await this.building;
    await this.toc.dispose();
    if (this.completed) {
      await this.port.releaseSnapshot(this.completed.snapshot);
      this.completed = null;
    }
    try {
      await this.session.close(discard);
    } catch (error) {
      this.closed = false;
      this.scheduleIndex();
      throw error;
    }
    this.unsubscribe();
    this.unsubscribeChanges();
    this.selection.dispose();
    this.listeners.clear();
  }
}
