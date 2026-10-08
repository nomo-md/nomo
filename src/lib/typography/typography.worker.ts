import { layoutParagraph } from './knuthPlass';
import type { ParagraphInput } from './types';

self.onmessage = (event: MessageEvent<{ id: number; input: ParagraphInput }>) => {
  const { id, input } = event.data;
  try {
    self.postMessage({ id, result: layoutParagraph(input) });
  } catch {
    self.postMessage({
      id,
      result: { status: 'fallback', reason: 'worker-failed', candidates: 0 },
    });
  }
};
