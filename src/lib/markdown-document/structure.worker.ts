import { MarkdownStructureScanner } from './structure';

const scanners = new Map<string, MarkdownStructureScanner>();
export type StructureWorkerRequest =
  | {
      id: number;
      type: 'start';
      job: string;
      revision: number;
      options?: ConstructorParameters<typeof MarkdownStructureScanner>[1];
    }
  | { id: number; type: 'chunk'; job: string; text: string; complete: boolean }
  | { id: number; type: 'cancel'; job: string };
self.onmessage = (event: MessageEvent<StructureWorkerRequest>) => {
  const request = event.data;
  try {
    if (request.type === 'start') {
      if (scanners.has(request.job)) throw new Error('结构扫描任务身份重复。');
      scanners.set(request.job, new MarkdownStructureScanner(request.revision, request.options));
      self.postMessage({ id: request.id, result: null });
    } else if (request.type === 'cancel') {
      scanners.delete(request.job);
      self.postMessage({ id: request.id, result: null });
    } else {
      const scanner = scanners.get(request.job);
      if (!scanner) throw new Error('结构扫描任务已取消。');
      const result = scanner.push(request.text, request.complete);
      if (result.complete) scanners.delete(request.job);
      self.postMessage({ id: request.id, result });
    }
  } catch (error) {
    scanners.delete(request.job);
    self.postMessage({
      id: request.id,
      error: error instanceof Error ? error.message : String(error),
    });
  }
};
