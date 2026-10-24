import { MarkdownSessionController } from './controller';
import { recordAcceptance } from './acceptanceProbe';
import { buildMarkdownIndex, type MarkdownIndexedSnapshot, type MarkdownIndexChange } from './incrementalIndex';
import { invalidateMarkdownIndex, type MarkdownIndexInvalidation } from './indexInvalidation';
import { MarkdownMountPolicy, type MarkdownMountMode } from './mountPolicy';
import { DEFAULT_MARKDOWN_MOUNT_LIMITS, type MarkdownMountLimits } from './windowingLimits';
import type { MarkdownDocumentPort, MarkdownSessionStatus, SnapshotReference } from './port';
import { readCompleteSnapshot } from './port';
import { DocumentConflictError, type DocumentWindow, type DocumentIndexProgress } from './protocol';
import { markdownWorkScheduler, MarkdownTaskPriority } from './scheduler';
import type { MarkdownStructureIndex } from './structureIndex';
import { MarkdownGlobalSelection } from './selection';
import { AutomaticMarkdownToc } from './automaticToc';
import { emptyEnhancements } from './enhancements';
import { createMarkdownDocumentLayouts } from './layoutCache';
import { MarkdownSearchController } from './searchController';
import type { SearchOptions } from './searchProtocol';
import { NativeStructureClient } from './nativeStructureClient';
import { NativeMarkdownStructureIndex } from './nativeStructureIndex';
import type { StructureQueryPort, StructureTarget } from './structureProtocol';
import type { LocalHeadingWindow } from './localHeadingProjection';

export interface MarkdownLoadingState {
  status: MarkdownSessionStatus;
  index: MarkdownStructureIndex | null;
  progress: DocumentIndexProgress;
  mountMode: MarkdownMountMode;
  windowLimited: boolean;
  mountRestriction: MarkdownMountPolicy['restriction'];
  failure: unknown | null;
  timings: {
    captured: number;
    readable?: number;
    editable?: number;
    indexed?: number;
    stable?: number;
  };
}
export type PreparedMarkdownWindow = {
  window: DocumentWindow;
  mode: 'semantic' | 'source';
  blockIds: string[];
} | { mode: 'preparing' | 'restricted'; fromByte: number; toByte: number; blockIds: string[] };
export interface MarkdownVisibleDemand {
  byte: number;
  revision: number;
  generation: number;
}

/**
 * 文档级运行时只持有会话、索引和可取消任务。视图可以卸载，完整正文和历史不会随之释放。
 * 保留上一个完整索引快照用于增量收敛，最多另持有一个正在构建的快照。
 */
