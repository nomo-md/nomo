import { describe, expect, it, vi } from 'vitest';
import { createRenderTaskQueue } from './renderTaskQueue';

describe('render task scheduling', () => {
  it('yields between expensive jobs and cancels the last queued closure during the yield', async () => {
    vi.useFakeTimers();
    let now = 0; const clock = vi.spyOn(performance, 'now').mockImplementation(() => now);
    try {
      const run = createRenderTaskQueue(1), abort = new AbortController();
      const first = run(async () => { now += 8; return 'first'; });
      const next = vi.fn(async () => 'next');
      const second = run(next, abort.signal).catch(error => error.name);
      await first; await Promise.resolve(); await Promise.resolve();
      expect(next).not.toHaveBeenCalled();
      expect(vi.getTimerCount()).toBe(1);
      abort.abort(); expect(await second).toBe('AbortError');
      expect(vi.getTimerCount()).toBe(0);
      expect(await run(async () => 'fresh')).toBe('fresh');
    } finally { clock.mockRestore(); vi.useRealTimers(); }
  });
  it('bounds concurrency and skips cancelled queued work without blocking subsequent jobs', async () => {
    const run = createRenderTaskQueue(1);
    let finish!: () => void;
    const first = run(() => new Promise<void>(resolve => { finish = resolve; }));
    await Promise.resolve();
    const abort = new AbortController();
    const skipped = vi.fn(async () => 'obsolete');
    const cancelled = run(skipped, abort.signal).catch(error => error.name);
    const lastTask = vi.fn(async () => 'latest');
    const last = run(lastTask);
    abort.abort();
    expect(await cancelled).toBe('AbortError');
    expect(lastTask).not.toHaveBeenCalled();
    finish();
    await first;
    expect(await last).toBe('latest');
    expect(skipped).not.toHaveBeenCalled();
  });
});
