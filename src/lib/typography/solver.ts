import { layoutParagraph } from './knuthPlass';
import TypographyWorker from './typography.worker?worker&inline';
import type { ParagraphInput, ParagraphLayout } from './types';

export function createTypographySolver() {
  let worker: Worker | null = null;
  let failed = false;
  let nextId = 0;
  const pending = new Map<
    number,
    { resolve: (result: ParagraphLayout) => void; timer: ReturnType<typeof setTimeout> }
  >();
  const fail = () => {
    failed = true;
    worker?.terminate();
    worker = null;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.resolve({ status: 'fallback', reason: 'worker-failed', candidates: 0 });
    }
    pending.clear();
  };
  return {
    solve(input: ParagraphInput): Promise<ParagraphLayout> {
      if (failed)
        return Promise.resolve({ status: 'fallback', reason: 'worker-failed', candidates: 0 });
      if (typeof Worker === 'undefined')
        return new Promise((resolve) => setTimeout(() => resolve(layoutParagraph(input)), 0));
      try {
        if (!worker) {
          worker = new TypographyWorker();
          worker.onerror = fail;
          worker.onmessage = ({ data }: MessageEvent<{ id: number; result: ParagraphLayout }>) => {
            const request = pending.get(data.id);
            if (!request) return;
            clearTimeout(request.timer);
            pending.delete(data.id);
            request.resolve(data.result);
          };
        }
        const id = ++nextId;
        return new Promise((resolve) => {
          pending.set(id, { resolve, timer: setTimeout(fail, 5_000) });
          worker!.postMessage({ id, input });
        });
      } catch {
        fail();
        return Promise.resolve({ status: 'fallback', reason: 'worker-failed', candidates: 0 });
      }
    },
    reset() {
      failed = false;
    },
    destroy: fail,
  };
}
