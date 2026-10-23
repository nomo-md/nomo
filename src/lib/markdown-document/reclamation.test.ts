import { describe, expect, it, vi } from 'vitest';
import { reclaimMarkdownViews } from './reclamation';

describe('关闭视图分批回收', () => {
  it('回收先让出当前点击任务，单个释放失败后其余窗口仍释放', async () => {
    const releases = [vi.fn(), vi.fn(() => { throw new Error('broken view'); }), vi.fn()];
    const pending = reclaimMarkdownViews(releases);
    expect(releases[0]).not.toHaveBeenCalled();
    await expect(pending).rejects.toThrow('broken view');
    expect(releases[2]).toHaveBeenCalledOnce();
  });
});
