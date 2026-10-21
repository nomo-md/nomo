import { describe, expect, it, vi } from 'vitest';
import { MarkdownTaskPriority, MarkdownWorkScheduler } from './scheduler';

describe('滚动时的近视口准备', () => {
  it('持续交互不暂停近视口任务，全文后台工作仍等待', async () => {
    vi.useFakeTimers();
    try {
      let now = 0;
      const scheduler = new MarkdownWorkScheduler(4, () => now);
      const owner = {};
      const calls: string[] = [];
      scheduler.interacting(120);
      scheduler.enqueue(owner, 'index', MarkdownTaskPriority.Index, () => calls.push('index'));
      scheduler.enqueue(owner, 'near', MarkdownTaskPriority.NearViewport, () => calls.push('near'));
      await vi.advanceTimersByTimeAsync(1);
      expect(calls).toEqual(['near']);
      now = 130;
      await vi.advanceTimersByTimeAsync(130);
      expect(calls).toEqual(['near', 'index']);
      scheduler.cancel(owner);
    } finally { vi.useRealTimers(); }
  });
});
