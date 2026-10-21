// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProgressiveMarkdownDocument } from './runtime';
import { MarkdownViewportHost } from './viewportHost';
import { buildMarkdownIndex } from './incrementalIndex';
import { MarkdownStructureScanner } from './structure';
import { createMemoryMarkdownPort } from './port';

const localBuild: typeof buildMarkdownIndex = (port, snapshot, options) =>
  buildMarkdownIndex(port, snapshot, {
    ...options,
    scan: async (port, snapshot, task) => {
      const scanner = new MarkdownStructureScanner(snapshot.revision, task.checkpoint);
      let from = task.checkpoint?.fromByte ?? 0;
      do {
        task.signal.throwIfAborted();
        const part = await port.readSnapshot(
          snapshot,
          from,
          Math.min(snapshot.byteLength, from + 16 * 1024),
        );
        from = part.toByte;
        if ((await task.onBatch(scanner.push(part.text, from === snapshot.byteLength))) === false)
          return;
      } while (from < snapshot.byteLength);
    },
  });

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  document.body.replaceChildren();
});
async function setup(text: string, mode: 'semantic' | 'source' = 'semantic') {
  const port = createMemoryMarkdownPort();
  const runtime = new ProgressiveMarkdownDocument(
    port,
    await port.openDocument({ text }),
    localBuild,
  );
  cleanups.push(() => runtime.close(true));
  await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
  const pane = document.createElement('div');
  pane.className = mode + '-pane';
  const target = document.createElement('div');
  pane.append(target);
  document.body.append(pane);
  const failures: unknown[] = [];
  const host = new MarkdownViewportHost(
    runtime,
    target,
    mode,
    {},
    { onActive() {}, onFailure: (error) => failures.push(error), writeClipboard: async () => {} },
  );
  cleanups.push(() => host.dispose());
  await vi.waitFor(() => expect(host.activeCore ?? host.activeSource).toBeDefined());
  return { port, runtime, host, target, failures };
}

