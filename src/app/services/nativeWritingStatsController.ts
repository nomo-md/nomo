import type { MarkdownDocumentPort } from '../../lib/markdown-document/port';
import type { StatsRange, WritingStatsState } from '../../lib/outline/writingStatsProtocol';

export interface NativeStatsDocument {
  documentId: string;
  revision: number;
  port: MarkdownDocumentPort;
  selection: StatsRange | null;
  ready: (signal: AbortSignal) => Promise<boolean>;
}

/** No document strings or Worker are reachable from the desktop statistics path. */
export function createNativeWritingStatsController(publish: (state: WritingStatsState) => void) {
  let input: NativeStatsDocument | null = null;
  let state: WritingStatsState = { status: 'pending', full: null, selected: null };
  let range: StatsRange | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: AbortController | undefined;
  let requestId = 0;
  let selectionVersion = 0;
  let fullRevision = -1;
  let suspended = false;
  let destroyed = false;
  function cancel() {
    clearTimeout(timer); timer = undefined;
    abort?.abort(); abort = undefined;
    if (input?.port.cancelWritingStats) {
      void input.port.cancelWritingStats(requestId).catch(error => console.warn('Cancel writing statistics failed', error));
    }
  }
  function schedule(delay: number) {
    clearTimeout(timer);
    state = { ...state, status: 'pending' }; publish(state);
    timer = setTimeout(() => { timer = undefined; void dispatch(); }, delay);
  }
  async function dispatch() {
    if (!input || suspended || destroyed) return;
    const current = input;
    const id = ++requestId;
    const version = selectionVersion;
    abort?.abort(); const task = new AbortController(); abort = task;
    const isCurrent = () => !task.signal.aborted && !suspended && !destroyed && input === current && id === requestId && version === selectionVersion;
    try {
      if (!await current.ready(task.signal) || !isCurrent()) return;
      if (!current.port.writingStats) throw new Error('Native writing statistics unavailable');
      const result = await current.port.writingStats({ documentId: current.documentId, revision: current.revision,
        requestId: id, selectionVersion: version, selection: range ? { from: range.from, to: range.to } : null });
      if (!isCurrent() || result.documentId !== current.documentId || result.revision !== current.revision ||
        result.requestId !== id || result.selectionVersion !== version) return;
      fullRevision = current.revision;
      state = { status: 'ready', full: result.full, selected: result.selected }; publish(state);
    } catch (error) {
      if (!isCurrent()) return;
      if ((error as { code?: string })?.code === 'stats-cancelled') return;
      console.warn('Native writing statistics failed', error);
      state = { ...state, status: 'error', selected: null }; publish(state);
    }
  }
  return {
    setDocument(next: NativeStatsDocument) {
      if (destroyed) return;
      if (!suspended && input?.documentId === next.documentId && input.revision === next.revision) return;
      const sameDocument = input?.documentId === next.documentId;
      if (!sameDocument) fullRevision = -1;
      cancel(); input = next; range = next.selection?.from !== next.selection?.to ? next.selection : null;
      suspended = false; selectionVersion++;
      state = { status: 'pending', full: sameDocument ? state.full : null, selected: null };
      // App's content scheduler already applied its 500 ms debounce.
      schedule(0);
    },
    setSelection(next: StatsRange | null) {
      if (!input || destroyed || suspended || state.status === 'error') return;
      const selected = next && next.from !== next.to ? next : null;
      if (selected?.from === range?.from && selected?.to === range?.to) return;
      cancel(); range = selected; selectionVersion++;
      if (!range && state.full && fullRevision === input.revision) { state = { ...state, status: 'ready', selected: null }; publish(state); }
      else schedule(120);
    },
    invalidate() { cancel(); input = null; range = null; state = { status: 'pending', full: null, selected: null }; publish(state); },
    suspend() { cancel(); suspended = true; range = null; state = { ...state, selected: null }; publish(state); },
    destroy() { cancel(); destroyed = true; input = null; },
  };
}
