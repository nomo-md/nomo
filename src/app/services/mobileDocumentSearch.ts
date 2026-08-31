/** Mobile drawer search keeps only snippets. Open Markdown uses its existing immutable string. */
export interface MobileSnippet {
  text: string;
  highlightStart: number;
  highlightEnd: number;
  leadingEllipsis: boolean;
  trailingEllipsis: boolean;
}
export interface MobileSearchDocument {
  key: string;
  version: number;
  path: string;
  sessionId?: string;
  content?: string;
}
export interface MobileSearchEvent {
  taskId: string;
  key?: string;
  version?: number;
  status: 'running' | 'result' | 'completed' | 'cancelled';
  snippet?: MobileSnippet | null;
  error?: string | null;
  completed: number;
  total: number;
}
export interface MobileSearchPort {
  scanMemory?(text: string, query: string, signal: AbortSignal): Promise<MobileSnippet | null>;
  listen(callback: (event: MobileSearchEvent) => void): Promise<() => void>;
  start(request: {
    taskId: string;
    query: string;
    sources: Omit<MobileSearchDocument, 'content'>[];
  }): Promise<void>;
  cancel(taskId: string): Promise<void>;
}
export interface MobileSearchSnapshot {
  phase: 'idle' | 'waiting' | 'searching' | 'completed' | 'failed';
  results: Map<string, { snippet?: MobileSnippet | null; error?: string | null }>;
  completed: number;
  total: number;
}
export const emptyMobileSearch = (): MobileSearchSnapshot => ({
  phase: 'idle',
  results: new Map(),
  completed: 0,
  total: 0,
});
export const normalizeMobileSearch = (value: string) => value.toLowerCase().normalize('NFKD');

export function createMobileMemoryScanner(query: string) {
  const needle = normalizeMobileSearch(query.trim());
  const keep = Array.from(needle).length + 160;
  let carry = '';
  let discarded = false;
  return (chunk: string, eof: boolean): MobileSnippet | null => {
    if (!needle) return null;
    const window = carry + chunk;
    const normalized = normalizeMobileSearch(window);
    const match = normalized.indexOf(needle);
    const chars = Array.from(window);
    if (match >= 0) {
      let normalizedOffset = 0;
      let from = -1;
      let to = 0;
      for (let index = 0; index < chars.length; index += 1) {
        const length = normalizeMobileSearch(chars[index]).length;
        if (from < 0 && normalizedOffset + length > match) from = index;
        normalizedOffset += length;
        if (normalizedOffset >= match + needle.length) {
          to = index + 1;
          break;
        }
      }
      if (from >= 0 && (eof || chars.length - to >= 54)) {
        const start = Math.max(0, from - 34);
        const end = Math.min(chars.length, start + 160);
        return {
          text: chars.slice(start, end).join(''),
          highlightStart: chars.slice(start, Math.min(from, end)).join('').length,
          highlightEnd: chars.slice(start, Math.min(to, end)).join('').length,
          leadingEllipsis: discarded || start > 0,
          trailingEllipsis: !eof || end < chars.length,
        };
      }
    }
    discarded ||= chars.length > keep;
    carry = chars.slice(-keep).join('');
    return null;
  };
}