describe('完整会话的真实窗口宿主', () => {
  it('编辑替换块组后恢复输入焦点，主动移到其他控件时不抢回', async () => {
    const { host, target, runtime } = await setup('# 标题\n\n正文');
    const nativeFocus = HTMLElement.prototype.focus;
    // jsdom 不把 contenteditable div 视为可聚焦元素；浏览器原生支持。
    const focus = vi.spyOn(HTMLElement.prototype, 'focus').mockImplementation(function(this: HTMLElement) {
      if (this.classList.contains('ProseMirror')) this.tabIndex = -1;
      nativeFocus.call(this);
    });
    try {
    await host.reveal(0, true);
    const old = target.querySelector<HTMLElement>('.ProseMirror')!;
    old.focus();
    expect(target.contains(document.activeElement)).toBe(true);
    host.activeCore!.execute({ type: 'insertText', text: '新' });
    (document.activeElement as HTMLElement).blur();
    await vi.waitFor(() => expect(target.querySelector('.ProseMirror')).not.toBe(old));
    await runtime.session.flushEdits();
    await vi.waitFor(() => expect(target.contains(document.activeElement)).toBe(true));
    const next = target.querySelector('.ProseMirror');
    host.activeCore!.execute({ type: 'insertText', text: '另' });
    const button = document.body.appendChild(document.createElement('button'));
    button.focus();
    await vi.waitFor(() => expect(target.querySelector('.ProseMirror')).not.toBe(next));
    expect(document.activeElement).toBe(button);
    } finally { focus.mockRestore(); }
  });
  it('连续滚动原位保留重叠 DOM，边界往返不销毁保留带视图', async () => {
    const { host, target, failures } = await setup('段落内容\n\n'.repeat(4000));
    const pane = target.parentElement!;
    pane.scrollTop = 2600;
    pane.dispatchEvent(new Event('scroll'));
    await vi.waitFor(() => expect(target.querySelectorAll('.markdown-session-window').length).toBeGreaterThan(1));
    const before = new Map(Array.from(target.querySelectorAll<HTMLElement>('.markdown-session-window'), el => [el.dataset.markdownGroup, el]));
    const destroyed = host.diagnostics.destroyed;
    const removed: Node[] = [];
    const observer = new MutationObserver(records => records.forEach(record => removed.push(...record.removedNodes)));
    observer.observe(target, { childList: true });
    for (const top of [2900, 2700, 2900, 2600]) {
      pane.scrollTop = top;
      pane.dispatchEvent(new Event('scroll'));
      await new Promise(resolve => setTimeout(resolve, 45));
    }
    observer.disconnect();
    for (const [id, element] of before) {
      expect(target.querySelector(`[data-markdown-group="${id}"]`)).toBe(element);
      expect(removed).not.toContain(element);
    }
    expect(host.diagnostics.destroyed).toBe(destroyed);
    expect(host.diagnostics.moved).toBe(0);
    expect(host.diagnostics.mounted).toBeLessThan(15);
    expect(failures).toEqual([]);
  });

  it('前方编辑改变字节位置后，未受影响的块组保留实例', async () => {
    const { runtime, host, target, failures } = await setup('重复内容\n\n'.repeat(4000));
    const byte = runtime.state.index!.blocks[700].fromByte;
    await host.reveal(byte);
    await new Promise(resolve => setTimeout(resolve, 100));
    const views = new Map(Array.from(target.querySelectorAll<HTMLElement>('.markdown-session-window'), el => [el.dataset.markdownGroup, el]));
    await runtime.session.applyEdits({ viewGeneration: runtime.session.state.viewGeneration,
      historyGroupId: 'prefix', edits: [{ fromByte: 0, toByte: 0, insertedText: '新增\n\n' }] });
    await vi.waitFor(() => expect(runtime.state.index?.revision).toBe(runtime.state.status.revision));
    await host.reveal(byte + 8);
    for (const [id, element] of views) {
      const current = target.querySelector(`[data-markdown-group="${id}"]`);
      if (current) expect(current).toBe(element);
    }
    expect([...views.values()].some(el => el.isConnected)).toBe(true);
    expect(failures).toEqual([]);
  });

  it('连续远跳仅保留最新请求的目标选区', async () => {
    const { runtime, host, failures } = await setup('# 起点\n\n' + '段落\n\n'.repeat(4000) + '# 终点\n');
    const end = runtime.state.index!.outline.at(-1)!;
    await Promise.all([host.reveal(end.byte, true), host.reveal(0, true)]);
    expect(host.captureSelection().headByte).toBe(2);
    expect(failures).toEqual([]);
  });

  it('静止重复渲染保留标题锚点和同一个表格 DOM，不产生正文事务', async () => {
    const text = '# 重复标题\n\n| 名称 | 内容 |\n| --- | --- |\n| 项目 | 文字 |\n\n## 重复标题\n\n结尾';
    const { runtime, host, target, failures } = await setup(text);
    const table = target.querySelector('table');
    expect(table).not.toBeNull();
    const ids = runtime.state.index!.outline.map(heading => heading.id);
    expect(new Set(ids).size).toBe(2);
    for (let i = 0; i < 8; i++) {
      await host.reveal(0);
      await new Promise(resolve => setTimeout(resolve, 30));
      expect([...target.querySelectorAll('h1,h2')].map(heading => heading.id)).toEqual(ids);
      expect(target.querySelector('table')).toBe(table);
    }
    expect(runtime.state.status.revision).toBe(0);
    expect(await runtime.readMarkdown()).toBe(text);
    expect(failures).toEqual([]);
  });

  it('跨窗口导航保留全局去重标题 ID，返回后仍可再次定位', async () => {
    const text = '# 重复标题\n\n' + '段落内容\n\n'.repeat(3500) + '# 重复标题\n\n结尾';
    const { runtime, host, target, failures } = await setup(text);
    const headings = runtime.state.index!.outline;
    expect(headings).toHaveLength(2);
    expect(headings[0].id).not.toBe(headings[1].id);
    for (const heading of [headings[1], headings[0], headings[1]]) {
      await host.reveal(heading.byte, true);
      await new Promise(resolve => setTimeout(resolve, 30));
      expect(target.querySelector('h1')?.id).toBe(heading.id);
      // 源码索引指向 #，语义光标应落在其后的标题文本起点。
      expect(host.captureSelection()?.headByte).toBe(heading.byte + 2);
    }
    expect(runtime.state.status.revision).toBe(0);
    expect(failures).toEqual([]);
  });

  it('标题编辑与撤销更新锚点，派生锚点不增加历史步骤', async () => {
    const text = '# 标题\n\n正文';
    const { runtime, host, target, failures } = await setup(text);
    await host.reveal(0, true);
    host.activeCore!.execute({ type: 'insertText', text: '新' });
    await runtime.session.flushEdits();
    await vi.waitFor(() => expect(runtime.state.index?.revision).toBe(runtime.state.status.revision));
    await host.reveal(0);
    expect(target.querySelector('h1')?.id).toBe('新标题');
    expect(await runtime.readMarkdown()).toBe('# 新标题\n\n正文');
    let rawFallback = false;
    const observer = new MutationObserver(() => { rawFallback ||= !!target.querySelector('.cm-editor'); });
    observer.observe(target, { childList: true, subtree: true });
    host.activeCore!.execute({ type: 'undo' });
    await vi.waitFor(() => expect(runtime.state.status.canRedo).toBe(true));
    await vi.waitFor(() => expect(runtime.state.index?.revision).toBe(runtime.state.status.revision));
    await host.reveal(0);
    expect(target.querySelector('h1')?.id).toBe('标题');
    expect(await runtime.readMarkdown()).toBe(text);
    observer.disconnect();
    expect(rawFallback).toBe(false);
    expect(failures).toEqual([]);
  });

  it('长文只挂载有界窗口，尾部导航和编辑保留未挂载正文及统一撤销', async () => {
    const text = '# 开头\n\n' + '段落内容\n\n'.repeat(3500) + '最后一段';
    const { runtime, host, target, failures } = await setup(text);
    expect(target.textContent!.length).toBeLessThan(text.length / 4);
    expect(target.textContent).not.toContain('最后一段');
    await host.reveal(runtime.state.status.byteLength, true);
    expect(target.textContent).toContain('最后一段');
    expect(host.activeCore).toBeDefined();
    host.activeCore!.execute({ type: 'insertText', text: '！' });
    await runtime.session.flushEdits();
    expect(await runtime.readMarkdown()).toBe(text + '！');
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe(text);
    expect(failures).toEqual([]);
  });

  it('全选的中文组字提交替换完整正文，组字结束前不提交中间文本', async () => {
    const text = '原始正文\n\n'.repeat(3500);
    const { runtime, host, target, failures } = await setup(text, 'source');
    host.selectAll();
    expect(host.captureSelection()).toEqual({
      anchorByte: 0,
      headByte: runtime.state.status.byteLength,
    });
    const input = target.querySelector('textarea')!;
    input.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    input.value = 'zhong';
    input.dispatchEvent(
      new InputEvent('beforeinput', {
        bubbles: true,
        cancelable: true,
        inputType: 'insertCompositionText',
        data: 'zhong',
        isComposing: true,
      }),
    );
    expect(runtime.state.status.revision).toBe(0);
    input.value = '中文';
    input.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '中文' }));
    await Promise.resolve();
    await runtime.session.flushEdits();
    expect(await runtime.readMarkdown()).toBe('中文');
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe(text);
    expect(failures).toEqual([]);
  });

  it('源码窗口回收后语义窗口继续同一历史，恢复保留混合换行', async () => {
    const text = '# 标题\r\n\r\n正文\n';
    const { runtime, host, target, failures } = await setup(text, 'source');
    host.activeSource!.view.dispatch({ changes: { from: 0, insert: '新' } });
    await runtime.session.flushEdits();
    await host.dispose();
    expect(await runtime.readMarkdown()).toBe('新' + text);
    await runtime.session.undo();
    expect(await runtime.readMarkdown()).toBe(text);
    expect(target.children).toHaveLength(0);
    expect(failures).toEqual([]);
  });
});
