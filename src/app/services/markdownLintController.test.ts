import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMarkdownLintController } from './markdownLintController';

afterEach(() => vi.unstubAllGlobals());

describe('Markdown lint fixed capacity', () => {
  it('checks exactly 500000 UTF-16 units and skips larger input independently of old settings', () => {
    const posted = vi.fn();
    vi.stubGlobal('Worker', class {
      postMessage = posted;
      terminate() {}
    });
    const changed = vi.fn();
    const controller = createMarkdownLintController(changed);
    const input = { tabId: 'doc', version: 1, ruleSet: 'relaxed' as const, largeDocumentMode: false,
      markdown: '中'.repeat(500000), largeDocumentLimit: 1 };
    try {
      controller.checkNow(input);
      expect(changed).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'checking' }));
      expect(posted).toHaveBeenCalledOnce();
      controller.checkNow({ ...input, markdown: input.markdown + '中', largeDocumentLimit: 1000000 } as typeof input);
      expect(changed).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'skipped' }));
      expect(posted).toHaveBeenCalledOnce();
    } finally { controller.destroy(); }
  });
});
