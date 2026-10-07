import { afterEach, describe, expect, it, vi } from 'vitest';
import { createWritingStatsController } from './writingStatsController';
import type {
  StatsRequest,
  StatsResponse,
  WritingStatsState,
} from '../../lib/outline/writingStatsProtocol';

afterEach(() => vi.useRealTimers());
const stats = { chars: 10, words: 2, visibleChars: 2, lines: 1, headings: 0, readingMinutes: 1 };
function setup() {
  vi.useFakeTimers();
  const requests: StatsRequest[] = [],
    states: WritingStatsState[] = [];
  const worker = {
    onmessage: null as ((event: MessageEvent<StatsResponse>) => void) | null,
    onerror: null as ((event: ErrorEvent) => void) | null,
    postMessage: vi.fn((request: StatsRequest) => requests.push(request)),
    terminate: vi.fn(),
  };
  const snapshot = vi.fn(() => ({ type: 'doc' }));
  const controller = createWritingStatsController(
    (state) => states.push(state),
    () => worker,
  );
  const document = {
    documentId: 'a',
    markdown: '正文',
    mode: 'semantic' as const,
    semanticSnapshot: snapshot,
  };
  const reply = (request: StatsRequest, selected = null as typeof stats | null) =>
    worker.onmessage?.({
      data: {
        requestId: request.requestId,
        context: request.context,
        full: stats,
        selected,
      },
    } as MessageEvent<StatsResponse>);
  return { controller, worker, requests, states, snapshot, document, reply };
}
describe('writing statistics scheduling', () => {
  it('reuses content indexes across mode changes and only adds a missing semantic snapshot', () => {
    const s = setup();
    s.controller.setDocument({ ...s.document, mode: 'source' });
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[1]);
    expect(s.requests[1].context.contentRevision).toBe(s.requests[0].context.contentRevision);
    expect(s.requests[1].snapshot?.markdown).toBeUndefined();
    expect(typeof s.requests[1].snapshot?.semanticDoc).toBe('string');
    s.controller.setDocument({ ...s.document, mode: 'source' });
    vi.advanceTimersByTime(120);
    s.reply(s.requests[2]);
    expect(s.requests[2].snapshot).toBeUndefined();
    expect(s.snapshot).toHaveBeenCalledOnce();
    s.controller.destroy();
  });
  it('debounces ranges for 120 ms and sends one snapshot per content revision', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    for (let i = 1; i < 20; i++) {
      s.controller.setSelection({ from: 0, to: i });
      vi.advanceTimersByTime(10);
    }
    expect(s.requests).toHaveLength(1);
    vi.advanceTimersByTime(120);
    expect(s.requests).toHaveLength(2);
    expect(s.requests[1].selection?.to).toBe(19);
    expect(s.requests[1].snapshot).toBeUndefined();
    expect(s.snapshot).toHaveBeenCalledTimes(1);
    s.controller.destroy();
  });
  it('drops old document and mode results and keeps only the latest queued document', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.controller.setDocument({ ...s.document, documentId: 'b' });
    vi.advanceTimersByTime(120);
    s.controller.setDocument({ ...s.document, documentId: 'c', mode: 'source' });
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    expect(s.requests).toHaveLength(2);
    expect(s.requests[1].context.documentId).toBe('c');
    expect(s.states.at(-1)?.full).toBeNull();
    s.reply(s.requests[1]);
    expect(s.states.at(-1)?.status).toBe('ready');
    s.controller.destroy();
  });
  it('clears selected statistics immediately and ignores stale selection responses', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    s.controller.setSelection({ from: 0, to: 2 });
    vi.advanceTimersByTime(120);
    s.controller.setSelection(null);
    expect(s.states.at(-1)?.selected).toBeNull();
    s.reply(s.requests[1], stats);
    expect(s.states.at(-1)?.selected).toBeNull();
    s.controller.destroy();
  });
  it('does not lose a pending full calculation when selection is cancelled', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.controller.setSelection(null);
    s.reply(s.requests[0]);
    vi.advanceTimersByTime(120);
    expect(s.requests).toHaveLength(1);
    expect(s.states.at(-1)?.full).toEqual(stats);
    s.controller.destroy();
  });
  it('cancels queued work on destruction and reports Worker failure without fallback', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.worker.onerror?.(new ErrorEvent('error'));
    expect(s.states.at(-1)?.status).toBe('error');
    expect(s.worker.terminate).toHaveBeenCalledOnce();
    s.controller.setSelection({ from: 0, to: 1 });
    vi.advanceTimersByTime(500);
    expect(s.requests).toHaveLength(1);
    s.controller.destroy();
    const t = setup();
    t.controller.setDocument(t.document);
    t.controller.destroy();
    vi.advanceTimersByTime(500);
    expect(t.requests).toHaveLength(0);
  });
  it('invalidates pending content while retaining valid old values until the stable revision arrives', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    s.controller.setSelection({ from: 1, to: 2 });
    vi.advanceTimersByTime(120);
    s.controller.suspend();
    s.controller.setDocument(s.document);
    s.reply(s.requests[1], stats);
    expect(s.states.at(-1)).toMatchObject({ status: 'ready', full: stats, selected: null });
    s.controller.setSelection({ from: 1, to: 3 });
    vi.advanceTimersByTime(500);
    expect(s.requests).toHaveLength(2);
    s.controller.setDocument({ ...s.document, markdown: '新正文', revision: 1 });
    vi.advanceTimersByTime(120);
    expect(s.requests[2].snapshot?.markdown).toBe('新正文');
    s.controller.destroy();
  });
  it('merges requests queued behind a busy Worker and retains the last valid selection value', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0], stats);
    s.controller.setSelection({ from: 1, to: 2 });
    vi.advanceTimersByTime(120);
    s.controller.setSelection({ from: 1, to: 3 });
    vi.advanceTimersByTime(120);
    s.controller.setSelection({ from: 1, to: 4 });
    vi.advanceTimersByTime(120);
    expect(s.states.at(-1)?.selected).toEqual(stats);
    s.reply(s.requests[1]);
    expect(s.requests).toHaveLength(3);
    expect(s.requests[2].selection).toEqual({ from: 1, to: 4 });
    expect(s.requests[2].snapshot).toBeUndefined();
    s.controller.destroy();
  });
  it('reports a current document parse failure even if its initial selection was cancelled', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.controller.setSelection(null);
    const request = s.requests[0];
    s.worker.onmessage?.({
      data: { requestId: request.requestId, context: request.context, error: 'Parse failed' },
    } as MessageEvent<StatsResponse>);
    expect(s.states.at(-1)?.status).toBe('error');
    s.controller.destroy();
  });
  it('finishes an initial full calculation after a mode switch with no selection', () => {
    const s = setup();
    s.controller.setDocument(s.document);
    vi.advanceTimersByTime(120);
    s.controller.setDocument({ ...s.document, mode: 'source' });
    s.controller.setSelection(null);
    vi.advanceTimersByTime(120);
    s.reply(s.requests[0]);
    expect(s.requests).toHaveLength(2);
    s.reply(s.requests[1]);
    expect(s.states.at(-1)?.status).toBe('ready');
    s.controller.destroy();
  });
});
