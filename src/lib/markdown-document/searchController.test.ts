import { describe, expect, it, vi } from 'vitest';
import { MarkdownSessionController } from './controller';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownSearchController } from './searchController';
import { searchMarkdownSnapshot } from './tasks';
import samples from './searchSamples.json';

async function fixture(text: string) {
  const port = createMemoryMarkdownPort(), session = new MarkdownSessionController(port, await port.openDocument({ text }));
  const searches = new MarkdownSearchController(session);
  return { port, session, searches, options: () => ({ signal: new AbortController().signal }),
    read: () => session.withSnapshot(snapshot => readCompleteSnapshot(port, snapshot)),
    close: async () => { await searches.dispose(); await session.close(true); } };
}

describe('Markdown 原生任务和有界分页契约', () => {
  it('启动前提交草稿和物化目录后捕获新版本，不误取消自身', async () => {
    const f = await fixture('cat');
    let pending = true;
    const detach = f.session.attachEdits({
      awaitCompositionEnd: async () => {},
      commitPendingEdits: async () => {
        if (!pending) return;
        pending = false;
        await f.session.applyEdits({ viewGeneration: f.session.state.viewGeneration, historyGroupId: 'draft',
          edits: [{ fromByte: 3, toByte: 3, insertedText: ' cat' }] });
      },
    });
    f.session.beforeSnapshot = async () => {
      f.session.beforeSnapshot = undefined;
      await f.session.applyEdits({ viewGeneration: f.session.state.viewGeneration, historyGroupId: 'toc', derived: true,
        edits: [{ fromByte: 0, toByte: 0, insertedText: 'cat ' }] });
    };
    const result = await f.searches.replaceAll('cat', 'dog', f.options());
    expect(result.count).toBe(3); expect(await f.read()).toBe('dog dog dog');
    detach(); await f.close();
  });
  it('共享 Unicode 样本与旧 JS /iu 匹配一致', async () => {
    for (const sample of samples) {
      const f = await fixture(sample.source);
      const snapshot = await f.port.captureSnapshot(f.session.documentId);
      const matches = await searchMarkdownSnapshot(f.port, snapshot, sample.query, { ...sample, ...f.options() });
      const results = await f.searches.search(sample.query, { ...sample, ...f.options() });
      expect(results.totalCount, JSON.stringify(sample)).toBe(sample.count);
      expect((await results.page(0)).matches.map(({ index: _, ...match }) => match)).toEqual(matches.map(({ text: _, ...match }) => match));
      await f.port.releaseSnapshot(snapshot); await f.close();
    }
  });
  it('十万命中只保留三页，搜索任务不调用正文读取，迟到分页不扩展缓存', async () => {
    const f = await fixture('a '.repeat(100_000)), read = vi.spyOn(f.port, 'readSnapshot');
    const result = await f.searches.search('a', f.options());
    expect(result.totalCount).toBe(100_000);
    for (const index of [0, 127, 128, 510, 99_999, 0]) {
      expect((await result.match(index))?.fromByte).toBe(index * 2);
      expect(result.residentMatches).toBeLessThanOrEqual(384);
    }
    expect(read).not.toHaveBeenCalled();
    await result.dispose(); expect(result.residentMatches).toBe(0); await f.close();
  });
  it('新输入取消准备，不能覆盖新正文', async () => {
    const f = await fixture('cat cat');
    let resume!: () => void;
    const prepare = f.port.prepareReplaceAll;
    f.port.prepareReplaceAll = async request => { const result = await prepare(request); await new Promise<void>(resolve => { resume = resolve; }); return result; };
    const replacing = f.searches.replaceAll('cat', 'dog', f.options());
    const rejected = expect(replacing).rejects.toThrow();
    await vi.waitFor(() => expect(resume).toBeTypeOf('function'));
    await f.session.applyEdits({ viewGeneration: 0, historyGroupId: 'typing', edits: [{ fromByte: 0, toByte: 0, insertedText: 'new ' }] });
    resume(); await rejected; expect(await f.read()).toBe('new cat cat'); await f.close();
  });
  it('先监听再启动，完成事件早于响应也正确；确认丢失只重试相同操作', async () => {
    const f = await fixture('cat cat'), commit = f.port.commitReplaceAll;
    let first = true;
    const calls: unknown[][] = [];
    f.port.commitReplaceAll = async (...args) => {
      calls.push(args); const result = await commit(...args);
      if (first) { first = false; throw Object.assign(new Error('response lost'), { code: undefined }); } return result;
    };
    const order: string[] = [], listen = f.port.listenSearch, prepare = f.port.prepareReplaceAll;
    f.port.listenSearch = async listener => { order.push('listen'); return listen(listener); };
    f.port.prepareReplaceAll = async request => { order.push('start'); const result = await prepare(request);
      await vi.waitFor(async () => expect((await f.port.searchTaskProgress(result.taskId)).state).toBe('prepared')); return result; };
    const result = await f.searches.replaceAll('cat', 'dog', f.options());
    expect(order).toEqual(['listen', 'start']); expect(calls).toHaveLength(2); expect(calls[0]).toEqual(calls[1]);
    expect(result.count).toBe(2); expect(result.status.revision).toBe(1); expect(await f.read()).toBe('dog dog');
    await f.session.undo(); expect(await f.read()).toBe('cat cat'); await f.session.undo(true); expect(await f.read()).toBe('dog dog'); await f.close();
  });
  it('提交门禁覆盖间隙输入且取消发布之后仍报告成功', async () => {
    const f = await fixture('cat'), commit = f.port.commitReplaceAll, abort = new AbortController();
    let resume!: () => void;
    f.port.commitReplaceAll = async (...args) => { const result = await commit(...args); await new Promise<void>(resolve => { resume = resolve; }); return result; };
    const replacing = f.searches.replaceAll('cat', 'dog', { signal: abort.signal });
    await vi.waitFor(() => expect(resume).toBeTypeOf('function'));
    expect(f.session.state.nativeCommitBusy).toBe(true);
    await expect(f.session.applyEdits({ viewGeneration: 0, historyGroupId: 'x', edits: [{ fromByte: 0, toByte: 0, insertedText: 'x' }] })).rejects.toThrow('文档状态');
    abort.abort(); resume(); expect((await replacing).count).toBe(1); expect(await f.read()).toBe('dog'); await f.close();
  });
  it('全部替换请求只带参数，CRLF 重叠范围及无变化不会增加历史', async () => {
    const f = await fixture('\r\n\r\n\r\n'), prepare = vi.spyOn(f.port, 'prepareReplaceAll');
    const result = await f.searches.replaceAll('\n\r', 'x', f.options());
    expect(await f.read()).toBe('\rxx\n'); expect(result.count).toBe(2);
    expect(Object.keys(prepare.mock.calls[0][0])).not.toContain('edits');
    expect(Object.keys(prepare.mock.calls[0][0])).not.toContain('matches');
    const revision = f.session.state.committed.revision;
    await f.searches.replaceAll('x', 'x', f.options()); expect(f.session.state.committed.revision).toBe(revision);
    await f.session.undo(); expect(await f.read()).toBe('\r\n\r\n\r\n'); await f.close();
  });
  it('启动响应迟到时仍释放任务，不让关闭后的监听器驻留', async () => {
    const f = await fixture('cat'), abort = new AbortController(), start = f.port.startSearch;
    const release = vi.spyOn(f.port, 'releaseTask'); let resume!: () => void;
    f.port.startSearch = async query => { const identity = await start(query); await new Promise<void>(resolve => { resume = resolve; }); return identity; };
    const searching = f.searches.search('cat', { signal: abort.signal }), rejected = expect(searching).rejects.toThrow();
    await vi.waitFor(() => expect(resume).toBeTypeOf('function')); abort.abort(); resume(); await rejected;
    expect(release).toHaveBeenCalled(); await f.close();
  });
  it('新查询等待旧启动确认释放，旧请求不能晚到后撤销新搜索', async () => {
    const f = await fixture('cat dog dog'), start = f.port.startSearch;
    let resume!: () => void, starts = 0;
    f.port.startSearch = async query => {
      starts++; if (starts === 1) await new Promise<void>(resolve => { resume = resolve; });
      return start(query);
    };
    const first = f.searches.search('cat', f.options()), rejected = expect(first).rejects.toThrow();
    await vi.waitFor(() => expect(resume).toBeTypeOf('function'));
    const next = f.searches.search('dog', f.options());
    await new Promise(resolve => setTimeout(resolve, 5)); expect(starts).toBe(1);
    resume(); await rejected; expect((await next).totalCount).toBe(2); await f.close();
  });
  it('发布后的释放失败不会伪装成替换失败', async () => {
    const f = await fixture('cat'), release = f.port.releaseTask;
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => {});
    f.port.releaseTask = async id => { await release(id); throw new Error('release response lost'); };
    expect((await f.searches.replaceAll('cat', 'dog', f.options())).count).toBe(1);
    expect(await f.read()).toBe('dog'); expect(warning).toHaveBeenCalled();
    warning.mockRestore(); f.port.releaseTask = release; await f.close();
  });
  it('两次响应丢失后仍确认同一操作，恢复阅读锚点而不再次替换', async () => {
    const f = await fixture('cat'), commit = f.port.commitReplaceAll, mapped = vi.fn();
    let lost = 2;
    f.port.commitReplaceAll = async (...args) => { const result = await commit(...args);
      if (lost-- > 0) throw new Error('response lost'); return result; };
    await expect(f.searches.replaceAll('cat', 'dog dog', f.options(), () => ({ anchors: [3] }), mapped)).rejects.toThrow('response lost');
    await f.session.retryPendingEdits();
    expect(mapped).toHaveBeenCalledWith([7]); expect(await f.read()).toBe('dog dog');
    expect(f.session.state.committed.revision).toBe(1); await f.close();
  });
  it('两次发送均失败时确认未发布，显式重试后解除门禁并保留正文', async () => {
    const f = await fixture('cat'), commit = f.port.commitReplaceAll;
    f.port.commitReplaceAll = async () => { throw new Error('not delivered'); };
    await expect(f.searches.replaceAll('cat', 'dog', f.options())).rejects.toThrow('not delivered');
    f.port.commitReplaceAll = commit;
    await expect(f.session.retryPendingEdits()).rejects.toThrow('上次替换未提交');
    expect(f.session.state.failure).toBeNull(); expect(await f.read()).toBe('cat');
    await f.session.applyEdits({ viewGeneration: f.session.state.viewGeneration, historyGroupId: 'typing',
      edits: [{ fromByte: 3, toByte: 3, insertedText: '!' }] });
    expect(await f.read()).toBe('cat!'); await f.close();
  });
});
