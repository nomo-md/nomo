import type {
  StatsContext,
  StatsRange,
  StatsRequest,
  StatsResponse,
  WritingStatsState,
} from '../../lib/outline/writingStatsProtocol';

interface StatsWorker {
  onmessage: ((event: MessageEvent<StatsResponse>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(request: StatsRequest): void;
  terminate(): void;
}
interface DocumentInput {
  documentId: string;
  markdown: string;
  mode: 'source' | 'semantic';
  revision?: number;
  semanticSnapshot: () => Record<string, unknown> | null;
}

/** 管理单个 Worker 和最新请求；内容快照只在修订改变后、空闲时生成。 */
export function createWritingStatsController(
  onState: (state: WritingStatsState) => void,
  createWorker: () => StatsWorker = () =>
    new Worker(new URL('../../lib/outline/writingStats.worker.ts', import.meta.url), {
      type: 'module',
    }),
) {
  let worker: StatsWorker | null = null;
  let input: DocumentInput | null = null;
  let context: StatsContext = {
    documentId: '',
    contentRevision: 0,
    selectionRevision: 0,
    mode: 'semantic',
  };
  let state: WritingStatsState = { status: 'pending', full: null, selected: null };
  let range: StatsRange | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight: number | null = null;
  let nextRequestId = 0;
  let snapshotNeeded = false;
  let queued = false;
  let destroyed = false;
  let suspended = false;
  let sourceSnapshotRevision = -1;
  let semanticSnapshotRevision = -1;
  const publish = () => onState({ ...state });
  const cancelTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  const fail = () => {
    cancelTimer();
    queued = false;
    inFlight = null;
    worker?.terminate();
    worker = null;
    sourceSnapshotRevision = semanticSnapshotRevision = -1;
    state = { ...state, status: 'error', selected: null };
    publish();
  };
  function dispatch() {
    if (destroyed || suspended || !input || state.status === 'error') return;
    if (inFlight !== null) {
      queued = true;
      return;
    }
    try {
      if (!worker) {
        worker = createWorker();
        const currentWorker = worker;
        worker.onerror = () => {
          if (!destroyed && worker === currentWorker) fail();
        };
        worker.onmessage = ({ data }) => {
          if (destroyed || data.requestId !== inFlight) return;
          inFlight = null;
          const sameDocument =
            data.context.documentId === context.documentId &&
            data.context.contentRevision === context.contentRevision &&
            data.context.mode === context.mode;
          const current =
            sameDocument && data.context.selectionRevision === context.selectionRevision;
          if (sameDocument && data.error) {
            fail();
            return;
          }
          if (current) {
            state = {
              status: 'ready',
              full: data.full ?? state.full,
              selected: data.selected ?? null,
            };
            publish();
          } else if (sameDocument && data.full && !data.error) {
            // Full statistics are independent of the obsolete selection revision.
            state = { ...state, status: 'ready', full: data.full };
            publish();
          }
          if (!range && !snapshotNeeded && state.full) {
            cancelTimer();
            queued = false;
          }
          if (queued) {
            queued = false;
            dispatch();
          }
        };
      }
      const request: StatsRequest = {
        requestId: ++nextRequestId,
        context: { ...context },
        selection: range ? { ...range } : null,
      };
      if (snapshotNeeded) {
        request.snapshot = {};
        if (sourceSnapshotRevision !== context.contentRevision) {
          request.snapshot.markdown = input.markdown;
          sourceSnapshotRevision = context.contentRevision;
        }
        if (context.mode === 'semantic' && semanticSnapshotRevision !== context.contentRevision) {
          const doc = input.semanticSnapshot();
          // 单个字符串的跨线程传输避免逐个复制大文档中的节点对象；选区请求不走此分支。
          request.snapshot.semanticDoc = doc ? JSON.stringify(doc) : null;
          semanticSnapshotRevision = context.contentRevision;
        }
        snapshotNeeded = false;
      }
      inFlight = request.requestId;
      worker.postMessage(request);
    } catch {
      fail();
    }
  }
  function schedule() {
    cancelTimer();
    timer = setTimeout(() => {
      timer = null;
      dispatch();
    }, 120);
  }
  return {
    setDocument(next: DocumentInput) {
      if (destroyed) return;
      if (
        input?.documentId === next.documentId &&
        input.markdown === next.markdown &&
        input.mode === next.mode &&
        input.revision === next.revision
      )
        return;
      const sameDocument = input?.documentId === next.documentId;
      const sameContent =
        !suspended &&
        sameDocument &&
        input?.markdown === next.markdown &&
        input.revision === next.revision;
      cancelTimer();
      queued = false;
      input = next;
      range = null;
      suspended = false;
      context = {
        documentId: next.documentId,
        contentRevision: context.contentRevision + (sameContent ? 0 : 1),
        selectionRevision: context.selectionRevision + 1,
        mode: next.mode,
      };
      snapshotNeeded =
        sourceSnapshotRevision !== context.contentRevision ||
        (next.mode === 'semantic' && semanticSnapshotRevision !== context.contentRevision);
      state = {
        status: sameDocument && state.full ? 'ready' : 'pending',
        full: sameDocument ? state.full : null,
        selected: null,
      };
      publish();
      schedule();
    },
    setSelection(selection: StatsRange | null) {
      if (destroyed || suspended || !input || state.status === 'error') return;
      range = selection && selection.from !== selection.to ? selection : null;
      context = { ...context, selectionRevision: context.selectionRevision + 1 };
      queued = false;
      cancelTimer();
      if (!range) {
        state = { ...state, selected: null };
        publish();
        if (snapshotNeeded || !state.full) schedule();
      } else schedule();
    },
    invalidate() {
      cancelTimer();
      queued = false;
      input = null;
      range = null;
      context = { ...context, contentRevision: context.contentRevision + 1 };
      state = { status: 'pending', full: null, selected: null };
      publish();
    },
    suspend() {
      cancelTimer();
      queued = false;
      range = null;
      suspended = true;
      context = {
        ...context,
        contentRevision: context.contentRevision + 1,
        selectionRevision: context.selectionRevision + 1,
      };
      state = { ...state, selected: null };
      publish();
    },
    destroy() {
      destroyed = true;
      cancelTimer();
      queued = false;
      worker?.terminate();
      worker = null;
    },
  };
}
