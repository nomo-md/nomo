import { describe, expect, it, vi } from 'vitest';
import { NativeOutlineView } from './nativeOutlineView';
import { NativeStructureClient } from './nativeStructureClient';
import { STRUCTURE_SYNTAX_VERSION, type StructureIdentity, type StructureQuery, type StructureQueryPort, type StructureResponse } from './structureProtocol';

const identity = (revision = 0): StructureIdentity => ({ documentId: 'outline-test', revision, indexGeneration: revision + 1, syntaxVersion: STRUCTURE_SYNTAX_VERSION });
const response = (id: StructureIdentity, items: unknown[], state: 'ready' | 'building' = 'ready'): StructureResponse => ({ ...id, state, indexedBytes: 20, totalBytes: 20, semanticComplete: state === 'ready', failure: null, next: null, fragment: null, items });
const row = (position = 0) => ({ id: `slug-${position}`, blockId: `block-${position}`, blockHeadingIndex: 0, title: `标题 ${position}`, level: 1, byte: position, line: position + 1, position, collapsed: false, parentByte: null, hasChildren: true });
const window = (fromIndex = 0, complete = true) => ({ viewId: 'view', viewGeneration: 0, fromIndex, visibleCount: 1000, projectionComplete: complete, rows: [row(fromIndex)] });
const open = (port: StructureQueryPort, revision = 0) => NativeStructureClient.open(port, 'outline-test', revision, new AbortController().signal);
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

describe('原生大纲视图消费', () => {
  it('新修订构建中与查询失败均保留最后可读窗口，完成后校准', async () => {
    let building = false, fail = false;
    const query = vi.fn(async (id: StructureIdentity, q: StructureQuery) => {
      if (q.kind === 'openOutline') return response(id, [{ viewId: 'view' }]);
      if (fail) throw new Error('disk full');
      return response(id, [{ ...window(), rows: building ? [] : [row()], projectionComplete: !building }], building ? 'building' : 'ready');
    });
    const port: StructureQueryPort = { startStructure: async (_, revision) => identity(revision), queryStructure: query as StructureQueryPort['queryStructure'], closeStructureOutline: vi.fn(async () => {}) };
    const first = await open(port); const view = new NativeOutlineView(first);
    const previous = await view.load();
    const next = await open(port, 1); building = true; view.bind(next);
    expect(await view.load()).toBe(previous);
    fail = true; await expect(view.load()).rejects.toThrow('disk full'); expect(view.snapshot).toBe(previous);
    fail = building = false; expect((await view.load())?.rows[0].title).toBe('标题 0');
    await view.dispose(); first.dispose(); next.dispose();
    expect(port.closeStructureOutline).toHaveBeenCalledTimes(1);
    expect(port.closeStructureOutline).toHaveBeenCalledWith('outline-test', 'view');
  });
  it('较早滚动查询的迟到响应不能覆盖新窗口', async () => {
    const older = deferred<StructureResponse>();
    const port: StructureQueryPort = { startStructure: async () => identity(), closeStructureOutline: async () => {},
      queryStructure: vi.fn(async (id, q) => q.kind === 'openOutline' ? response(id, [{ viewId: 'view' }]) :
        q.kind === 'outlineView' && q.fromIndex === 0 ? older.promise : response(id, [window(400)])) as StructureQueryPort['queryStructure'] };
    const client = await open(port); const view = new NativeOutlineView(client);
    const pending = view.load(0); await Promise.resolve();
    await view.load(400); older.resolve(response(identity(), [window(0)])); await pending;
    expect(view.snapshot?.fromIndex).toBe(400);
    await view.dispose(); client.dispose();
  });
  it('面板在创建期间销毁，迟到句柄仍释放一次', async () => {
    const opening = deferred<StructureResponse>();
    const close = vi.fn(async () => {});
    const port: StructureQueryPort = { startStructure: async () => identity(), queryStructure: (() => opening.promise) as StructureQueryPort['queryStructure'], closeStructureOutline: close };
    const client = await open(port); const view = new NativeOutlineView(client);
    const pending = view.load(); const disposed = view.dispose();
    opening.resolve(response(identity(), [{ viewId: 'view' }]));
    await Promise.all([pending, disposed]); expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenCalledWith('outline-test', 'view');
    expect(view.snapshot).toBeNull(); client.dispose();
  });
  it('释放传输失败保留句柄以便重试，会话已经关闭则视为已回收', async () => {
    const close = vi.fn().mockRejectedValueOnce(new Error('transport')).mockRejectedValueOnce({ code: 'document-closed' });
    const port: StructureQueryPort = { startStructure: async () => identity(), closeStructureOutline: close,
      queryStructure: vi.fn(async (id, q) => response(id, q.kind === 'openOutline' ? [{ viewId: 'view' }] : [window()])) as StructureQueryPort['queryStructure'] };
    const client = await open(port); const view = new NativeOutlineView(client); await view.load();
    await expect(view.dispose()).rejects.toThrow('transport');
    await view.dispose(); expect(close).toHaveBeenCalledTimes(2); expect(view.snapshot).toBeNull(); client.dispose();
  });
});