export class ProgressiveMarkdownDocument {
  readonly session: MarkdownSessionController;
  readonly selection: MarkdownGlobalSelection;
  readonly searches: MarkdownSearchController;
  readonly layouts = createMarkdownDocumentLayouts();
  private toc: AutomaticMarkdownToc;
  private completed: MarkdownIndexedSnapshot | null = null;
  private indexingSnapshot: SnapshotReference | undefined;
  private ending?: Promise<void>;
  private currentIndex: MarkdownStructureIndex | null = null;
  private previousNative: NativeMarkdownStructureIndex | null = null;
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
  private pressureOwners = new Set<object>();
  private mode: MarkdownMountMode = 'windowed';
  private readonly listeners = new Set<(state: MarkdownLoadingState, change?: MarkdownIndexChange) => void>();
  private unsubscribe: () => void;
  private unsubscribeChanges: () => void;
  private readonly started: number;
  private readonly timings: MarkdownLoadingState['timings'];
  private latestInteraction = 0;
  private queuedYield: (() => void) | null = null;
  private tocPossible: boolean | null = null;
  private publishedSignature = '';
  private publishedFailure: unknown;
  private visibleDemands = new Map<object, MarkdownVisibleDemand>();
  private nativeTocRunning: Promise<void> | null = null;
  private nativeTocAbort: AbortController | null = null;
  localHeadings = new Map<object, LocalHeadingWindow>();
  private localHeadingLease = Symbol('local-headings');
  private localHeadingRevision = -1;
  previewHeadings(owner: object, window: LocalHeadingWindow) {
    const next = new Map(this.localHeadings).set(owner, window);
    try { (this.nativeStructure ?? this.previousNative)?.client.retain(this.localHeadingLease, [...next.values()]); }
    catch (error) { this.failure = error; this.publish(true); return; }
    this.localHeadings = next;
    this.localHeadingRevision = this.session.state.committed.revision;
    this.publish(true);
  }
  get nativeStructure() { return this.currentIndex instanceof NativeMarkdownStructureIndex ? this.currentIndex : null; }
  releaseStructureOwner(owner: object) { this.nativeStructure?.release(owner); this.previousNative?.release(owner); }
  get usesNativeStructure() { return !!this.port.startStructure && !!this.port.queryStructure; }
  async resolveStructure(namespace: 'heading' | 'slug' | 'footnote' | 'link' | 'footnoteReference', key: string, signal = new AbortController().signal): Promise<StructureTarget & { revision: number }> {
    await this.session.awaitTransactions();
    for (;;) {
      signal.throwIfAborted();
      await this.refreshIndex();
      const client = this.nativeStructure?.client;
      if (!client) throw new Error('当前原生结构索引不可用。');
      const response = await client.query<StructureTarget>({ kind: 'resolve', namespace, key }, signal);
      if (response.revision !== this.session.state.committed.revision) continue;
      const result = response.items[0];
      if (!result) throw new Error('原生目标查询没有返回状态。');
      if (result.status !== 'incomplete' || response.state === 'ready') return { ...result, revision: response.revision };
      await new Promise(resolve => setTimeout(resolve, 30));
    }
  }

