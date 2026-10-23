import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { MARKDOWN_SEARCH_EVENT, type MarkdownSearchPort } from './searchProtocol';
import { createMemorySearchPort } from './memorySearchPort';
import { MarkdownDocumentSession, type MarkdownSnapshot } from './session';
import type { DocumentStatus, DocumentTransaction, DocumentWindow } from './protocol';
import { DocumentConflictError } from './protocol';
import type { MarkdownEncoding } from '../services/storage';
import type { NativeStatsRequest, NativeStatsResponse } from '../outline/writingStatsProtocol';

export interface MarkdownSessionStatus extends DocumentStatus {
  encoding?: MarkdownEncoding;
  path?: string | null;
  persistenceWarning?: string | null;
}
export interface SnapshotReference {
  snapshotId: string;
  documentId: string;
  revision: number;
  byteLength: number;
}
export interface MarkdownDocumentPort extends MarkdownSearchPort {
  writingStats?(request: NativeStatsRequest): Promise<NativeStatsResponse>;
  cancelWritingStats?(requestId?: number): Promise<void>;
  openDocument(
    input: { path: string; fresh?: boolean } | { text: string } | { recoveryId: string },
  ): Promise<MarkdownSessionStatus>;
  status(documentId: string): Promise<MarkdownSessionStatus>;
  readWindow(
    documentId: string,
    revision: number,
    fromByte: number,
    toByte: number,
  ): Promise<DocumentWindow>;
  applyTransaction(
    documentId: string,
    transaction: DocumentTransaction,
  ): Promise<MarkdownSessionStatus>;
  undo(documentId: string, revision: number): Promise<MarkdownSessionStatus>;
  redo(documentId: string, revision: number): Promise<MarkdownSessionStatus>;
  captureSnapshot(documentId: string): Promise<SnapshotReference>;
  captureOriginal(documentId: string): Promise<SnapshotReference>;
  readSnapshot(
    snapshot: SnapshotReference,
    fromByte: number,
    toByte: number,
  ): Promise<DocumentWindow>;
  releaseSnapshot(snapshot: SnapshotReference): Promise<void>;
  saveSnapshot(
    snapshot: SnapshotReference,
    path: string,
    overwriteExternal?: boolean,
  ): Promise<MarkdownSessionStatus>;
  close(documentId: string, discard: boolean): Promise<void>;
  closeMany?(documents: Array<{ documentId: string; discard: boolean }>): Promise<void>;
  detach?(documentId: string): Promise<void>;
}

function native<T>(request: Record<string, unknown>): Promise<T> {
  return invoke<T>('markdown_document_command', { request }).catch((failure: unknown) => {
    if (failure instanceof Error) throw failure;
    const detail = failure as { code?: string; message?: string } | null;
    const message = detail?.message ?? String(failure);
    if (detail?.code === 'revision-conflict') throw new DocumentConflictError(message);
    throw Object.assign(new Error(message), { code: detail?.code });
  });
}
export function createNativeMarkdownPort(): MarkdownDocumentPort {
  return {
    listenSearch: listener => listen(MARKDOWN_SEARCH_EVENT, event => listener(event.payload as import('./searchProtocol').SearchProgress)),
    startSearch: request => native({ type: 'startSearch', ...request }),
    readSearchPage: (taskId, fromIndex, limit) => native({ type: 'readSearchPage', taskId, fromIndex, limit }),
    searchTaskProgress: taskId => native({ type: 'searchTaskProgress', taskId }),
    prepareReplaceAll: request => native({ type: 'prepareReplaceAll', ...request }),
    commitReplaceAll: (documentId, taskId, operationId) => native({ type: 'commitReplaceAll', documentId, taskId, operationId }),
    cancelTask: taskId => native({ type: 'cancelTask', taskId }),
    releaseTask: taskId => native({ type: 'releaseTask', taskId }),
    writingStats: (request) => native({ type: 'writingStats', ...request }),
    cancelWritingStats: (requestId) => native({ type: 'cancelWritingStats', requestId }),
    openDocument: (input) =>
      'recoveryId' in input
        ? native({ type: 'recover', documentId: input.recoveryId })
        : native({ type: 'open', ...input }),
    status: (documentId) => native({ type: 'status', documentId }),
    readWindow: (documentId, revision, fromByte, toByte) =>
      native({ type: 'readWindow', documentId, revision, fromByte, toByte }),
    applyTransaction: (documentId, transaction) =>
      native({ type: 'apply', transaction: { ...transaction, documentId } }),
    undo: (documentId, revision) => native({ type: 'undo', documentId, revision }),
    redo: (documentId, revision) => native({ type: 'redo', documentId, revision }),
    captureSnapshot: (documentId) => native({ type: 'captureSnapshot', documentId }),
    captureOriginal: (documentId) => native({ type: 'captureOriginal', documentId }),
    readSnapshot: (snapshot, fromByte, toByte) =>
      native({ type: 'readSnapshot', snapshotId: snapshot.snapshotId, fromByte, toByte }),
    releaseSnapshot: (snapshot) =>
      native({ type: 'releaseSnapshot', snapshotId: snapshot.snapshotId }),
    saveSnapshot: (snapshot, path, overwriteExternal = false) =>
      native({ type: 'saveSnapshot', snapshotId: snapshot.snapshotId, path, overwriteExternal }),
    close: (documentId, discard) => native({ type: 'close', documentId, discard }),
    closeMany: documents => native({ type: 'closeMany', documents }),
    detach: (documentId) => native({ type: 'detach', documentId }),
  };
}

