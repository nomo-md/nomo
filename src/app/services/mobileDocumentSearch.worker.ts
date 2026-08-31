import { createMobileMemoryScanner } from './mobileDocumentSearch';

let scan: ReturnType<typeof createMobileMemoryScanner> | undefined;
self.onmessage = (event: MessageEvent<{ query: string; chunk: string; eof: boolean }>) => {
  scan ??= createMobileMemoryScanner(event.data.query);
  const snippet = scan(event.data.chunk, event.data.eof);
  self.postMessage({ snippet, done: Boolean(snippet) || event.data.eof });
};
