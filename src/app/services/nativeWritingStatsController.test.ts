import { afterEach, describe, expect, it, vi } from 'vitest';
import { createWritingStatsController } from './writingStatsController';
import type { NativeStatsRequest, NativeStatsResponse, WritingStatsState } from '../../lib/outline/writingStatsProtocol';
import type { MarkdownDocumentPort } from '../../lib/markdown-document/port';

afterEach(() => vi.useRealTimers());
const full = { chars: 10, words: 2, visibleChars: 4, lines: 1, headings: 0, readingMinutes: 1 };
function setup() {
  vi.useFakeTimers();
  const calls: Array<{ request: NativeStatsRequest; resolve: (value: NativeStatsResponse) => void; reject: (reason: unknown) => void }> = [];
  const port = { writingStats: vi.fn(request => new Promise<NativeStatsResponse>((resolve, reject) => calls.push({ request, resolve, reject }))),
    cancelWritingStats: vi.fn(async () => {}) } as unknown as MarkdownDocumentPort;
  const worker = vi.fn(() => { throw new Error('Desktop must not create a statistics Worker'); });
  const states: WritingStatsState[] = [];
  const controller = createWritingStatsController(state => states.push(state), worker);
  const document = { documentId: 'native-a', revision: 3, port, selection: null, ready: async () => true };
  function reply(index: number, selected: typeof full | null = null) {
    const call = calls[index]; call.resolve({ ...call.request, full, selected, metrics: {
      elapsedMs: 1, snapshotBytesRead: 0, workspaceBoundBytes: 1, indexBytes: 1, cacheHit: true } });
  }
  return { calls, port, worker, states, controller, document, reply };
}
describe('native writing statistics', () => {
  it('sends identities and UTF-8 ranges without creating a Worker or capturing text', async () => {
    const s = setup(); s.controller.setNativeDocument(s.document);
    await vi.advanceTimersByTimeAsync(0);
    expect(s.calls[0].request).toEqual({ documentId: 'native-a', revision: 3, requestId: 1, selectionVersion: 1, selection: null });
    s.reply(0); await vi.advanceTimersByTimeAsync(0);
    s.controller.setSelection({ from: 9, to: 2, sourceCoordinates: 'utf8' });
    await vi.advanceTimersByTimeAsync(119); expect(s.calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1); expect(s.calls[1].request.selection).toEqual({ from: 9, to: 2 });
    expect(s.worker).not.toHaveBeenCalled(); s.controller.destroy();
  });
  it('cancels switched documents and ignores late results and errors', async () => {
    const s = setup(); s.controller.setNativeDocument(s.document); await vi.advanceTimersByTimeAsync(0);
    s.controller.setNativeDocument({ ...s.document, documentId: 'native-b' }); await vi.advanceTimersByTimeAsync(0);
    s.reply(0, full); await vi.advanceTimersByTimeAsync(0); expect(s.states.at(-1)?.full).toBeNull();
    s.reply(1); await vi.advanceTimersByTimeAsync(0); expect(s.states.at(-1)?.status).toBe('ready');
    expect(s.port.cancelWritingStats).toHaveBeenCalledWith(1); s.controller.destroy();
  });
  it('does not reuse totals from an older revision when a selection collapses', async () => {
    const s = setup(); s.controller.setNativeDocument(s.document); await vi.advanceTimersByTimeAsync(0); s.reply(0); await vi.advanceTimersByTimeAsync(0);
    s.controller.setNativeDocument({ ...s.document, revision: 4 }); await vi.advanceTimersByTimeAsync(0);
    s.controller.setSelection({ from: 1, to: 5 }); s.controller.setSelection(null);
    await vi.advanceTimersByTimeAsync(120); expect(s.calls.at(-1)?.request.revision).toBe(4);
    s.controller.destroy(); s.reply(s.calls.length - 1); await vi.advanceTimersByTimeAsync(0);
    expect(s.states.at(-1)?.status).toBe('pending');
  });
  it('waits for passive readiness, and suspension prevents the delayed request', async () => {
    const s = setup(); let resolve!: (ready: boolean) => void;
    s.controller.setNativeDocument({ ...s.document, ready: () => new Promise(r => { resolve = r; }) });
    await vi.advanceTimersByTimeAsync(0); s.controller.suspend(); resolve(true);
    await vi.advanceTimersByTimeAsync(0); expect(s.calls).toHaveLength(0); s.controller.destroy();
  });
  it('resumes the same revision after an unchanged inline draft settles', async () => {
    const s = setup(); s.controller.setNativeDocument(s.document);
    await vi.advanceTimersByTimeAsync(0); s.reply(0); await vi.advanceTimersByTimeAsync(0);
    s.controller.suspend();
    s.controller.setNativeDocument({ ...s.document, selection: { from: 9, to: 2, sourceCoordinates: 'utf8' } });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.calls[1].request.selection).toEqual({ from: 9, to: 2 });
    s.reply(1, full); await vi.advanceTimersByTimeAsync(0);
    expect(s.states.at(-1)).toEqual({ status: 'ready', full, selected: full });
    s.controller.destroy();
  });
});
