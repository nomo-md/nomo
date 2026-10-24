import { MarkdownSessionController, type MarkdownControllerState } from './controller';
import {
  DocumentConflictError,
  type DocumentSelection,
  type DocumentWindow,
  type SourceEdit,
} from './protocol';
import { editsTouchRange, mapMarkdownByte } from './positions';
import { utf8Length, utf8ToUtf16 } from './utf8Text';

/** 一个挂载窗口的待确认输入。只缓存该范围，跨视图修改后必须重新读取，不能补回旧内容。 */
export class MarkdownWindowBridge {
  private generation: number;
  private submitting = false;
  private pending = 0;
  private stale = false;
  private unsubscribe: () => void;
  private unsubscribeChanges: () => void;
  private source: string;
  constructor(
    readonly session: MarkdownSessionController,
    readonly window: DocumentWindow,
    private readonly callbacks: {
      onStale(): void;
      onRebase?(fromByte: number): void;
      onDerivedSource?(source: string): boolean;
      onState?(state: MarkdownControllerState): void;
      onError(error: unknown): void;
    },
  ) {
    if (session.state.pendingTransactions || session.state.committed.revision !== window.revision)
      throw new DocumentConflictError();
    this.source = window.text;
    this.generation = session.state.viewGeneration;
    this.unsubscribeChanges = session.subscribeChanges((change) => {
      if (this.submitting || this.stale) return;
      if (change.edits && !editsTouchRange(change.edits, this.window)) {
        this.window.fromByte = mapMarkdownByte(this.window.fromByte, change.edits);
        this.window.toByte = mapMarkdownByte(this.window.toByte, change.edits);
        this.generation = change.generation;
        callbacks.onRebase?.(this.window.fromByte);
      } else if (change.derived && change.edits && !this.pending && callbacks.onDerivedSource) {
        const touching = change.edits.filter(edit => editsTouchRange([edit], this.window));
        if (!touching.length || touching.some(edit => edit.fromByte < this.window.fromByte || edit.toByte > this.window.toByte)) return;
        let source = this.source;
        for (const edit of [...touching].reverse()) {
          const from = utf8ToUtf16(this.source, edit.fromByte - this.window.fromByte);
          const to = utf8ToUtf16(this.source, edit.toByte - this.window.fromByte);
          source = source.slice(0, from) + edit.insertedText + source.slice(to);
        }
        if (!callbacks.onDerivedSource(source)) return;
        this.source = source;
        this.window.fromByte = mapMarkdownByte(this.window.fromByte, change.edits, -1);
        this.window.toByte = this.window.fromByte + utf8Length(source);
        this.generation = change.generation;
        callbacks.onRebase?.(this.window.fromByte);
      }
    });
    this.unsubscribe = session.subscribe((state) => {
      if (!state.failure && state.pendingTransactions === 0) this.pending = 0;
      if (this.submitting) this.generation = state.viewGeneration;
      else if (this.generation !== state.viewGeneration && !this.stale) {
        this.stale = true;
        callbacks.onStale();
      }
      // 只确认整个乐观输入队列；中间确认不能把后续输入标成旧修订的内容。
      if (!state.failure && !state.pendingTransactions && !this.stale)
        this.window.revision = state.committed.revision;
      callbacks.onState?.(state);
    });
  }
  get text() {
    return this.source;
  }
  get pinned() {
    return this.pending > 0;
  }
  get isStale() {
    return this.stale;
  }
  isSynchronized(revision: number) {
    const state = this.session.state;
    return !this.stale && !this.pending && !state.pendingTransactions && !state.failure &&
      !state.historyBusy && !state.nativeCommitBusy && !state.closing &&
      this.generation === state.viewGeneration && this.window.revision === revision &&
      state.committed.revision === revision;
  }
  commit(
    change: {
      source: string;
      edits: SourceEdit[];
      selectionBefore?: DocumentSelection;
      selectionAfter?: DocumentSelection;
    },
    historyGroupId: string,
  ) {
    if (this.stale || this.session.state.historyBusy || this.session.state.nativeCommitBusy || this.session.state.closing)
      throw new DocumentConflictError();
    if (!change.edits.length) return;
    this.pending++;
    this.submitting = true;
    const result = this.session.applyEdits({
      ...change,
      historyGroupId,
      viewGeneration: this.generation,
    });
    this.submitting = false;
    this.source = change.source;
    this.window.toByte = this.window.fromByte + utf8Length(change.source);
    void result.catch((error) => {
      this.callbacks.onError(error);
    });
  }
  dispose() {
    if (this.pending) throw new Error('窗口仍有未确认输入，不能回收。');
    this.unsubscribeChanges();
    this.unsubscribe();
  }
}
