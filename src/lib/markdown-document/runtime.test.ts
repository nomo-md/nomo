import { describe, expect, it, vi } from 'vitest';
import { ProgressiveMarkdownDocument } from './runtime';
import { buildMarkdownIndex } from './incrementalIndex';
import { MarkdownStructureScanner } from './structure';
import { createMemoryMarkdownPort } from './port';
import { utf8Length } from './utf8Text';

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
    expect(first.window.text.length).toBeLessThan(16 * 1024);
    expect(first.window.text).not.toContain('搜索尾部');
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
