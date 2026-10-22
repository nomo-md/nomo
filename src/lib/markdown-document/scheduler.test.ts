import { describe, expect, it, vi } from 'vitest';
import { MarkdownTaskPriority, MarkdownWorkScheduler } from './scheduler';

describe('滚动时的近视口准备', () => {
  it('后台已预约延迟时，新正文任务提前唤醒，后台仍等待交互结束', async () => {
    vi.useFakeTimers();
    try {
      const scheduler = new MarkdownWorkScheduler(4, () => Date.now());
      const owner = {}, calls: string[] = [];
      scheduler.interacting(120);
      scheduler.enqueue(owner, 'index', MarkdownTaskPriority.Index, () => calls.push('index'));
      await vi.advanceTimersByTimeAsync(10);
      scheduler.enqueue(owner, 'body', MarkdownTaskPriority.VisibleText, () => calls.push('body'));
      await vi.advanceTimersByTimeAsync(1);
      expect(calls).toEqual(['body']);
      await vi.advanceTimersByTimeAsync(109);
      expect(calls).toEqual(['body', 'index']);
      scheduler.cancel(owner);
    } finally { vi.useRealTimers(); }
  });

  it('暂停的正文不唤醒后台，恢复后立即运行；取消后不得补执行', async () => {
    vi.useFakeTimers();
    try {
      const scheduler = new MarkdownWorkScheduler(4, () => Date.now());
      const background = {}, body = {}, calls: string[] = [];
      scheduler.interacting(120);
      scheduler.enqueue(background, 'index', MarkdownTaskPriority.Index, () => calls.push('index'));
      scheduler.pause(body, true);
      scheduler.enqueue(body, 'body', MarkdownTaskPriority.VisibleText, () => calls.push('body'));
      await vi.advanceTimersByTimeAsync(10);
      expect(calls).toEqual([]);
      scheduler.pause(body, false);
      await vi.advanceTimersByTimeAsync(1);
      expect(calls).toEqual(['body']);
      scheduler.enqueue(body, 'stale', MarkdownTaskPriority.Input, () => calls.push('stale'));
      scheduler.cancel(body);
      await vi.advanceTimersByTimeAsync(200);
      expect(calls).toEqual(['body', 'index']);
      scheduler.cancel(background);
    } finally { vi.useRealTimers(); }
  });

  it('批次耗尽后剩余正文不等待后台交互期限', async () => {
    vi.useFakeTimers();
    try {
      let now = 0;
      const scheduler = new MarkdownWorkScheduler(4, () => now), owner = {}, calls: string[] = [];
      scheduler.interacting(120);
      scheduler.enqueue(owner, 'first', MarkdownTaskPriority.VisibleText, () => { calls.push('first'); now += 5; });
      scheduler.enqueue(owner, 'second', MarkdownTaskPriority.VisibleText, () => calls.push('second'));
      await vi.advanceTimersByTimeAsync(2);
      expect(calls).toEqual(['first', 'second']);
      scheduler.cancel(owner);
    } finally { vi.useRealTimers(); }
  });
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