  constructor(
    readonly port: MarkdownDocumentPort,
    status: MarkdownSessionStatus,
    private readonly build: typeof buildMarkdownIndex = buildMarkdownIndex,
    started = performance.now(),
  ) {
    this.started = started;
    this.timings = { captured: performance.now() - this.started };
    this.session = new MarkdownSessionController(port, status);
    this.searches = new MarkdownSearchController(this.session);
    this.toc = new AutomaticMarkdownToc(this.session, build);
    this.session.beforeSnapshot = signal => this.materializeToc(signal);
    this.selection = new MarkdownGlobalSelection(this.session);
    this.unsubscribeChanges = this.session.subscribeChanges((change) => {
      this.visibleDemands.clear();
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
        if (previous) void this.session.releaseSnapshot(previous.snapshot).catch(error => {
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
      if (this.currentIndex instanceof NativeMarkdownStructureIndex) {
        if (this.currentIndex.progress.state === 'ready') {
          this.previousNative?.dispose(); this.previousNative = this.currentIndex;
          this.currentIndex.client.dispose();
        } else this.currentIndex.dispose();
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
            enhancements: emptyEnhancements(),
            metricsComplete: false,
          };
    return {
      status,
      index: this.currentIndex,
      progress,
      mountMode: this.mode,
      windowLimited: this.usesNativeStructure || this.policy.windowLimited,
      mountRestriction: this.policy.restriction,
      failure: this.failure,
      timings: { ...this.timings },
    };
  }
  get elapsed() { return performance.now() - this.started; }
  subscribe(listener: (state: MarkdownLoadingState, change?: MarkdownIndexChange) => void) {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }
  updateMountLimits(limits: MarkdownMountLimits) {
    if (this.closed || !this.policy.updateLimits(limits)) return;
    if (!this.hidden) this.publish(true);
  }
  private publish(force = false, change?: MarkdownIndexChange) {
    if (this.closed) return;
    this.mode = this.policy.update(
      this.state.progress,
      performance.now(),
      performance.now() - this.latestInteraction < 2000,
      this.pressureOwners.size > 0,
    );
    if (this.usesNativeStructure) this.mode = 'windowed';
    const state = this.state;
    if (this.idleTimer) clearTimeout(this.idleTimer);
    this.idleTimer = null;
    const quietAt = this.latestInteraction + DEFAULT_MARKDOWN_MOUNT_LIMITS.idleMilliseconds;
    const nextCheck = this.policy.recoveryAt ??
      (state.windowLimited && quietAt > performance.now() ? quietAt : null);
    if (!this.hidden && nextCheck !== null) {
      this.idleTimer = setTimeout(() => { this.idleTimer = null; this.publish(); },
        Math.max(1, nextCheck - performance.now() + 1));
    }
    const signature = JSON.stringify([state.status.revision, state.status.dirty, state.status.canUndo, state.status.canRedo,
      state.status.persistenceWarning, this.session.state.viewGeneration, this.session.state.pendingTransactions,
      this.session.state.nativeCommitBusy, this.session.state.historyBusy, this.session.state.closing,
      state.progress.state, state.progress.indexedBytes, state.progress.layoutUnits, state.progress.enhancements, state.mountMode, state.windowLimited, state.mountRestriction, state.timings]);
    // 滚动只更新时间和挂载策略；元数据没有变化时不让应用反复派生大纲、持久化标签。
    if (!force && signature === this.publishedSignature && this.publishedFailure === state.failure) return;
    this.publishedSignature = signature; this.publishedFailure = state.failure;
    if (import.meta.env.VITE_NOMO_ACCEPTANCE === '1') recordAcceptance('index', { documentId: state.status.documentId, revision: state.status.revision,
      state: state.progress.state, timings: state.timings, failure: !!state.failure });
    for (const listener of this.listeners) listener(state, change);
  }
  noteVisible(stage: 'readable' | 'editable' | 'stable') {
    if (this.timings[stage] === undefined) {
      this.timings[stage] = performance.now() - this.started;
      this.publish();
    }
  }
  setMountPressure(owner: object, pressured: boolean) {
    if (this.pressureOwners.has(owner) === pressured) return;
    if (pressured) this.pressureOwners.add(owner); else this.pressureOwners.delete(owner);
    this.publish();
  }
  interacting() {
    this.latestInteraction = performance.now();
    markdownWorkScheduler.interacting();
    this.publish();
  }
  setHidden(hidden: boolean) {
    this.hidden = hidden;
    const native = this.nativeStructure;
    if (native) void this.port.updateStructureDemand?.(native.client.identity, !hidden, 1).catch(error => {
      if ((error as { code?: string })?.code !== 'revision-conflict') { this.failure = error; this.publish(); }
    });
    if (hidden) {
      void this.searches.cancel().catch(error => { this.failure = error; this.publish(); });
      this.visibleDemands.clear();
      if (this.idleTimer) clearTimeout(this.idleTimer);
      this.idleTimer = null;
    }
    markdownWorkScheduler.pause(this, hidden);
    if (!hidden) { this.publish(); this.scheduleIndex(); }
  }
  /** 需求只提升下一扫描批次，不创建另一个索引或从未知结构内部解析。 */
  requestVisibleStructure(owner: object, demand: MarkdownVisibleDemand | null) {
    if (!demand && !this.visibleDemands.has(owner)) return;
    if (demand && !this.hidden && !this.closed && demand.revision === this.session.state.committed.revision)
      this.visibleDemands.set(owner, demand);
    else this.visibleDemands.delete(owner);
    if (this.queuedYield) this.enqueueIndexYield();
    else if (!this.building) this.scheduleIndex();
  }
  get structurePriority(): MarkdownTaskPriority {
    const status = this.session.state.committed;
    for (const [owner, demand] of this.visibleDemands) {
      const index = this.currentIndex?.revision === status.revision ? this.currentIndex : null;
      if (demand.revision !== status.revision || index?.progress.state === 'ready' ||
        (index && demand.byte < index.progress.indexedBytes)) this.visibleDemands.delete(owner);
    }
    return this.visibleDemands.size ? MarkdownTaskPriority.VisibleText : MarkdownTaskPriority.Index;
  }
  private enqueueIndexYield() {
    markdownWorkScheduler.enqueue(this, 'index-yield', this.structurePriority, () => {
      const resolve = this.queuedYield;
      this.queuedYield = null;
      resolve?.();
    });
  }
  private scheduleIndex() {
    if (this.closed || this.hidden || this.timer) return;
    this.timer = setTimeout(
      () => {
        this.timer = null;
        markdownWorkScheduler.enqueue(this, 'index', this.structurePriority, () => {
          void this.refreshIndex();
        });
      },
      this.completed || this.previousNative ? 80 : 0,
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
    if (this.usesNativeStructure) {
      this.building = (async () => {
        try {
          const status = this.session.state.committed;
          const client = await NativeStructureClient.open(this.port as StructureQueryPort, status.documentId, status.revision, abort.signal);
          const index = new NativeMarkdownStructureIndex(client, status.byteLength);
          if (generation !== this.generation || this.closed) { index.dispose(); return; }
          this.currentIndex = index;
          for (;;) {
            const complete = await index.refresh(abort.signal);
            if (generation !== this.generation || this.closed) return;
            if (this.hidden) { await this.port.updateStructureDemand?.(client.identity, false, 1); return; }
            if (complete && this.previousNative) {
              const previous = this.previousNative;
              await index.adopt(previous, abort.signal);
              previous.dispose(); if (this.previousNative === previous) this.previousNative = null;
            }
            if (complete && status.revision > this.localHeadingRevision && !this.session.hasUnsettledEdits) {
              this.localHeadings = new Map(); client.releaseRetained(this.localHeadingLease);
            }
            this.failure = null; this.publish(true);
            if (complete) { this.timings.indexed = performance.now() - this.started; this.publish(true); break; }
            // 首批结构尚未出现时不额外等待完整后台轮询周期；已有可见前缀后恢复低频查询。
            await new Promise(resolve => setTimeout(resolve, index.progress.indexedBytes ? 80 : 16)); abort.signal.throwIfAborted();
          }
        } catch (error) {
          if (!abort.signal.aborted && !this.closed) { this.failure = error; this.publish(); }
        }
      })().finally(() => {
        this.building = null;
        if (!this.closed && !this.hidden && this.nativeStructure?.progress.state === 'ready' && this.session.state.committed.revision > 0) {
          void this.materializeNativeToc().catch(error => {
            if (!this.closed && (error as { name?: string })?.name !== 'AbortError' &&
              (error as { code?: string })?.code !== 'revision-conflict' && !(error instanceof DocumentConflictError)) { this.failure = error; this.publish(); }
          });
        }
      });
      await this.building; return;
    }
    this.building = (async () => {
      let snapshot: SnapshotReference | undefined;
      try {
        await this.session.awaitSnapshotReleases();
        abort.signal.throwIfAborted();
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
              this.enqueueIndexYield();
            }),
          onProgress: (index, change) => {
            if (
              this.closed ||
              generation !== this.generation ||
              index.revision !== this.session.state.committed.revision
            )
              return;
            this.currentIndex = index;
            this.publish(false, change);
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
        if (previous) await this.session.releaseSnapshot(previous.snapshot);
        if (this.completed && !this.hidden) {
          await this.toc.apply(this.completed.index, this.completed.snapshot);
        }
      } catch (error) {
        if (!abort.signal.aborted && !this.closed) {
          this.failure = error;
          this.publish();
        }
      } finally {
        this.indexingSnapshot = undefined;
        if (snapshot && snapshot !== this.completed?.snapshot) await this.session.releaseSnapshot(snapshot);
      }
    })().finally(() => {
      this.building = null;
    });
    await this.building;
  }
  /** 语义请求明确区分等待与受限，只有主动源码请求才返回原文投影。 */
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
    const block = this.nativeStructure ? (await this.nativeStructure.client.blocks(Math.max(0, byte === status.byteLength ? byte - 1 : byte), byte + 1)).items[0] : index?.blockAt(byte);
    if (requestedMode === 'semantic' && status.byteLength > 0 &&
      (!block || byte > block.toByte || (byte === block.toByte && byte < status.byteLength)))
      return { mode: 'preparing', fromByte: byte, toByte: byte, blockIds: [] };
    if (requestedMode === 'semantic' && block && (block.mode === 'raw' || block.toByte - block.fromByte > 32 * 1024))
      return { mode: 'restricted', fromByte: block.fromByte, toByte: block.toByte, blockIds: [block.id] };
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
    return { window, mode: requestedMode === 'semantic' ? 'semantic' : 'source', blockIds: block ? [block.id] : [] };
  }
  readMarkdown(signal?: AbortSignal) {
    return this.session.withSnapshot((snapshot) =>
      readCompleteSnapshot(this.port, snapshot, signal),
      'explicit', signal,
    );
  }
  /** 保存、恢复、导出在冻结前物化同版本目录，后台窗口是否挂载不影响完整性。 */
  private materializeNativeToc(signal?: AbortSignal): Promise<void> {
    if (this.closed) return Promise.reject(new DOMException('文档已关闭。', 'AbortError'));
    if (this.nativeTocRunning) return this.nativeTocRunning;
    const abort = this.nativeTocAbort = new AbortController();
    const cancel = () => abort.abort(signal?.reason);
    if (signal?.aborted) cancel();
    else signal?.addEventListener('abort', cancel, { once: true });
    return this.nativeTocRunning = this.applyNativeToc(abort.signal).finally(() => {
      signal?.removeEventListener('abort', cancel);
      this.nativeTocRunning = null; this.nativeTocAbort = null;
    });
  }
  private async applyNativeToc(signal: AbortSignal) {
    if (this.session.state.committed.revision === 0) return;
    if (!this.port.materializeStructureToc) throw new Error('原生自动目录协议不可用。');
    await this.session.flushEdits();
    const base = { revision: this.session.state.committed.revision, generation: this.session.state.viewGeneration };
    const client = await NativeStructureClient.open(this.port as StructureQueryPort, this.session.documentId, base.revision, signal);
    try {
      await this.port.updateStructureDemand?.(client.identity, true, 2);
      while ((await client.query({ kind: 'progress' }, signal)).state !== 'ready') {
        await new Promise(resolve => setTimeout(resolve, 40)); signal.throwIfAborted();
      }
      const request = { identity: client.identity, operationId: crypto.randomUUID(), editGeneration: base.generation,
        selection: this.selection.selection };
      await this.session.commitNativeMutation(base, signal, () => this.port.materializeStructureToc!(request));
    } finally {
      if (this.hidden) await this.port.updateStructureDemand?.(client.identity, false, 1).catch(() => undefined);
      client.dispose();
    }
  }
  async materializeToc(signal?: AbortSignal) {
    if (this.usesNativeStructure) {
      for (;;) {
        signal?.throwIfAborted();
        try { await this.materializeNativeToc(signal); return; }
        catch (error) {
          if (this.closed || (!(error instanceof DocumentConflictError) &&
              (error as { code?: string })?.code !== 'revision-conflict')) throw error;
        }
      }
    }
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
      } finally { await this.session.releaseSnapshot(snapshot); }
    }
  }
  search(query: string, options: SearchOptions) {
    return this.searches.search(query, options);
  }
  async replaceAll(query: string, replacement: string, options: SearchOptions) {
    const result = await this.searches.replaceAll(query, replacement, options, () => ({ selection: this.selection.selection }));
    return result.count;
  }
  async retry() {
    try {
      await this.port.status(this.session.documentId);
    } catch (error) {
      if (!(error instanceof Error) || !('code' in error) || error.code !== 'document-closed') throw error;
      const recovered = await this.port.openDocument({ recoveryId: this.session.documentId });
      // 只接回同一份持久化正文；版本不一致时保留前端待确认输入，不把它套到未知基线上。
      if (recovered.documentId !== this.session.documentId ||
        recovered.revision !== this.session.state.committed.revision)
        throw new DocumentConflictError('恢复资料版本已变化，当前编辑仍保留，请重新打开文档核对。');
    }
    this.failure = null;
    await this.session.retryPendingEdits();
    this.scheduleIndex();
    this.publish();
  }
  async suspendTasks() {
    await this.searches.cancel();
    this.setHidden(true); this.abort?.abort(); this.generation++;
    if (this.timer) { clearTimeout(this.timer); this.timer = null; }
    markdownWorkScheduler.cancel(this); this.queuedYield?.(); this.queuedYield = null;
    await this.building?.catch(() => undefined);
    try { await this.toc.dispose(); }
    finally { this.toc = new AutomaticMarkdownToc(this.session, this.build); }
    if (this.completed) { await this.session.releaseSnapshot(this.completed.snapshot); this.completed = null; }
    await this.session.awaitSnapshotReleases();
    this.nativeStructure?.dispose(); this.previousNative?.dispose(); this.previousNative = null; this.currentIndex = null;
  }
  /** 关闭准备只取消可重建索引，保护保存/导出正在物化的目录和快照。 */
  cancelBackgroundWork() {
    this.setHidden(true);
    this.abort?.abort(); this.generation++;
    if (this.timer) { clearTimeout(this.timer); this.timer = null; }
    markdownWorkScheduler.cancel(this); this.queuedYield?.(); this.queuedYield = null;
  }
  private finish(operation: () => Promise<void>): Promise<void> {
    if (this.ending) return this.ending;
    const ending = Promise.resolve().then(operation);
    this.ending = ending;
    void ending.finally(() => { if (this.ending === ending) this.ending = undefined; }).catch(() => {});
    return ending;
  }
  /** 跨窗归属已提交后只释放本 WebView 的任务与快照，不关闭原生文档。 */
  detach(): Promise<void> { return this.finish(() => this.detachResources()); }
  private async detachResources() {
    if (this.closed) return;
    await this.closeResources(true, () => this.port.detach?.(this.session.documentId) ?? Promise.resolve());
  }
  /** 卸载使用 setHidden；只有完成关闭/丢弃流程才释放会话和磁盘历史。 */
  close(discard = false, closeStorage?: () => Promise<void>): Promise<void> {
    return this.finish(() => this.closeResources(discard, closeStorage));
  }
  private async closeResources(discard: boolean, closeStorage?: () => Promise<void>) {
    if (this.closed) return;
    await this.searches.cancel();
    if (!discard) await this.session.flushEdits();
    this.closed = true;
    this.nativeTocAbort?.abort();
    this.visibleDemands.clear();
    this.abort?.abort();
    if (this.timer) clearTimeout(this.timer);
    if (this.idleTimer) clearTimeout(this.idleTimer);
    markdownWorkScheduler.cancel(this); this.queuedYield?.(); this.queuedYield = null;
    let tocDisposed = false;
    try {
      await this.session.prepareClose(discard);
      await this.building?.catch(() => undefined);
      await this.nativeTocRunning?.catch(() => undefined);
      tocDisposed = true;
      await this.toc.dispose();
      if (this.completed) {
        await this.session.releaseSnapshot(this.completed.snapshot);
        this.completed = null;
      }
      await this.session.awaitSnapshotReleases();
      await this.session.closePrepared(discard, closeStorage);
    } catch (error) {
      this.session.cancelClose();
      if (tocDisposed) this.toc = new AutomaticMarkdownToc(this.session, this.build);
      this.closed = false;
      this.scheduleIndex();
      throw error;
    }
    this.unsubscribe(); this.unsubscribeChanges(); this.selection.dispose(); this.listeners.clear(); this.pressureOwners.clear();
    this.nativeStructure?.dispose(); this.previousNative?.dispose(); this.previousNative = null;
    this.localHeadings.clear(); NativeStructureClient.releaseDocument(this.session.documentId);
    await this.searches.dispose();
    this.layouts.dispose();
  }
}
