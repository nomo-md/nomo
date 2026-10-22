import { describe, expect, it, vi } from 'vitest';
import { ProgressiveMarkdownDocument } from './runtime';
import { buildMarkdownIndex } from './incrementalIndex';
import { MarkdownStructureScanner } from './structure';
import { createMemoryMarkdownPort } from './port';
import { utf8Length } from './utf8Text';
import { MarkdownTaskPriority, markdownWorkScheduler } from './scheduler';

const localBuild: typeof buildMarkdownIndex = (port, snapshot, options) =>
  buildMarkdownIndex(port, snapshot, {
    ...options,
    scan: async (port, snapshot, task) => {
      const scanner = new MarkdownStructureScanner(snapshot.revision, task.checkpoint);
      let from = task.checkpoint?.fromByte ?? 0;
      do {
        task.signal.throwIfAborted();
        const window = await port.readSnapshot(
          snapshot,
          from,
          Math.min(snapshot.byteLength, from + 16 * 1024),
        );
        from = window.toByte;
        if ((await task.onBatch(scanner.push(window.text, from === snapshot.byteLength))) === false)
          return;
        if (from < snapshot.byteLength) await task.shouldYield?.();
      } while (from < snapshot.byteLength);
    },
  });

describe('渐进 Markdown 文档运行时', () => {
  it('未索引远跳从安全起点逐批推进，批间让出输入与其他视口并接受撤销', async () => {
    const port = createMemoryMarkdownPort();
    const status = await port.openDocument({ text: '正文\n\n'.repeat(6000) });
    const read = port.readSnapshot.bind(port);
    const owner = {}, other = {};
    const calls: string[] = [];
    const offsets: number[] = [];
    const priorities: MarkdownTaskPriority[] = [];
    vi.spyOn(port, 'readSnapshot').mockImplementation(async (...args) => {
      offsets.push(args[1]);
      calls.push('scan');
      priorities.push(runtime.structurePriority);
      if (offsets.length === 1) {
        markdownWorkScheduler.enqueue(other, 'visible', MarkdownTaskPriority.VisibleText, () => {
          calls.push('visible');
          runtime.requestVisibleStructure(owner, null);
        });
        markdownWorkScheduler.enqueue(other, 'input', MarkdownTaskPriority.Input, () => calls.push('input'));
      }
      return read(...args);
    });
    const runtime = new ProgressiveMarkdownDocument(port, status, localBuild);
    runtime.requestVisibleStructure(owner, { byte: status.byteLength - 1, revision: 0, generation: 1 });
    try {
      await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
      expect(offsets[0]).toBe(0);
      expect(offsets.length).toBeGreaterThan(1);
      expect(calls.slice(0, 4)).toEqual(['scan', 'input', 'visible', 'scan']);
      expect(priorities[0]).toBe(MarkdownTaskPriority.VisibleText);
      expect(priorities.slice(1).every(priority => priority === MarkdownTaskPriority.Index)).toBe(true);
    } finally { markdownWorkScheduler.cancel(other); await runtime.close(true); }
  });

  it('多个宿主共享可撤销提权，修订变化和隐藏清除过期需求', async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '正文\n\n'.repeat(1000) }),
      async (...args) => { await gate; return localBuild(...args); });
    const left = {}, right = {};
    try {
      runtime.requestVisibleStructure(left, { byte: 0, revision: 0, generation: 1 });
      runtime.requestVisibleStructure(right, { byte: 8000, revision: 0, generation: 1 });
      runtime.requestVisibleStructure(left, null);
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.VisibleText);
      await runtime.session.applyEdits({ viewGeneration: 0, historyGroupId: 'new', edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }] });
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.Index);
      runtime.requestVisibleStructure(left, { byte: 0, revision: runtime.state.status.revision, generation: 2 });
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.VisibleText);
      runtime.setHidden(true);
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.Index);
    } finally { release(); await runtime.close(true); }
  });

  it('首轮索引尚未完成时编辑，继承已发布的未变化块身份', async () => {
    const port = createMemoryMarkdownPort();
    let release!: () => void;
    const pause = new Promise<void>(resolve => { release = resolve; });
    const build: typeof buildMarkdownIndex = (port, snapshot, options) => buildMarkdownIndex(port, snapshot, {
      ...options,
      scan: async (port, snapshot, task) => {
        const scanner = new MarkdownStructureScanner(snapshot.revision, task.checkpoint);
        let from = task.checkpoint?.fromByte ?? 0;
        do {
          task.signal.throwIfAborted();
          const part = await port.readSnapshot(snapshot, from, Math.min(snapshot.byteLength, from + 1024));
          from = part.toByte;
          if (await task.onBatch(scanner.push(part.text, from === snapshot.byteLength)) === false) return;
          if (snapshot.revision === 0) await pause;
        } while (from < snapshot.byteLength);
      },
    });
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '重复\n\n'.repeat(1000) }), build);
    try {
      await vi.waitFor(() => expect(runtime.state.progress.state).toBe('partial'));
      const ids = runtime.state.index!.blocks.slice(2, 20).map(block => block.id);
      await runtime.session.applyEdits({ viewGeneration: 0, historyGroupId: 'partial',
        edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }] });
      release();
      await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
      expect(runtime.state.index!.blocks.slice(2, 20).map(block => block.id)).toEqual(ids);
    } finally { release(); await runtime.close(true); }
  });
  it('目录物化不增加撤销步骤，后续正文操作、重做和回到原始片段保持一致', async () => {
    const port = createMemoryMarkdownPort();
    const original = '<!-- toc -->\r\n- 手写原目录  \r\n<!-- /toc -->\r\n\r\n# 原标题\r\n\r\n尾部';
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: original }), localBuild);
    expect(await runtime.readMarkdown()).toBe(original);
    const titleByte = utf8Length(original.slice(0, original.indexOf('原标题')));
    await runtime.session.applyEdits({ viewGeneration: 0, historyGroupId: 'title',
      edits: [{ fromByte: titleByte, toByte: titleByte + utf8Length('原标题'), insertedText: '新标题更长' }] });
    await runtime.session.applyEdits({ viewGeneration: runtime.session.state.viewGeneration, historyGroupId: 'body',
      edits: [{ fromByte: runtime.state.status.byteLength, toByte: runtime.state.status.byteLength, insertedText: '！' }] });
    expect(await runtime.readMarkdown()).toContain('[新标题更长](#新标题更长)');
    await runtime.session.undo();
    const afterBodyUndo = await runtime.readMarkdown();
    expect(afterBodyUndo).toContain('[新标题更长](#新标题更长)');
    expect(afterBodyUndo.endsWith('尾部')).toBe(true);
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe(original);
    expect(runtime.state.status.dirty).toBe(false);
    await runtime.session.undo(true);
    expect(await runtime.readMarkdown()).toContain('[新标题更长](#新标题更长)');
    await runtime.session.undo(true);
    expect((await runtime.readMarkdown()).endsWith('尾部！')).toBe(true);
    await runtime.close(true);
  });
  it('新插入的目录在标题手动改回初始文本时也重新派生', async () => {
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '# 原标题\n' }), localBuild);
    const edit = async (fromByte: number, toByte: number, insertedText: string) => {
      await runtime.session.applyEdits({ viewGeneration: runtime.session.state.viewGeneration,
        historyGroupId: crypto.randomUUID(), edits: [{ fromByte, toByte, insertedText }] });
    };
    await edit(runtime.state.status.byteLength, runtime.state.status.byteLength, '\n<!-- toc -->\n- [原标题](#原标题)\n<!-- /toc -->');
    await edit(2, 2 + utf8Length('原标题'), '新标题');
    expect(await runtime.readMarkdown()).toContain('[新标题](#新标题)');
    await edit(2, 2 + utf8Length('新标题'), '原标题');
    expect(await runtime.readMarkdown()).toContain('[原标题](#原标题)');
    await runtime.close(true);
  });
  it('首个窗口不读取全文，后台索引与未挂载尾部替换共用完整快照', async () => {
    const port = createMemoryMarkdownPort();
    const source = '# 开头\n\n' + '段落\n\n'.repeat(25_000) + '搜索尾部';
    const runtime = new ProgressiveMarkdownDocument(
      port,
      await port.openDocument({ text: source }),
      localBuild,
    );
    const first = await runtime.prepareWindow(0);
    expect(first.mode).toBe('preparing');
    const sourceWindow = await runtime.prepareWindow(0, 'source');
    expect(sourceWindow.mode).toBe('source');
    if (!('window' in sourceWindow)) throw new Error('缺少源码窗口');
    expect(sourceWindow.window.text.length).toBeLessThan(16 * 1024);
    expect(sourceWindow.window.text).not.toContain('搜索尾部');
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'), { timeout: 5000 });
    expect(runtime.state.progress.indexedBytes).toBe(runtime.state.status.byteLength);
    const matches = await runtime.search('搜索尾部', { signal: new AbortController().signal });
    expect(matches.matches).toHaveLength(1);
    await runtime.replaceAll('搜索尾部', '替换完成', { signal: new AbortController().signal });
    expect((await runtime.readMarkdown()).endsWith('替换完成')).toBe(true);
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe(source);
    await runtime.close();
  });
  it('关闭会话会取消后台工作并释放全部快照，隐藏视图不释放正文或历史', async () => {
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(
      port,
      await port.openDocument({ text: '正文' }),
      localBuild,
    );
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
    runtime.setHidden(true);
    await runtime.session.applyEdits({
      viewGeneration: 0,
      historyGroupId: 'g',
      edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }],
    });
    expect(await runtime.readMarkdown()).toBe('新正文');
    runtime.setHidden(false);
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'), { timeout: 3000 });
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe('正文');
    await runtime.close();
    await expect(port.status(runtime.session.documentId)).rejects.toThrow('关闭');
  });
});