/** 浏览器与契约测试使用相同接口；文件路径 IO 只由桌面端提供。 */
export function createMemoryMarkdownPort(): MarkdownDocumentPort {
  const sessions = new Map<string, MarkdownDocumentSession>();
  const snapshots = new Map<string, MarkdownSnapshot>();
  let sequence = 0;
  const originals = new Map<string, MarkdownSnapshot>();
  const session = (id: string) => {
    const result = sessions.get(id);
    if (!result) throw new Error('文档会话已关闭。');
    return result;
  };
  const snapshot = (id: string) => {
    const result = snapshots.get(id);
    if (!result) throw new Error('文档快照已释放。');
    return result;
  };
  const search = createMemorySearchPort(session);
  return {
    ...search.port,
    async openDocument(input) {
      if ('recoveryId' in input) return session(input.recoveryId).status();
      if (!('text' in input)) throw new Error('浏览器文档入口需要文件正文。');
      const id = `memory-md-${++sequence}`;
      const result = new MarkdownDocumentSession(id, input.text);
      sessions.set(id, result);
      originals.set(id, result.captureSnapshot());
      return result.status();
    },
    async status(id) {
      return session(id).status();
    },
    async readWindow(id, revision, fromByte, toByte) {
      return session(id).readWindow(revision, fromByte, toByte);
    },
    async applyTransaction(id, transaction) {
      const status = session(id).applyTransaction(transaction);
      search.invalidate(id); return status;
    },
    async undo(id, revision) {
      search.invalidate(id); return session(id).undo(revision);
    },
    async redo(id, revision) {
      search.invalidate(id); return session(id).redo(revision);
    },
    async captureSnapshot(id) {
      const result = session(id).captureSnapshot();
      const snapshotId = `memory-snapshot-${++sequence}`;
      snapshots.set(snapshotId, result);
      return {
        snapshotId,
        documentId: id,
        revision: result.revision,
        byteLength: result.text.byteLength,
      };
    },
    async captureOriginal(id) {
      session(id);
      const result = originals.get(id)!;
      const snapshotId = `memory-original-${++sequence}`;
      snapshots.set(snapshotId, result);
      return { snapshotId, documentId: id, revision: 0, byteLength: result.text.byteLength };
    },
    async readSnapshot(reference, fromByte, toByte) {
      const result = snapshot(reference.snapshotId);
      return {
        documentId: result.documentId,
        revision: result.revision,
        ...result.text.readWindow(fromByte, toByte),
      };
    },
    async releaseSnapshot(reference) {
      snapshots.delete(reference.snapshotId);
    },
    async saveSnapshot(reference) {
      return session(reference.documentId).markSaved(snapshot(reference.snapshotId));
    },
    async close(id, discard) {
      search.invalidate(id);
      const current = session(id);
      if (current.status().dirty && !discard) throw new Error('文档尚有未保存内容。');
      if ([...snapshots.values()].some((snapshot) => snapshot.documentId === id))
        throw new Error('文档仍有正在使用的快照。');
      current.close();
      sessions.delete(id);
      originals.delete(id);
    },
    async closeMany(documents) {
      for (const { documentId, discard } of documents) {
        if (session(documentId).status().dirty && !discard) throw new Error('组合文档尚有未保存内容。');
        if ([...snapshots.values()].some(snapshot => snapshot.documentId === documentId)) throw new Error('组合文档仍有正在使用的快照。');
      }
      for (const { documentId } of documents) { search.invalidate(documentId); session(documentId).close(); sessions.delete(documentId); originals.delete(documentId); }
    },
  };
}

/** 显式全文任务才可物化字符串；按返回范围推进，避免重复或遗漏窗口边界字符。 */
export async function readCompleteSnapshot(
  port: MarkdownDocumentPort,
  snapshot: SnapshotReference,
  signal?: AbortSignal,
): Promise<string> {
  const chunks: string[] = [];
  for (let from = 0; from < snapshot.byteLength; ) {
    signal?.throwIfAborted();
    const window = await port.readSnapshot(
      snapshot,
      from,
      Math.min(snapshot.byteLength, from + 256 * 1024),
    );
    if (
      window.documentId !== snapshot.documentId ||
      window.revision !== snapshot.revision ||
      window.fromByte !== from ||
      window.toByte <= from
    )
      throw new Error('完整快照读取返回了不连续或过期的窗口。');
    chunks.push(window.text);
    from = window.toByte;
  }
  signal?.throwIfAborted();
  return chunks.join('');
}
