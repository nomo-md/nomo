import { describe, expect, it, vi } from 'vitest';
import { DocumentCloseCoordinator, DocumentOperationRegistry, type DocumentCloseHooks } from './documentCloseCoordinator';

function deferred<T = void>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function scenario(overrides: Partial<DocumentCloseHooks> = {}) {
  let visible = true;
  const hooks: DocumentCloseHooks = {
    prepare: vi.fn(async () => {}), decide: vi.fn(async () => true), persist: vi.fn(async () => {}),
    detach: vi.fn(() => { visible = false; }), releaseViews: vi.fn(async () => {}), validate: vi.fn(),
    release: vi.fn(async () => {}), restore: vi.fn(async () => { visible = true; }), resume: vi.fn(), report: vi.fn(), ...overrides,
  };
  return { hooks, visible: () => visible };
}

describe('后台关闭的所有权与操作保护', () => {
  it('按文档等待编辑操作不受其他标签阻塞，关联文档完成前仍须等待', async () => {
    const operations = new DocumentOperationRegistry(), unrelated = deferred(), related = deferred();
    const other = operations.run('other', 'edit', () => unrelated.promise);
    const member = operations.run('member', 'edit', () => related.promise);
    let ready = false;
    const waiting = operations.awaitSource(['active', 'member'], ['edit', 'copy']).then(() => { ready = true; });
    await Promise.resolve(); expect(ready).toBe(false);
    related.resolve(); await member; await waiting; expect(ready).toBe(true);
    unrelated.resolve(); await other;
  });
  it('输入收口等待期间就拒绝新操作，准备取消后不恢复第二次', async () => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred(), { hooks } = scenario({ prepare: () => gate.promise, decide: async () => false });
    const closing = coordinator.request(['a'], hooks);
    await expect(coordinator.operations.run('a', 'edit', async () => {})).rejects.toThrow('文档正在关闭');
    const reopening = coordinator.reopen('a');
    gate.resolve();
    expect(await closing).toBe(false); expect(await reopening).toBe(false);
    expect(hooks.restore).not.toHaveBeenCalled();
    await expect(coordinator.operations.run('a', 'edit', async () => 1)).resolves.toBe(1);
  });
  it('保存未完成时先移除标签，重复关闭复用同一任务，保存成功后才释放', async () => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred<boolean>(), { hooks, visible } = scenario();
    const save = coordinator.operations.run('a', 'save', async operation => { operation.snapshot(3); return gate.promise; });
    await vi.waitFor(() => expect(coordinator.operations.coversRevision('a', 3)).toBe(true));
    const closing = coordinator.request(['a'], hooks);
    expect(coordinator.request(['a'], hooks)).toBe(closing);
    expect(await closing).toBe(true); expect(visible()).toBe(false); expect(hooks.release).not.toHaveBeenCalled();
    gate.resolve(true); await save; await coordinator.awaitSettled();
    expect(hooks.release).toHaveBeenCalledOnce(); expect(coordinator.pending).toBe(0);
  });
  it.each(['failed', 'cancelled'] as const)('后台保存 %s 时恢复标签，保留源会话', async status => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred<boolean>(), { hooks, visible } = scenario();
    const save = coordinator.operations.run('a', 'save', async operation => { operation.snapshot(2); const result = await gate.promise; operation.saveResult = { status, error: 'disk error' }; return result; });
    await coordinator.request(['a'], hooks); gate.resolve(false); await save; await coordinator.awaitSettled();
    expect(visible()).toBe(true); expect(hooks.restore).toHaveBeenCalledOnce(); expect(hooks.release).not.toHaveBeenCalled();
  });
  it('其他文档的保存不阻碍目标关闭', async () => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred<boolean>(), { hooks } = scenario();
    const save = coordinator.operations.run('other', 'save', () => gate.promise);
    await coordinator.request(['a'], hooks); await coordinator.awaitSettled(); expect(hooks.release).toHaveBeenCalledOnce();
    gate.resolve(true); await save;
  });
  it('导出捕获保护源会话，独立输出只阻止窗口退出', async () => {
    const coordinator = new DocumentCloseCoordinator(), capture = deferred(), output = deferred(), { hooks } = scenario();
    const job = coordinator.operations.run('a', 'export', async operation => { await capture.promise; operation.releaseSource(); await output.promise; });
    await coordinator.request(['a'], hooks); expect(hooks.release).not.toHaveBeenCalled();
    capture.resolve(); await coordinator.awaitSettled(); expect(hooks.release).toHaveBeenCalledOnce();
    let exiting = false; const exit = coordinator.operations.awaitAll().then(() => { exiting = true; });
    await Promise.resolve(); expect(exiting).toBe(false); output.resolve(); await job; await exit; expect(exiting).toBe(true);
  });
  it('记录持久化失败不移除标签，不释放源会话', async () => {
    const { hooks, visible } = scenario({ persist: vi.fn(async () => { throw new Error('write failed'); }) });
    const coordinator = new DocumentCloseCoordinator(); expect(await coordinator.request(['a'], hooks)).toBe(false);
    expect(visible()).toBe(true); expect(hooks.detach).not.toHaveBeenCalled(); expect(hooks.release).not.toHaveBeenCalled(); expect(hooks.resume).toHaveBeenCalled();
  });
  it('取消关闭不启动销毁，并允许重新登记文档操作', async () => {
    const { hooks } = scenario({ decide: vi.fn(async () => false) }), coordinator = new DocumentCloseCoordinator();
    expect(await coordinator.request(['a'], hooks)).toBe(false);
    expect(hooks.detach).not.toHaveBeenCalled(); await expect(coordinator.operations.run('a', 'edit', async () => 1)).resolves.toBe(1);
  });
  it('组合中任一保存失败时统一恢复，所有成员均未最终释放', async () => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred<boolean>(), { hooks } = scenario();
    const save = coordinator.operations.run('b', 'save', () => gate.promise);
    await coordinator.request(['a', 'b'], hooks); gate.resolve(false); await save; await coordinator.awaitSettled();
    expect(hooks.restore).toHaveBeenCalledOnce(); expect(hooks.release).not.toHaveBeenCalled();
  });
  it('后台关闭期间重开取消最终释放，迟到保存不能关闭恢复的标签', async () => {
    const coordinator = new DocumentCloseCoordinator(), gate = deferred<boolean>(), { hooks, visible } = scenario();
    const save = coordinator.operations.run('a', 'save', () => gate.promise);
    await coordinator.request(['a'], hooks); expect(await coordinator.reopen('a')).toBe(true); expect(visible()).toBe(true);
    gate.resolve(true); await save; await coordinator.awaitSettled(); expect(hooks.release).not.toHaveBeenCalled();
  });
  it('同文档保存按接受顺序执行，旧修订不能覆盖新保存', async () => {
    const operations = new DocumentOperationRegistry(), gate = deferred(), writes: number[] = [];
    const first = operations.run('a', 'save', async operation => { operation.snapshot(1); await gate.promise; writes.push(1); return true; });
    const second = operations.run('a', 'save', async operation => { operation.snapshot(2); writes.push(2); return true; });
    await Promise.resolve(); expect(writes).toEqual([]); gate.resolve(); await Promise.all([first, second]); expect(writes).toEqual([1, 2]);
  });
});
