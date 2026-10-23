import {
  DocumentConflictError,
  type DocumentSelection,
  type DocumentStatus,
  type DocumentTransaction,
  type DocumentWindow,
} from './protocol';
import { Utf8Text } from './utf8Text';

interface HistoryEntry {
  group: string;
  before: Utf8Text;
  after: Utf8Text;
  selectionBefore?: DocumentSelection;
  selectionAfter?: DocumentSelection;
}
export interface MarkdownSnapshot {
  readonly documentId: string;
  readonly revision: number;
  readonly text: Utf8Text;
}

/** 纯会话内核：不持有 DOM/组件回调，不因视图释放而销毁正文或历史。 */
export class MarkdownDocumentSession {
  private text: Utf8Text;
  private saved: Utf8Text;
  private revision = 0;
  private history: HistoryEntry[] = [];
  private cursor = 0;
  private selection?: DocumentSelection;
  private acknowledgements = new Map<string, { fingerprint: string; status: DocumentStatus }>();
  private closed = false;
  private historyBarrier = false;
  private hasDerivedEdits = false;
  private acknowledgementBytes = 0;
  constructor(
    readonly documentId: string,
    source: string,
    savedSource = source,
  ) {
    this.text = Utf8Text.from(source);
    this.saved = source === savedSource ? this.text : Utf8Text.from(savedSource);
  }
  private assertOpen() {
    if (this.closed) throw new Error('文档会话已经关闭。');
  }
  status(): DocumentStatus {
    this.assertOpen();
    return {
      hasDerivedEdits: this.hasDerivedEdits,
      documentId: this.documentId,
      revision: this.revision,
      byteLength: this.text.byteLength,
      dirty: !this.text.equals(this.saved),
      canUndo: this.cursor > 0,
      canRedo: this.cursor < this.history.length,
      selection: this.selection ? { ...this.selection } : undefined,
    };
  }
  readWindow(revision: number, fromByte: number, toByte: number): DocumentWindow {
    this.assertOpen();
    if (revision !== this.revision) throw new DocumentConflictError();
    return { documentId: this.documentId, revision, ...this.text.readWindow(fromByte, toByte) };
  }
  captureSnapshot(): MarkdownSnapshot {
    this.assertOpen();
    return Object.freeze({ documentId: this.documentId, revision: this.revision, text: this.text });
  }
  markSaved(snapshot: MarkdownSnapshot) {
    this.assertOpen();
    if (snapshot.documentId !== this.documentId)
      throw new DocumentConflictError('保存快照属于另一份文档。');
    this.saved = snapshot.text;
    return this.status();
  }
  applyTransaction(transaction: DocumentTransaction): DocumentStatus {
    this.assertOpen();
    if (!transaction.operationId || !transaction.historyGroupId)
      throw new Error('事务缺少操作身份或历史分组。');
    const fingerprint = JSON.stringify(transaction);
    const acknowledged = this.acknowledgements.get(transaction.operationId);
    if (acknowledged) {
      if (acknowledged.fingerprint !== fingerprint)
        throw new DocumentConflictError('同一操作身份不能携带不同内容。');
      return { ...acknowledged.status };
    }
    if (transaction.baseRevision !== this.revision) throw new DocumentConflictError();
    const edits = [...transaction.edits].sort(
      (a, b) => a.fromByte - b.fromByte || a.toByte - b.toByte,
    );
    for (let i = 0; i < edits.length; i++) {
      const edit = edits[i];
      if (
        !this.text.isBoundary(edit.fromByte) ||
        !this.text.isBoundary(edit.toByte) ||
        edit.fromByte > edit.toByte ||
        (i > 0 && (edit.fromByte < edits[i - 1].toByte || edit.fromByte === edits[i - 1].fromByte))
      )
        throw new RangeError('事务范围重叠、越界或切开了字符。');
    }
    let next = this.text;
    for (const edit of edits.reverse())
      next = next.replace(edit.fromByte, edit.toByte, edit.insertedText);
    const validateSelection = (selection: DocumentSelection | undefined, text: Utf8Text) => {
      if (
        selection &&
        (!text.isBoundary(selection.anchorByte) || !text.isBoundary(selection.headByte))
      )
        throw new RangeError('选区不是有效的全局正文位置。');
    };
    validateSelection(transaction.selectionBefore, this.text);
    validateSelection(transaction.selectionAfter, next);
    if (!next.equals(this.text)) {
      this.hasDerivedEdits ||= !!transaction.derived;
      if (!transaction.derived) {
      this.history.length = this.cursor;
      const previous = this.history[this.cursor - 1];
      if (!this.historyBarrier && previous?.group === transaction.historyGroupId) {
        previous.after = next;
        previous.selectionAfter = transaction.selectionAfter;
      } else {
        this.history.push({
          group: transaction.historyGroupId,
          before: this.text,
          after: next,
          selectionBefore: transaction.selectionBefore,
          selectionAfter: transaction.selectionAfter,
        });
        this.cursor++;
      }
      }
      this.text = next;
      this.revision++;
      this.historyBarrier = false;
    }
    this.selection = transaction.selectionAfter;
    const status = this.status();
    this.acknowledgements.set(transaction.operationId, { fingerprint, status });
    this.acknowledgementBytes += fingerprint.length * 2;
    // 超出确认窗口的重试仍会被 baseRevision 拒绝，绝不重复应用。
    while (this.acknowledgements.size > 128 || this.acknowledgementBytes > 4 * 1024 * 1024) {
      const oldest = this.acknowledgements.keys().next().value!;
      this.acknowledgementBytes -= this.acknowledgements.get(oldest)!.fingerprint.length * 2;
      this.acknowledgements.delete(oldest);
    }
    return status;
  }
  undo(baseRevision = this.revision): DocumentStatus {
    this.assertOpen();
    if (baseRevision !== this.revision) throw new DocumentConflictError();
    this.historyBarrier = true;
    if (this.cursor > 0) {
      const entry = this.history[--this.cursor];
      this.text = entry.before;
      this.selection = entry.selectionBefore;
      this.revision++;
    }
    return this.status();
  }
  /** 浏览器适配器发布已经计算好的候选树，历史仍只有一个用户操作。 */
  publishPrepared(base: MarkdownSnapshot, next: Utf8Text, group: string, selection?: DocumentSelection) {
    this.assertOpen();
    if (base.documentId !== this.documentId || base.revision !== this.revision) throw new DocumentConflictError();
    if (!next.equals(this.text)) {
      this.history.length = this.cursor;
      this.history.push({ group, before: this.text, after: next, selectionBefore: this.selection, selectionAfter: selection });
      this.cursor++; this.text = next; this.revision++; this.historyBarrier = true; this.selection = selection;
    }
    return this.status();
  }
  redo(baseRevision = this.revision): DocumentStatus {
    this.assertOpen();
    if (baseRevision !== this.revision) throw new DocumentConflictError();
    this.historyBarrier = true;
    if (this.cursor < this.history.length) {
      const entry = this.history[this.cursor++];
      this.text = entry.after;
      this.selection = entry.selectionAfter;
      this.revision++;
    }
    return this.status();
  }
  close() {
    this.history = [];
    this.acknowledgements.clear();
    this.text = this.saved = Utf8Text.from('');
    this.closed = true;
  }
}
