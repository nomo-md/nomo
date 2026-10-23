import { DocumentConflictError } from './protocol';
import type { MarkdownDocumentSession, MarkdownSnapshot } from './session';
import type { MarkdownDocumentPort, SnapshotReference } from './port';
import { searchMarkdownSnapshot } from './tasks';
import { Utf8Text, utf8Length } from './utf8Text';
import { SEARCH_PAGE_SIZE, type MarkdownSearchPort, type SearchQuery, type ReplaceQuery,
  type SearchProgress, type SearchMatch, type ReplaceResult } from './searchProtocol';

interface Task {
  query: SearchQuery; progress: SearchProgress; snapshot: MarkdownSnapshot; abort: AbortController;
  prepared?: { request: ReplaceQuery; tree: Utf8Text; count: number; changed: number; points: number[] };
}
/** 仅浏览器内存端使用 JS 扫描；桌面端不会以此作为异常降级路径。 */
export function createMemorySearchPort(session: (id: string) => MarkdownDocumentSession) {
  const tasks = new Map<string, Task>(), listeners = new Set<(progress: SearchProgress) => void>();
  const completed = new Map<string, ReplaceResult>();
  const emit = (task: Task, update: Partial<SearchProgress>) => {
    task.progress = { ...task.progress, ...update };
    for (const listener of listeners) listener(task.progress);
  };
  const cancel = (id: string) => {
    const task = tasks.get(id); if (!task) return;
    task.abort.abort(); tasks.delete(id); emit(task, { state: 'cancelled' });
  };
  const invalidate = (id: string) => { for (const [key, task] of tasks) if (task.query.documentId === id) cancel(key); };
  const get = (id: string) => {
    const task = tasks.get(id); if (!task) throw Object.assign(new Error('搜索结果已经失效'), { code: 'search-expired' });
    if (session(task.query.documentId).status().revision !== task.query.baseRevision) throw new DocumentConflictError();
    task.abort.signal.throwIfAborted(); return task;
  };
  const scan = (task: Task, onMatch: (match: SearchMatch, text: string) => boolean | void, report = true) => {
    const snapshot: SnapshotReference = { ...task.query, revision: task.snapshot.revision,
      snapshotId: task.progress.taskId, byteLength: task.snapshot.text.byteLength };
    const reader = { async readSnapshot(_: SnapshotReference, from: number, to: number) {
      return { ...task.snapshot.text.readWindow(from, to), documentId: snapshot.documentId, revision: snapshot.revision };
    } } as MarkdownDocumentPort;
    return searchMarkdownSnapshot(reader, snapshot, task.query.query, {
      ...task.query, signal: task.abort.signal, retainMatches: false,
      onMatch: (match, index) => onMatch({ index, fromByte: match.fromByte, toByte: match.toByte,
        preservedPrefix: match.preservedPrefix, preservedSuffix: match.preservedSuffix }, match.text),
      onProgress: progress => { if (report) emit(task, { scannedBytes: progress.scannedBytes, matchCount: progress.matches }); },
    });
  };
  const create = (query: SearchQuery, run: (task: Task) => Promise<void>) => {
    for (const key of tasks.keys()) cancel(key);
    const snapshot = session(query.documentId).captureSnapshot();
    if (snapshot.revision !== query.baseRevision) throw new DocumentConflictError();
    const identity = { requestId: query.requestId, taskId: crypto.randomUUID(), documentId: query.documentId, baseRevision: query.baseRevision };
    const task: Task = { query, snapshot, abort: new AbortController(), progress: { ...identity,
      state: 'queued', scannedBytes: 0, totalBytes: snapshot.text.byteLength, matchCount: 0 } };
    tasks.set(identity.taskId, task);
    void run(task).catch(error => { if (!task.abort.signal.aborted) emit(task, {
      state: 'failed', error: { code: 'memory-search-failed', message: String(error) } }); });
    return identity;
  };
  const port: MarkdownSearchPort = {
    async listenSearch(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    async startSearch(query) { return create(query, async task => {
      await scan(task, () => {}); task.abort.signal.throwIfAborted(); emit(task, { state: 'ready' });
    }); },
    async searchTaskProgress(id) { return get(id).progress; },
    async readSearchPage(id, fromIndex, limit) {
      const task = get(id), totalCount = task.progress.matchCount, matches: SearchMatch[] = [];
      if (task.progress.state !== 'ready') throw new Error('搜索尚未完成');
      limit = Math.min(SEARCH_PAGE_SIZE, Math.max(1, limit));
      await scan(task, match => { if (match.index >= fromIndex) matches.push(match); return matches.length < limit; }, false);
      task.progress.matchCount = totalCount; get(id);
      return { ...task.progress, totalCount, fromIndex, matches };
    },
    async prepareReplaceAll(request) { return create(request, async task => {
      const replacement = Utf8Text.from(request.replacement);
      let tree = task.snapshot.text, delta = 0, changed = 0, count = 0;
      const selected = request.selection;
      const sourcePoints = [...(selected ? [selected.anchorByte, selected.headByte] : []), ...(request.anchors ?? [])];
      const mapped: Array<number | undefined> = sourcePoints.map(() => undefined);
      await scan(task, (match, text) => {
        count++; if (text === request.replacement) return;
        const from = match.fromByte + utf8Length(match.preservedPrefix), to = match.toByte - utf8Length(match.preservedSuffix);
        for (let index = 0; index < sourcePoints.length; index++) {
          const point = sourcePoints[index]; if (mapped[index] !== undefined) continue;
          if (point < from) mapped[index] = point + delta;
          else if (point < to) {
            const before = selected && ((index === 0 && selected.anchorByte < selected.headByte) || (index === 1 && selected.headByte < selected.anchorByte));
            mapped[index] = from + delta + (before ? 0 : replacement.byteLength);
          }
        }
        tree = tree.replaceScalars(from + delta, to + delta, replacement);
        delta += replacement.byteLength - (to - from); changed++;
      });
      task.abort.signal.throwIfAborted();
      const points = sourcePoints.map((point, index) => {
        let result = mapped[index] ?? point + delta;
        while (result < tree.byteLength && !tree.isBoundary(result)) result++;
        return result;
      });
      task.prepared = { request, tree, changed, count, points };
      emit(task, { state: 'prepared', matchCount: count });
    }); },
    async commitReplaceAll(documentId, taskId, operationId) {
      const key = `${documentId}:${operationId}`, acknowledged = completed.get(key);
      if (acknowledged) { if (acknowledged.taskId !== taskId) throw new DocumentConflictError(); return acknowledged; }
      const task = get(taskId), prepared = task.prepared;
      if (!prepared || prepared.request.operationId !== operationId || task.query.documentId !== documentId) throw new DocumentConflictError();
      const selection = prepared.request.selection ? { anchorByte: prepared.points[0], headByte: prepared.points[1] } : undefined;
      const status = session(documentId).publishPrepared(task.snapshot, prepared.tree, operationId, selection);
      const result = { ...task.progress, status, count: prepared.count, changedCount: prepared.changed,
        anchors: prepared.points.slice(selection ? 2 : 0) };
      completed.set(key, result); while (completed.size > 128) completed.delete(completed.keys().next().value!);
      emit(task, { state: 'completed' }); return result;
    },
    async cancelTask(id) { cancel(id); }, async releaseTask(id) { cancel(id); },
  };
  return { port, invalidate };
}
