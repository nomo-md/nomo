import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createMobileDocumentSearch,
  nativeMobileSearchPort,
  scanMobileMemory,
  type MobileSearchEvent,
  type MobileSearchPort,
  type MobileSearchSnapshot,
} from './mobileDocumentSearch';

function harness() {
  let deliver: (event: MobileSearchEvent) => void = () => undefined;
  const stop = vi.fn();
  const port: MobileSearchPort = {
    listen: vi.fn(async (callback) => {
      deliver = callback;
      return stop;
    }),
    start: vi.fn(async () => undefined),
    cancel: vi.fn(async () => undefined),
  };
  let state: MobileSearchSnapshot;
  const search = createMobileDocumentSearch(port, (next) => {
    state = next;
  });
  return {
    port,
    search,
    stop,
    get state() {
      return state;
    },
    deliver: (event: MobileSearchEvent) => deliver(event),
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('mobile document search', () => {
  it('sends only bounded unsaved chunks to the worker and terminates it on close', async () => {
    class WorkerStub {
      static current: WorkerStub;
      postMessage = vi.fn();
      terminate = vi.fn();
      onmessage?: (event: { data: { snippet: null; done: boolean } }) => void;
      constructor() {
        WorkerStub.current = this;
      }
    }
    vi.stubGlobal('Worker', WorkerStub);
    const abort = new AbortController();
    const promise = nativeMobileSearchPort.scanMemory!('x'.repeat(100_000), 'tail', abort.signal);
    const worker = WorkerStub.current;
    expect(worker.postMessage).toHaveBeenCalledOnce();
    expect(worker.postMessage.mock.calls[0][0].chunk.length).toBe(32768);
    worker.onmessage?.({ data: { snippet: null, done: false } });
    expect(worker.postMessage).toHaveBeenCalledTimes(2);
    abort.abort();
    expect(await promise).toBeNull();
    expect(worker.terminate).toHaveBeenCalledOnce();
    worker.onmessage?.({ data: { snippet: null, done: false } });
    expect(worker.postMessage).toHaveBeenCalledTimes(2);
  });

  it('debounces 200 sources without requesting full document contents', async () => {
    vi.useFakeTimers();
    const h = harness();
    const docs = Array.from({ length: 200 }, (_, i) => ({
      key: `doc-${i}`,
      version: 1,
      path: `${i}.md`,
    }));
    h.search.search('first', true, docs);
    await vi.advanceTimersByTimeAsync(200);
    h.search.search('second', true, docs);
    await vi.advanceTimersByTimeAsync(299);
    expect(h.port.start).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(h.port.start).toHaveBeenCalledOnce();
    expect(vi.mocked(h.port.start).mock.calls[0][0].sources).toHaveLength(200);
    expect(vi.mocked(h.port.start).mock.calls[0][0].query).toBe('second');
    h.search.dispose();
  });

  it('cancels on query change, close and unmount and rejects stale versions/results', async () => {
    vi.useFakeTimers();
    const h = harness();
    const docs = [{ key: 'one', version: 2, path: 'one.md' }];
    h.search.search('one', true, docs);
    await vi.advanceTimersByTimeAsync(300);
    const firstId = vi.mocked(h.port.start).mock.calls[0][0].taskId;
    h.search.search('two', true, docs);
    expect(h.port.cancel).toHaveBeenCalledWith(firstId);
    await vi.advanceTimersByTimeAsync(300);
    const nextId = vi.mocked(h.port.start).mock.calls[1][0].taskId;
    const event: MobileSearchEvent = {
      taskId: firstId,
      key: 'one',
      version: 2,
      status: 'result',
      error: 'read-failed',
      completed: 1,
      total: 1,
    };
    h.deliver(event);
    h.deliver({ ...event, taskId: nextId, version: 1 });
    expect(h.state.results.size).toBe(0);
    h.deliver({ ...event, taskId: nextId });
    expect(h.state.results.get('one')?.error).toBe('read-failed');
    h.search.search('two', false, docs);
    expect(h.port.cancel).toHaveBeenCalledWith(nextId);
    expect(h.state.phase).toBe('idle');
    h.search.dispose();
    expect(h.stop).toHaveBeenCalledOnce();
  });

  it('searches empty/dirty Markdown in memory, and dirty TXT through its session revision', async () => {
    vi.useFakeTimers();
    const h = harness();
    h.search.search('unsaved', true, [
      { key: 'empty', version: 1, path: 'empty.md', content: '' },
      { key: 'dirty', version: 3, path: 'dirty.md', content: 'unsaved changes' },
      { key: 'text', version: 7, path: 'large.txt', sessionId: 'session-1' },
    ]);
    await vi.runAllTimersAsync();
    expect(vi.mocked(h.port.start).mock.calls[0][0].sources).toEqual([
      { key: 'text', version: 7, path: 'large.txt', sessionId: 'session-1' },
    ]);
    expect(h.state.results.get('dirty')?.snippet?.text).toBe('unsaved changes');
    expect(h.state.results.get('empty')?.snippet).toBeNull();
    h.search.dispose();
  });

  it('does not start after disposal while event subscription is pending', async () => {
    vi.useFakeTimers();
    let resolve!: (stop: () => void) => void;
    const stop = vi.fn();
    const port: MobileSearchPort = {
      listen: () =>
        new Promise((done) => {
          resolve = done;
        }),
      start: vi.fn(async () => undefined),
      cancel: vi.fn(async () => undefined),
    };
    const search = createMobileDocumentSearch(port, () => undefined);
    search.search('query', true, [{ key: 'file', path: 'a.md', version: 1 }]);
    await vi.advanceTimersByTimeAsync(300);
    search.dispose();
    resolve(stop);
    await vi.runAllTimersAsync();
    expect(port.start).not.toHaveBeenCalled();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('finds Unicode text across chunks and keeps at most 160 original characters', async () => {
    const text = 'x'.repeat(32766) + 'Ａé中😀XYZ' + 'tail'.repeat(100);
    const snippet = await scanMobileMemory(text, 'ae\u0301中😀xyz', () => false);
    expect(snippet).not.toBeNull();
    expect(Array.from(snippet!.text).length).toBeLessThanOrEqual(160);
    expect(snippet!.text.slice(snippet!.highlightStart, snippet!.highlightEnd)).toBe('Ａé中😀XYZ');
    expect(await scanMobileMemory(text, 'XYZ', () => true)).toBeNull();
  });
});
