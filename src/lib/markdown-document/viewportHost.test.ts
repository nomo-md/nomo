// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProgressiveMarkdownDocument } from './runtime';
import { MarkdownViewportHost, type MarkdownPresentation } from './viewportHost';
import { buildMarkdownIndex } from './incrementalIndex';
import { MarkdownStructureScanner } from './structure';
import { createMemoryMarkdownPort } from './port';
import { MarkdownTaskPriority } from './scheduler';

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
async function setup(text: string, mode: 'semantic' | 'source' = 'semantic', onOpenSource?: (range: { fromByte: number; toByte: number; byte: number }) => void) {
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
    { onActive() {}, onFailure: (error) => failures.push(error), writeClipboard: async () => {}, onOpenSource },
  );
  cleanups.push(() => host.dispose());
  await vi.waitFor(() => expect(target.querySelector('.markdown-session-window')).not.toBeNull());
  return { port, runtime, host, target, failures };
}

describe('完整会话的真实窗口宿主', () => {
  it.each([
    { height: 240, offset: 64, scale: 1 },
    { height: 800, offset: 160, scale: 1 },
    { height: 1200, offset: 180, scale: 1 },
    { height: 800, offset: 160, scale: 1.5 },
  ])('大纲在 $height 高编辑区按 $offset 对齐（缩放 $scale），高亮跟随同一参考线', async ({ height, offset, scale }) => {
    const { runtime, host, target, failures } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    const pane = target.parentElement!;
    Object.defineProperty(pane, 'clientHeight', { value: height });
    Object.defineProperty(pane, 'scrollHeight', { value: 6000 });
    Object.defineProperty(target, 'offsetWidth', { value: 500 });
    pane.getBoundingClientRect = () => new DOMRect(0, 100, 500, height);
    target.getBoundingClientRect = () => new DOMRect(0, 300 - pane.scrollTop, 500 * scale, 4000);
    const headings = [...target.querySelectorAll<HTMLElement>('h1')];
    const positions = [0, 900];
    headings.forEach((heading, i) => {
      heading.getBoundingClientRect = () => new DOMRect(0, 300 + positions[i] * scale - pane.scrollTop, 500, 40);
    });
    const second = runtime.state.index!.outline[1];
    const reveal = vi.spyOn(host.activeCore!, 'revealHeadingAnchor');
    await host.reveal(second.byte, true, 'outline');
    expect(headings[1].getBoundingClientRect().top - pane.getBoundingClientRect().top).toBeCloseTo(offset);
    expect(reveal).toHaveBeenCalledWith(second.id, false);
    expect(host.activeOutlineId).toBe(second.id);
    // 资源重排后仍保持点击的标题位置；用户滚动后交回真实参考线判定。
    positions[1] += 100;
    target.dispatchEvent(new Event('load'));
    await vi.waitFor(() => expect(headings[1].getBoundingClientRect().top - 100).toBeCloseTo(offset));
    pane.scrollTop = 0;
    pane.dispatchEvent(new Event('scroll'));
    expect(host.activeOutlineId).toBe(runtime.state.index!.outline[0].id);
    expect(host.position.byteOffset).not.toBe(second.byte);
    expect(failures).toEqual([]);
  });

  it('文末受滚动范围限制时保持可达位置及点击目标，用户接管后恢复参考线高亮', async () => {
    const { runtime, host, target } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    const pane = target.parentElement!;
    Object.defineProperty(pane, 'clientHeight', { value: 800 });
    Object.defineProperty(pane, 'scrollHeight', { value: 1200 });
    let scrollTop = 0;
    Object.defineProperty(pane, 'scrollTop', { get: () => scrollTop, set: value => { scrollTop = Math.min(400, Math.max(0, value)); } });
    pane.getBoundingClientRect = () => new DOMRect(0, 100, 500, 800);
    target.getBoundingClientRect = () => new DOMRect(0, 100 - scrollTop, 500, 1200);
    [...target.querySelectorAll<HTMLElement>('h1')].forEach((heading, i) => {
      heading.getBoundingClientRect = () => new DOMRect(0, 100 + i * 900 - scrollTop, 500, 40);
    });
    await host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
    expect(scrollTop).toBe(400);
    expect(host.activeOutlineId).toBe(runtime.state.index!.outline[1].id);
    pane.dispatchEvent(new Event('wheel'));
    expect(host.activeOutlineId).toBe(runtime.state.index!.outline[0].id);
  });

  it('大纲在源码模式使用标题行坐标，普通 reveal 保留原有定位方式', async () => {
    const { runtime, host, target, failures } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾', 'source');
    const pane = target.parentElement!;
    Object.defineProperty(pane, 'clientHeight', { value: 800 });
    Object.defineProperty(pane, 'scrollHeight', { value: 4000 });
    pane.getBoundingClientRect = () => new DOMRect(0, 100, 500, 800);
    target.getBoundingClientRect = () => new DOMRect(0, 100 - pane.scrollTop, 500, 3000);
    const second = runtime.state.index!.outline[1];
    vi.spyOn(host.activeSource!, 'byteTop').mockImplementation(byte => 100 + (byte === second.byte ? 900 : 0) - pane.scrollTop);
    await host.reveal(second.byte, true, 'outline');
    expect(pane.scrollTop).toBe(740);
    expect(host.activeOutlineId).toBe(second.id);
    pane.dispatchEvent(new Event('wheel'));
    pane.scrollTop = 0;
    expect(host.activeOutlineId).toBe(runtime.state.index!.outline[0].id);
    await host.reveal(second.byte);
    expect(pane.scrollTop).not.toBe(740);
    expect(failures).toEqual([]);
  });

  it('大纲等待挂载期间用户滚动会取消旧定位，连续点击仅保留最后目标', async () => {
    const { runtime, host, port, target } = await setup('# 第一章\n\n' + '正文\n\n'.repeat(3500) + '# 第二章\n\n结尾');
    const end = runtime.state.index!.outline[1];
    const read = port.readWindow.bind(port);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let started!: () => void;
    const waiting = new Promise<void>(resolve => { started = resolve; });
    vi.spyOn(port, 'readWindow').mockImplementation(async (...args) => {
      if (args[2] > 0) { started(); await gate; }
      return read(...args);
    });
    const pending = host.reveal(end.byte, true, 'outline');
    await waiting;
    target.parentElement!.dispatchEvent(new Event('wheel'));
    release();
    await pending;
    expect(host.captureSelection().headByte).not.toBe(end.byte + 2);
    await Promise.all([host.reveal(end.byte, true, 'outline'), host.reveal(0, true, 'outline')]);
    expect(host.captureSelection().headByte).toBe(2);
  });

  it('首次挂载前被浏览器截断的滚动目标，在内容安装后恢复实际位置', async () => {
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '正文段落\n\n'.repeat(4000) }), localBuild);
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
    const pane = document.body.appendChild(document.createElement('div'));
    pane.className = 'source-pane';
    const target = pane.appendChild(document.createElement('div'));
    let scrollTop = 0;
    Object.defineProperty(pane, 'scrollTop', {
      get: () => scrollTop,
      set: value => { scrollTop = target.querySelector('.markdown-session-window') ? value : 0; },
    });
    const failures: unknown[] = [];
    const host = new MarkdownViewportHost(runtime, target, 'source', {}, {
      onActive() {}, onFailure: error => failures.push(error), writeClipboard: async () => {},
    });
    try {
      const byte = runtime.state.index!.blocks[120].fromByte;
      host.scrollToByte(byte);
      await vi.waitFor(() => expect(scrollTop).toBeGreaterThan(0));
      expect(target.querySelector('.cm-editor')).not.toBeNull();
      expect(host.position.byteOffset).toBe(byte);
      expect(failures).toEqual([]);
    } finally { await host.dispose(); await runtime.close(true); }
  });

  it('屏内一批超过预算后先挂载安全块，不等待其余可见窗口读取', async () => {
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '安全正文\n\n'.repeat(200) }), localBuild);
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
    const target = document.body.appendChild(document.createElement('div'));
    Object.defineProperty(target, 'clientHeight', { value: 20000 });
    const read = port.readWindow.bind(port);
    let release!: () => void;
    const waiting = new Promise<void>(resolve => { release = resolve; });
    let reads = 0;
    vi.spyOn(port, 'readWindow').mockImplementation(async (...args) => {
      if (++reads === 1) await new Promise(resolve => setTimeout(resolve, 10));
      else await waiting;
      return read(...args);
    });
    const failures: unknown[] = [];
    const host = new MarkdownViewportHost(runtime, target, 'semantic', {}, {
      onActive() {}, onFailure: error => failures.push(error), writeClipboard: async () => {},
    });
    try {
      await vi.waitFor(() => expect(host.writable).toBe(true));
      expect(target.querySelectorAll('.ProseMirror')).toHaveLength(1);
      expect(host.presentation.coverage).toBe('pending');
      expect(host.presentation.timings.preview).toBeDefined();
      expect(host.presentation.timings.editable).toBeDefined();
      expect(host.presentation.timings.covered).toBeUndefined();
      expect(failures).toEqual([]);
    } finally { release(); await host.dispose(); await runtime.close(true); }
  });

  it('定位等待期间转向其他控件，完成后不重新登记旧焦点意图', async () => {
    const { host, runtime } = await setup('正文');
    const button = document.body.appendChild(document.createElement('button'));
    let release!: () => void;
    const wait = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(runtime.session, 'flushEdits').mockImplementationOnce(() => wait);
    const navigation = host.reveal(0, true);
    button.focus();
    release();
    await navigation;
    expect(document.activeElement).toBe(button);
    await host.setHidden(true);
    await host.setHidden(false);
    await vi.waitFor(() => expect(host.writable).toBe(true));
    expect(document.activeElement).toBe(button);
  });

  it('窗口读取失败保留已有正文，重试沿用会话且不抢其他控件的焦点', async () => {
    const text = '# 开头\n\n' + '正文\n\n'.repeat(5000) + '# 尾部\n';
    const { host, port, runtime, target } = await setup(text);
    const original = target.querySelector('.ProseMirror');
    const failure = vi.spyOn(port, 'readWindow').mockRejectedValueOnce(new Error('读取失败样本'));
    const button = document.body.appendChild(document.createElement('button'));
    button.focus();
    await expect(host.reveal(runtime.state.status.byteLength)).rejects.toThrow('读取失败样本');
    expect(original?.isConnected).toBe(true);
    expect(target.querySelector('[role="alert"]')?.textContent).toContain('读取失败样本');
    target.querySelector<HTMLButtonElement>('[role="alert"] button')!.click();
    await vi.waitFor(() => expect(target.querySelector('[role="alert"]')).toBeNull());
    await vi.waitFor(() => expect(host.activeCore?.canExecute({ type: 'insertText', text: '' })).toBe(true));
    expect(document.activeElement).toBe(button);
    expect(await runtime.readMarkdown() === text).toBe(true);
    failure.mockRestore();
  });

  it.each([
    ['段落', '巨型段落'.repeat(10000)],
    ['表格', '| 表头 | 内容 |\n| --- | --- |\n' + '| 行 | 内容 |\n'.repeat(4000)],
    ['列表', '- 列表条目内容\n'.repeat(4000)],
    ['HTML', '<pre>\n' + '正文内容\n'.repeat(10000) + '</pre>'],
    ['Details', '<details>\n<summary>标题</summary>\n\n' + '正文\n\n'.repeat(10000) + '</details>'],
  ])('巨型%s显示有界提示，完整会话内容仍可复制保存', async (_kind, text) => {
    const openSource = vi.fn();
    const { host, target, runtime, failures } = await setup(text, 'semantic', openSource);
    expect(target.querySelector('.markdown-session-restricted')).not.toBeNull();
    expect(target.querySelector('.cm-editor')).toBeNull();
    expect(target.textContent!.length).toBeLessThan(1000);
    expect(host.diagnostics.residentWindowBytes).toBeLessThan(32 * 1024);
    const block = runtime.state.index!.blocks.find(block => block.mode === 'raw')!;
    const byte = block.fromByte + 3;
    await host.reveal(byte, true);
    expect(host.position.byteOffset).toBe(byte);
    target.querySelector<HTMLButtonElement>('.markdown-session-restricted button')!.click();
    expect(openSource).toHaveBeenCalledWith({ fromByte: block.fromByte, toByte: block.toByte, byte });
    host.selectAll();
    expect(await runtime.selection.copy() === text).toBe(true);
    await runtime.session.withSnapshot(snapshot => runtime.session.saveCaptured(snapshot, 'fixture.md'));
    expect(await runtime.readMarkdown() === text).toBe(true);
    expect(failures).toEqual([]);
  });

  it('受限提示按实际高度更新缓存，字节导航可跨越提示到相邻正文', async () => {
    const text = '# 起点\n\n' + '巨'.repeat(20000) + '\n\n# 终点\n';
    const { host, target, runtime } = await setup(text);
    const raw = runtime.state.index!.blocks.find(block => block.mode === 'raw')!;
    await host.reveal(raw.fromByte + 3, true);
    const restricted = target.querySelector<HTMLElement>('.markdown-session-restricted')!;
    Object.defineProperty(restricted, 'offsetHeight', { value: 96 });
    Object.defineProperty(target, 'clientWidth', { value: 700 });
    const measured = host.diagnostics.measurements;
    await host.reveal(raw.fromByte + 3, true);
    await vi.waitFor(() => expect(host.diagnostics.measurements).toBeGreaterThan(measured));
    expect(host.diagnostics.cacheEntries).toBeGreaterThan(0);
    expect([...target.querySelectorAll<HTMLElement>('.markdown-session-spacer')].every(el => parseFloat(el.style.height) < 1000)).toBe(true);
    restricted.querySelector('button')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    await vi.waitFor(() => expect(host.activeCore).toBeDefined());
    expect(host.captureSelection().headByte).toBeGreaterThanOrEqual(raw.toByte);
    await runtime.session.applyEdits({ viewGeneration: runtime.session.state.viewGeneration,
      historyGroupId: 'restricted-edit', edits: [{ fromByte: raw.fromByte, toByte: raw.fromByte, insertedText: '改' }] });
    await runtime.session.undo();
    expect(await runtime.readMarkdown() === text).toBe(true);
  });

  it('部分索引首屏直接预览，同一 PM 开放编辑，等待期间不出现源码', async () => {
    let start!: () => void, finish!: () => void;
    const first = new Promise<void>(resolve => { start = resolve; });
    const tail = new Promise<void>(resolve => { finish = resolve; });
    const build: typeof buildMarkdownIndex = (port, snapshot, options) => buildMarkdownIndex(port, snapshot, {
      ...options, scan: async (port, snapshot, task) => {
        await first;
        const scanner = new MarkdownStructureScanner(snapshot.revision);
        const part = await port.readSnapshot(snapshot, 0, 1024);
        await task.onBatch(scanner.push(part.text));
        await tail;
        task.signal.throwIfAborted();
        const rest = await port.readSnapshot(snapshot, part.toByte, snapshot.byteLength);
        await task.onBatch(scanner.push(rest.text, true));
      },
    });
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '# 标题\n\n正文\n\n'.repeat(500) }), build);
    const target = document.body.appendChild(document.createElement('div'));
    const states: MarkdownPresentation[] = [];
    let preview: Element | null = null;
    let previewReadonly = false;
    const failures: unknown[] = [];
    const host = new MarkdownViewportHost(runtime, target, 'semantic', {}, {
      onActive() {}, onFailure: error => failures.push(error), writeClipboard: async () => {},
      onPresentation: state => {
        states.push(state);
        if (state.phase === 'preview') {
          preview = target.querySelector('.ProseMirror');
          previewReadonly = preview?.getAttribute('contenteditable') === 'false';
          host.updateOptions({ readonly: false });
          expect(preview?.getAttribute('contenteditable')).toBe('false');
        }
      },
    });
    let raw = false;
    const observer = new MutationObserver(() => { raw ||= !!target.querySelector('.cm-editor'); });
    observer.observe(target, { childList: true, subtree: true });
    try {
      await vi.waitFor(() => expect(target.querySelector('[role="status"]')).not.toBeNull());
      expect(host.presentation.phase).toBe('preparing');
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.VisibleText);
      expect(host.presentation.timings.preview).toBeUndefined();
      start();
      await vi.waitFor(() => expect(host.writable).toBe(true));
      expect(runtime.state.progress.state).toBe('partial');
      expect(previewReadonly).toBe(true);
      expect(target.querySelector('.ProseMirror')).toBe(preview);
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.Index);
      expect(runtime.state.status).toMatchObject({ revision: 0, dirty: false, canUndo: false });
      expect(host.presentation.coverage).toBe('semantic');
      expect(states.some(state => state.phase === 'preview')).toBe(true);
      expect(raw).toBe(false);
      expect(failures).toEqual([]);
    } finally { start(); finish(); observer.disconnect(); await host.dispose(); await runtime.close(true); }
  });

  it('预览中的写入门禁保护全局输入，源码宿主独立可编辑，隐藏撤销提权', async () => {
    let release!: () => void;
    const wait = new Promise<void>(resolve => { release = resolve; });
    const port = createMemoryMarkdownPort();
    const runtime = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '正文' }), async (...args) => {
      await wait; return localBuild(...args);
    });
    const target = document.body.appendChild(document.createElement('div'));
    const sourceTarget = document.body.appendChild(document.createElement('div'));
    const callbacks = { onActive() {}, onFailure(error: unknown) { throw error; }, writeClipboard: async () => {} };
    const host = new MarkdownViewportHost(runtime, target, 'semantic', {}, callbacks);
    const source = new MarkdownViewportHost(runtime, sourceTarget, 'source', {}, callbacks);
    try {
      await vi.waitFor(() => expect(source.writable).toBe(true));
      expect(host.writable).toBe(false);
      host.selectAll();
      const input = target.querySelector('textarea')!;
      expect(input.readOnly).toBe(true);
      input.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'insertText', data: '误输入' }));
      expect(runtime.state.status.revision).toBe(0);
      await host.setHidden(true);
      expect(runtime.structurePriority).toBe(MarkdownTaskPriority.Index);
      expect(source.writable).toBe(true);
    } finally { release(); await host.dispose(); await source.dispose(); await runtime.close(true); }
  });

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