export async function scanMobileMemory(
  text: string,
  query: string,
  cancelled: () => boolean,
): Promise<MobileSnippet | null> {
  const scan = createMobileMemoryScanner(query);
  for (let offset = 0; offset < text.length; offset += 32768) {
    if (cancelled()) return null;
    const result = scan(text.slice(offset, offset + 32768), offset + 32768 >= text.length);
    if (result) return result;
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
  return null;
}

export function createMobileDocumentSearch(
  port: MobileSearchPort,
  publish: (state: MobileSearchSnapshot) => void,
) {
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let activeId: string | undefined;
  let unlisten: (() => void) | undefined;
  let disposed = false;
  let current: MobileSearchSnapshot = emptyMobileSearch();
  let documents = new Map<string, MobileSearchDocument>();
  let memoryCompleted = 0;
  let backendCompleted = 0;
  let backendDone = true;
  let memoryDone = true;
  let memoryAbort = new AbortController();
  const update = () => {
    current.completed = memoryCompleted + backendCompleted;
    if (memoryDone && backendDone && current.phase === 'searching') current.phase = 'completed';
    publish({ ...current, results: new Map(current.results) });
  };
  const ready = port
    .listen((event) => {
      if (disposed || event.taskId !== activeId) return;
      if (event.key) {
        const document = documents.get(event.key);
        if (!document || document.version !== event.version) return;
        current.results.set(event.key, { snippet: event.snippet, error: event.error });
      }
      backendCompleted = event.completed;
      if (event.status === 'completed' || event.status === 'cancelled') backendDone = true;
      update();
    })
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
  // Retain rejection for start(), without creating an unhandled promise while the drawer is closed.
  void ready.catch(() => undefined);

  function cancel() {
    generation += 1;
    memoryAbort.abort();
    memoryAbort = new AbortController();
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    if (activeId)
      void port.cancel(activeId).catch(() => console.warn('Mobile search cancellation failed'));
    activeId = undefined;
  }

  async function start(query: string, items: MobileSearchDocument[], run: number) {
    const stale = () => disposed || run !== generation;
    try {
      await ready;
      if (stale()) return;
      const taskId = `mobile-${crypto.randomUUID()}`;
      activeId = taskId;
      current.phase = 'searching';
      const memory = items.filter((item) => item.content !== undefined);
      const sources = items
        .filter((item) => item.content === undefined)
        .map(({ content: _content, ...source }) => source);
      memoryDone = memory.length === 0;
      backendDone = sources.length === 0;
      update();
      if (sources.length) await port.start({ taskId, query, sources });
      if (stale()) {
        await port.cancel(taskId);
        return;
      }
      for (const document of memory) {
        const snippet = port.scanMemory
          ? await port.scanMemory(document.content!, query, memoryAbort.signal)
          : await scanMobileMemory(document.content!, query, stale);
        if (stale()) return;
        current.results.set(document.key, { snippet });
        memoryCompleted += 1;
        update();
      }
      memoryDone = true;
      update();
    } catch {
      if (!stale()) {
        current.phase = 'failed';
        update();
      }
    }
  }

  return {
    search(query: string, open: boolean, items: MobileSearchDocument[]) {
      cancel();
      if (disposed) return;
      documents = new Map(items.map((item) => [item.key, item]));
      current = emptyMobileSearch();
      memoryCompleted = 0;
      backendCompleted = 0;
      const normalized = query.trim();
      if (!open || !normalized) {
        publish(current);
        return;
      }
      if (new TextEncoder().encode(normalizeMobileSearch(normalized)).length > 65536) {
        current.phase = 'failed';
        publish(current);
        return;
      }
      current.phase = 'waiting';
      current.total = items.length;
      publish(current);
      const run = generation;
      timer = setTimeout(() => {
        timer = undefined;
        void start(normalized, items, run);
      }, 300);
    },
    dispose() {
      disposed = true;
      cancel();
      unlisten?.();
    },
  };
}

export const nativeMobileSearchPort: MobileSearchPort = {
  async scanMemory(text, query, signal) {
    if (signal.aborted || !text.length) return null;
    const worker = new Worker(new URL('./mobileDocumentSearch.worker.ts', import.meta.url), {
      type: 'module',
    });
    // The worker only receives one <=64KiB UTF-16 slice at a time, never the complete tab.
    return new Promise<MobileSnippet | null>((resolve, reject) => {
      let offset = 0;
      const cleanup = () => {
        worker.terminate();
        signal.removeEventListener('abort', abort);
      };
      const abort = () => {
        cleanup();
        resolve(null);
      };
      signal.addEventListener('abort', abort, { once: true });
      worker.onerror = () => {
        cleanup();
        reject(new Error('mobile-memory-search-failed'));
      };
      const send = () => {
        const chunk = text.slice(offset, offset + 32768);
        offset += chunk.length;
        worker.postMessage({ query, chunk, eof: offset >= text.length });
      };
      worker.onmessage = (
        event: MessageEvent<{ snippet: MobileSnippet | null; done: boolean }>,
      ) => {
        if (event.data.done) {
          cleanup();
          resolve(event.data.snippet);
        } else if (!signal.aborted) send();
      };
      send();
    });
  },
  async listen(callback) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<MobileSearchEvent>('nomo://mobile-search', (event) => callback(event.payload));
  },
  async start(request) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('start_mobile_document_search', { request });
  },
  async cancel(taskId) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('cancel_mobile_document_search', { taskId });
  },
};
