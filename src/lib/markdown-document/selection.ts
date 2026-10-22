import type { MarkdownSessionController } from './controller';
import type { SnapshotReference } from './port';
import { DocumentConflictError, type DocumentSelection } from './protocol';
import { mapMarkdownSelection } from './positions';
import { utf8Length } from './utf8Text';

interface SelectedSnapshot {
  snapshot: SnapshotReference;
  selection: DocumentSelection;
  generation: number;
  selectionVersion: number;
}

/**
 * 选区属于完整会话，DOM 只绘制可见交集。跨窗口复制和替换不能从某一个 PM/CM 取全文。
 * 剪切先写剪贴板，确认正文和选区都未变化后才删除；失败仍保留原文。
 */
export class MarkdownGlobalSelection {
  private current: DocumentSelection;
  private byteLength: number;
  private version = 0;
  private disposed = false;
  private readonly listeners = new Set<(selection: DocumentSelection) => void>();
  private readonly unsubscribeChanges: () => void;
  private readonly unsubscribeState: () => void;

  constructor(readonly session: MarkdownSessionController) {
    this.byteLength = session.state.committed.byteLength;
    this.current = session.state.committed.selection ?? { anchorByte: 0, headByte: 0 };
    this.unsubscribeChanges = session.subscribeChanges((change) => {
      if (change.edits) {
        this.byteLength += change.edits.reduce(
          (delta, edit) => delta + utf8Length(edit.insertedText) - (edit.toByte - edit.fromByte),
          0,
        );
        this.assign(mapMarkdownSelection(this.current, change.edits));
      } else {
        this.byteLength = session.state.committed.byteLength;
        this.assign(
          session.state.committed.selection ?? {
            anchorByte: Math.min(this.current.anchorByte, this.byteLength),
            headByte: Math.min(this.current.headByte, this.byteLength),
          },
        );
      }
    });
    this.unsubscribeState = session.subscribe((state) => {
      if (!state.pendingTransactions) this.byteLength = state.committed.byteLength;
    });
  }
  get selection(): DocumentSelection {
    return { ...this.current };
  }
  get range() {
    return {
      fromByte: Math.min(this.current.anchorByte, this.current.headByte),
      toByte: Math.max(this.current.anchorByte, this.current.headByte),
    };
  }
  private assertOpen() {
    if (this.disposed) throw new Error('文档选区已释放。');
  }
  private assign(selection: DocumentSelection) {
    if (
      selection.anchorByte === this.current.anchorByte &&
      selection.headByte === this.current.headByte
    )
      return;
    this.current = { ...selection };
    this.version++;
    for (const listener of this.listeners) {
      try {
        listener(this.selection);
      } catch (error) {
        queueMicrotask(() => {
          throw error;
        });
      }
    }
  }
  set(selection: DocumentSelection) {
    this.assertOpen();
    if (
      [selection.anchorByte, selection.headByte].some(
        (byte) => !Number.isSafeInteger(byte) || byte < 0 || byte > this.byteLength,
      )
    )
      throw new RangeError('全局选区超出文档范围。');
    this.assign(selection);
  }
  selectAll() {
    this.set({ anchorByte: 0, headByte: this.byteLength });
  }
  /** 窗口可据此绘制局部选区；不能为了选区而挂载窗口外正文。 */
  visibleRange(fromByte: number, toByte: number) {
    const range = this.range;
    const from = Math.max(fromByte, range.fromByte);
    const to = Math.min(toByte, range.toByte);
    return from <= to ? { fromByte: from, toByte: to } : null;
  }
  subscribe(listener: (selection: DocumentSelection) => void) {
    this.assertOpen();
    this.listeners.add(listener);
    listener(this.selection);
    return () => this.listeners.delete(listener);
  }
  private async withSelectedSnapshot<T>(run: (selected: SelectedSnapshot) => Promise<T>) {
    this.assertOpen();
    await this.session.flushEdits();
    const selected: Omit<SelectedSnapshot, 'snapshot'> = {
      selection: this.selection,
      generation: this.session.state.viewGeneration,
      selectionVersion: this.version,
    };
    return this.session.withSnapshot(async snapshot => {
      if (
        selected.generation !== this.session.state.viewGeneration ||
        snapshot.revision !== this.session.state.committed.revision
      )
        throw new DocumentConflictError();
      return await run({ ...selected, snapshot });
    });
  }
  private async read(selected: SelectedSnapshot, signal?: AbortSignal) {
    const fromByte = Math.min(selected.selection.anchorByte, selected.selection.headByte);
    const toByte = Math.max(selected.selection.anchorByte, selected.selection.headByte);
    const chunks: string[] = [];
    for (let cursor = fromByte; cursor < toByte; ) {
      signal?.throwIfAborted();
      const window = await this.session.port.readSnapshot(
        selected.snapshot,
        cursor,
        Math.min(toByte, cursor + 256 * 1024),
      );
      if (
        window.documentId !== selected.snapshot.documentId ||
        window.revision !== selected.snapshot.revision ||
        window.fromByte !== cursor ||
        window.toByte <= cursor ||
        window.toByte > toByte
      )
        throw new Error('选区范围不是完整的 UTF-8 或换行边界。');
      chunks.push(window.text);
      cursor = window.toByte;
    }
    signal?.throwIfAborted();
    return chunks.join('');
  }
  copy(signal?: AbortSignal) {
    return this.withSelectedSnapshot((selected) => this.read(selected, signal));
  }
  private async replace(selected: SelectedSnapshot, insertedText: string) {
    this.assertOpen();
    if (
      selected.generation !== this.session.state.viewGeneration ||
      selected.selectionVersion !== this.version ||
      selected.snapshot.revision !== this.session.state.committed.revision
    )
      throw new DocumentConflictError('正文或选区已经变化，未应用旧选区操作。');
    const fromByte = Math.min(selected.selection.anchorByte, selected.selection.headByte);
    const toByte = Math.max(selected.selection.anchorByte, selected.selection.headByte);
    const after = fromByte + utf8Length(insertedText);
    const pending = this.session.applyEdits({
      viewGeneration: selected.generation,
      edits: [{ fromByte, toByte, insertedText }],
      historyGroupId: globalThis.crypto.randomUUID(),
      selectionBefore: selected.selection,
      selectionAfter: { anchorByte: after, headByte: after },
    });
    // applyEdits 同步映射所有视图；后续输入在此折叠光标上继续，确认后不再重置它。
    if (this.session.state.viewGeneration !== selected.generation)
      this.assign({ anchorByte: after, headByte: after });
    return pending;
  }
  replaceSelection(insertedText: string, canWrite?: () => boolean) {
    return this.withSelectedSnapshot((selected) => {
      if (canWrite && !canWrite()) throw new DocumentConflictError('目标区域已不可编辑。');
      return this.replace(selected, insertedText);
    });
  }
  async cut(writeClipboard: (text: string) => Promise<void>, signal?: AbortSignal, canWrite?: () => boolean) {
    return this.withSelectedSnapshot(async (selected) => {
      const text = await this.read(selected, signal);
      await writeClipboard(text);
      signal?.throwIfAborted();
      if (canWrite && !canWrite()) throw new DocumentConflictError('目标区域已不可编辑。');
      return this.replace(selected, '');
    });
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.unsubscribeChanges();
    this.unsubscribeState();
    this.listeners.clear();
  }
}
