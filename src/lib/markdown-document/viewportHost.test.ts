// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProgressiveMarkdownDocument } from './runtime';
import { MarkdownViewportHost, type MarkdownPresentation } from './viewportHost';
import { buildMarkdownIndex } from './incrementalIndex';
import { MarkdownStructureScanner } from './structure';
import { createMemoryMarkdownPort } from './port';
import { MarkdownTaskPriority } from './scheduler';
import { ProseMirrorEditorCore } from '../editor-core/ProseMirrorEditorCore';
import { estimateMarkdownBlockHeight } from './viewport';
import { MarkdownSemanticWindow } from './semanticWindow';
import { markdownResidentBudget } from './residentBudget';
import { resolveMarkdownWindowingLimits } from './windowingLimits';

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
  const defaults = resolveMarkdownWindowingLimits(100);
  markdownResidentBudget(document).updateLimits(defaults.resident, defaults.mount);
  expect(markdownResidentBudget(document).diagnostics).toMatchObject({ hosts: 0, documents: 0, entries: 0,
    actual: { views: 0, sourceBytes: 0, domNodes: 0, imageBytes: 0 },
    reserved: { views: 0, sourceBytes: 0, domNodes: 0, imageBytes: 0 } });
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

function outlineGeometry(target: HTMLElement, positions: number[]) {
  const pane = target.parentElement!;
  Object.defineProperty(pane, 'clientHeight', { value: 800 });
  Object.defineProperty(pane, 'scrollHeight', { value: 8000 });
  const writes: number[] = [];
  let top = 0, pending = false;
  Object.defineProperty(pane, 'scrollTop', {
    get: () => top,
    set: value => {
      top = Math.min(7200, Math.max(0, value));
      writes.push(top);
      // 浏览器合并程序滚动事件，不能把动画自己的帧误判为用户接管。
      if (!pending) {
        pending = true;
        queueMicrotask(() => { pending = false; pane.dispatchEvent(new Event('scroll')); });
      }
    },
  });
  pane.getBoundingClientRect = () => new DOMRect(0, 100, 500, 800);
  target.getBoundingClientRect = () => new DOMRect(0, 300 - top, 500, 5000);
  for (const view of target.querySelectorAll<HTMLElement>('.markdown-session-window')) Object.defineProperty(view, 'offsetHeight', { value: 5000, configurable: true });
  target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
  const headings = [...target.querySelectorAll<HTMLElement>('h1')];
  headings.forEach((heading, i) => {
    heading.getBoundingClientRect = () => new DOMRect(0, 300 + positions[i] - top, 500, 40);
  });
  return { pane, writes, headings };
}

describe('完整会话的真实窗口宿主', () => {
  it('异步增强触发共享压力，隐藏及销毁释放登记，保护中的全文编辑和撤销仍完整', async () => {
    const text = '# 开头\n\n' + '完整正文\n\n'.repeat(300) + '末段';
    const first = await setup(text), second = await setup('另一个宿主', 'source');
    const budget = markdownResidentBudget(document);
    expect(budget.diagnostics.hosts).toBe(2);
    const root = first.target.querySelector('.markdown-session-window')!;
    const image = document.createElement('img'); image.src = '/large.png';
    Object.defineProperties(image, { complete: { value: true }, naturalWidth: { value: 8192 }, naturalHeight: { value: 8192 } });
    root.append(image);
    await vi.waitFor(() => expect(first.runtime.state.mountRestriction).toBe('resource-pressure'));
    expect(second.runtime.state.mountMode).toBe('windowed');
    expect(budget.diagnostics.actual.imageBytes).toBe(8192 * 8192 * 4);
    expect(budget.diagnostics.protectedExcess.imageBytes).toBeGreaterThan(0);
    await first.host.reveal(first.runtime.state.status.byteLength, true);
    first.host.activeCore!.execute({ type: 'insertText', text: '！' });
    await first.runtime.session.flushEdits();
    expect(await first.runtime.readMarkdown()).toBe(text + '！');
    await first.runtime.session.undo();
    expect(await first.runtime.readMarkdown()).toBe(text);
    await second.host.setHidden(true);
    expect(budget.diagnostics.hosts).toBe(1);
    await second.host.setHidden(false);
    await vi.waitFor(() => expect(budget.diagnostics.hosts).toBe(2));
    await second.host.dispose(); await first.host.dispose();
    expect(budget.diagnostics).toMatchObject({ hosts: 0, entries: 0, documents: 0,
      actual: { views: 0, sourceBytes: 0, domNodes: 0, imageBytes: 0 },
      reserved: { views: 0, sourceBytes: 0, domNodes: 0, imageBytes: 0 } });
    expect([...first.failures, ...second.failures]).toEqual([]);
  });

  it('标题调度直接使用已确认字节位置，不反复计算原文光标映射', async () => {
    const { host, runtime, target } = await setup('# 一\n\n| 列 | 内容 |\n| --- | --- |\n| a | b |\n\n# 二\n\n尾声');
    outlineGeometry(target, [0, 900]);
    const selection = vi.spyOn(host as any, 'selection');
    await host.reveal(runtime.state.index!.outline[1].byte, false, 'outline');
    expect(selection).not.toHaveBeenCalled();
    selection.mockRestore();
  });

  it('导航历史最多 32 条标量样本，销毁后清空', async () => {
    const { host } = await setup('# 标题\n\n正文');
    for (let i = 0; i < 35; i++) await host.reveal(0, false, 'outline');
    expect(host.diagnostics.navigations).toHaveLength(32);
    expect(host.diagnostics.navigations.every(sample => !('context' in sample))).toBe(true);
    await host.dispose();
    expect(host.diagnostics.navigations).toHaveLength(0);
  });
  it('旧视口在途读取未返回时，目标读取独立推进且旧结果不发布', async () => {
    const { host, runtime, port, target } = await setup('# 开头\n\n' + '正文\n\n'.repeat(2500) + '# 终点\n\n尾声');
    const end = runtime.state.index!.outline.at(-1)!;
    let release!: () => void, entered!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const waiting = new Promise<void>(resolve => { entered = resolve; });
    const read = port.readWindow.bind(port);
    const reads = vi.spyOn(port, 'readWindow').mockImplementation(async (...args) => {
      if (args[2] > 5000 && args[2] < end.byte - 5000) { entered(); await gate; }
      return read(...args);
    });
    const restore = vi.spyOn(ProseMirrorEditorCore.prototype, 'restoreSourceSelection');
    try {
      host.scrollToByte(Math.floor(end.byte / 2));
      await waiting;
      await host.reveal(end.byte, false, 'outline');
      expect(target.textContent).toContain('终点');
      expect(reads.mock.calls.some(args => args[2] > end.byte - 5000)).toBe(true);
      expect(restore).not.toHaveBeenCalled();
      release();
      await new Promise(resolve => setTimeout(resolve, 40));
      expect(host.activeOutlineId).toBe(end.id);
    } finally { release(); reads.mockRestore(); restore.mockRestore(); }
  });

  it('局部资源失效保留无关布局及高度索引，宽度变化仍全局失效', async () => {
    const { host, target } = await setup('# 标题\n\n正文');
    const internal = host as any;
    const heights = internal.heights;
    internal.layoutCache.put('unrelated', { height: 777 });
    target.querySelector('.markdown-session-window')!.dispatchEvent(new Event('load', { bubbles: true }));
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(internal.heights).toBe(heights);
    expect(internal.layoutCache.get('unrelated')?.height).toBe(777);
    Object.defineProperty(target, 'clientWidth', { value: 900 });
    await host.reveal(0);
    expect(internal.heights).not.toBe(heights);
    internal.onFont();
    expect(internal.layoutCache.get('unrelated')).toBeUndefined();
  });

  it('减少动态效果使用同一 140ms 时钟，正文就绪和动画完成分别记录', async () => {
    const { host, target, runtime } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
    outlineGeometry(target, [0, 900]);
    const original = window.matchMedia;
    window.matchMedia = (() => ({ matches: true })) as unknown as typeof window.matchMedia;
    try {
      await host.reveal(runtime.state.index!.outline[1].byte, false, 'outline');
      expect(host.diagnostics.navigation).toMatchObject({ duration: 140, status: 'ready', deadlineMiss: false });
      expect(host.diagnostics.navigation!.animationMs).toBeGreaterThanOrEqual(140);
      expect(host.diagnostics.navigation!.basicReadyMs).toBeLessThan(140);
    } finally { window.matchMedia = original; }
  });
  it('已挂载内容的动画不等待异步布局循环，后台读取挂起时仍按真实时间到达', async () => {
    const { runtime, host, target } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    const { pane } = outlineGeometry(target, [0, 900]);
    const pending = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
    await vi.waitFor(() => expect(pane.scrollTop).toBeGreaterThan(0));
    let release!: () => void, entered!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const waiting = new Promise<void>(resolve => { entered = resolve; });
    const transactions = vi.spyOn(runtime.session, 'awaitTransactions').mockImplementation(async () => { entered(); await gate; });
    try {
      target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
      await waiting;
      await vi.waitFor(() => expect(pane.scrollTop).toBeCloseTo(940), { timeout: 450 });
      await pending;
    } finally { release(); transactions.mockRestore(); }
  });

  it('帧间隔达到 100ms 时计入真实经过时间，不把 260ms 动画拉长为逐次 32ms 推进', async () => {
    const { runtime, host, target } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    await vi.waitFor(() => expect(host.writable).toBe(true));
    const { pane } = outlineGeometry(target, [0, 900]);
    const raf = vi.spyOn(globalThis, 'requestAnimationFrame').mockImplementation(callback =>
      window.setTimeout(() => callback(performance.now()), 100));
    const cancel = vi.spyOn(globalThis, 'cancelAnimationFrame').mockImplementation(id => window.clearTimeout(id));
    try {
      const pending = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
      await vi.waitFor(() => expect(pane.scrollTop).toBeCloseTo(940), { timeout: 500 });
      await pending;
    } finally { host.cancelOutlineNavigation(); raf.mockRestore(); cancel.mockRestore(); }
  });

  it('大纲固定时钟滚动，途中资源修正剩余轨迹，到达后局部保持位置', async () => {
    const { runtime, host, target, failures } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    const positions = [0, 900];
    const { pane, writes } = outlineGeometry(target, positions);
    const pending = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
    await vi.waitFor(() => expect(pane.scrollTop).toBeGreaterThan(0));
    expect(pane.scrollTop).toBeLessThan(940);
    positions[1] += 240;
    target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
    await pending;
    expect(pane.scrollTop).toBeCloseTo(1180);
    expect(writes.length).toBeGreaterThan(3);
    expect(writes.every((top, i) => !i || top >= writes[i - 1] - 1)).toBe(true);
    const settledCount = writes.length;
    positions[1] += 120;
    target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
    await vi.waitFor(() => expect(pane.scrollTop).toBeCloseTo(1300));
    expect(writes[settledCount]).toBeGreaterThan(1180);
    expect(writes[settledCount]).toBeCloseTo(1300);
    expect(failures).toEqual([]);
  });

  it('连续点击从动画当前位置转向最后目标，首章在正文顶部偏移之前也能准确到达', async () => {
    const { runtime, host, target } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
    const { pane, writes } = outlineGeometry(target, [0, 900]);
    const first = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
    await vi.waitFor(() => expect(pane.scrollTop).toBeGreaterThan(40));
    const turningTop = pane.scrollTop;
    writes.length = 0;
    const last = host.reveal(0, true, 'outline');
    expect(pane.scrollTop).toBe(turningTop);
    await Promise.all([first, last]);
    expect(pane.scrollTop).toBeCloseTo(40);
    expect(writes[0]).toBeGreaterThan(40);
    expect(writes[0]).toBeLessThan(turningTop);
    expect(host.activeOutlineId).toBe(runtime.state.index!.outline[0].id);
  });

  it.each([
    { windowLimited: false, mountMode: 'windowed' as const },
    { windowLimited: false, mountMode: 'progressive-full' as const },
    { windowLimited: true, mountMode: 'windowed' as const },
  ])('未加载远跳只在受限窗口模式直跳，$mountMode / $windowLimited', async ({ windowLimited, mountMode }) => {
    const text = '# 起点\n\n' + Array.from({ length: 2000 }, (_, i) => `第${String(i).padStart(4, '0')}段结束\n\n`).join('') + '# 终点\n\n尾声';
    const { runtime, host, port, target, failures } = await setup(text);
    const getState = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(runtime), 'state')!.get!;
    const state = vi.spyOn(runtime, 'state', 'get').mockImplementation(() => ({ ...getState.call(runtime), windowLimited, mountMode }));
    const blocks = runtime.state.index!.blocks;
    const lines = text.split('\n');
    const tops: number[] = [];
    let total = 0;
    for (const block of blocks) { tops.push(total); total += estimateMarkdownBlockHeight(block); }
    const end = runtime.state.index!.outline[1];
    const destination = tops[blocks.findIndex(block => block.id === end.blockId)];
    const pane = target.parentElement!;
    let top = 0;
    const uncovered: number[] = [], writes: number[] = [];
    Object.defineProperty(pane, 'clientHeight', { value: 800 });
    Object.defineProperty(pane, 'scrollHeight', { value: total });
    Object.defineProperty(pane, 'scrollTop', {
      get: () => top,
      set: value => {
        top = Math.min(total - 800, Math.max(0, value));
        writes.push(top);
        for (const point of [top, Math.min(top + 799, total - 1)]) {
          let i = tops.length - 1;
          while (tops[i] > point) i--;
          const marker = lines[blocks[i].fromLine - 1].replace(/^# /, '');
          if (![...target.querySelectorAll('.markdown-session-window')].some(view => view.textContent?.includes(marker))) uncovered.push(point);
        }
      },
    });
    pane.getBoundingClientRect = () => new DOMRect(0, 100, 500, 800);
    target.getBoundingClientRect = () => new DOMRect(0, 100 - top, 500, total);
    const measured = vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function(this: HTMLElement) {
      if (!this.classList.contains('markdown-session-window')) return 0;
      const first = blocks.findIndex(block => this.textContent?.includes(lines[block.fromLine - 1].replace(/^# /, '')));
      if (first < 0) return 0;
      return blocks.slice(first, first + 64).reduce((sum, block) => sum + estimateMarkdownBlockHeight(block), 0);
    });
    const geometry = vi.spyOn(ProseMirrorEditorCore.prototype, 'headingAnchorTop').mockImplementation(id =>
      100 + (id === end.id ? destination : 0) - top);
    const read = port.readWindow.bind(port);
    let release!: () => void, started!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const waiting = new Promise<void>(resolve => { started = resolve; });
    const reads = vi.spyOn(port, 'readWindow').mockImplementation(async (...args) => {
      if (args[2] > 0) { started(); await gate; }
      return read(...args);
    });
    const connectedNotifications: boolean[] = [];
    const onNavigation = () => {
      const destinationView = (host as any).mounted.find((view: any) =>
        end.byte >= view.window.fromByte && end.byte < view.window.toByte);
      if (destinationView?.element.isConnected)
        connectedNotifications.push(destinationView.semantic.bodyReady);
    };
    target.addEventListener('nomo:navigation-render', onNavigation);
    try {
      host.focus();
      const originFocus = document.activeElement;
      const pending = host.reveal(end.byte, true, 'outline');
      await waiting;
      await new Promise(resolve => setTimeout(resolve, 80));
      expect(target.textContent).toContain('起点');
      if (windowLimited) {
        expect(top).toBe(0);
        expect(document.activeElement).toBe(originFocus);
        expect(host.diagnostics.navigation).toMatchObject({ kind: 'direct', targetLoaded: false, status: 'preparing' });
        expect(host.diagnostics.navigation?.animationMs).toBeUndefined();
      } else {
        expect(top).toBeGreaterThan(0);
        await vi.waitFor(() => expect(host.diagnostics.navigation?.animationMs).toBeGreaterThanOrEqual(260));
        expect(host.diagnostics.navigation?.deadlineMiss).toBe(true);
        expect(top).toBeCloseTo(Math.min(total - 800, destination - 160));
      }
      expect(reads.mock.calls[0][2]).toBeGreaterThan(end.byte - 5000);
      release();
      await pending;
      expect(host.diagnostics.navigation?.status).toBe(windowLimited ? 'ready' : 'late');
      expect(writes.some(value => value > 0 && value < top)).toBe(!windowLimited);
      if (windowLimited) {
        expect(host.diagnostics.navigation?.duration).toBeUndefined();
        expect(host.diagnostics.navigation?.deadlineMiss).toBeUndefined();
        expect(host.diagnostics.navigation?.positionedMs).toBeGreaterThanOrEqual(80);
      } else expect(uncovered.length).toBeGreaterThan(0);
      expect(host.diagnostics.peakMounted).toBeLessThan(20);
      expect(failures).toEqual([]);
      expect(connectedNotifications.length).toBeGreaterThan(0);
      expect(connectedNotifications.every(Boolean)).toBe(true);
    } finally { target.removeEventListener('nomo:navigation-render', onNavigation); release(); measured.mockRestore(); geometry.mockRestore(); reads.mockRestore(); state.mockRestore(); }
  }, 10000);

  it.each(['windowed', 'progressive-full'] as const)('已加载目标在 %s 中始终保留动画', async mountMode => {
    const { runtime, host, target } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
    outlineGeometry(target, [0, 900]);
    await vi.waitFor(() => expect(host.writable).toBe(true));
    const getState = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(runtime), 'state')!.get!;
    const state = vi.spyOn(runtime, 'state', 'get').mockImplementation(() => ({ ...getState.call(runtime), mountMode, windowLimited: true }));
    try {
      await host.reveal(runtime.state.index!.outline[1].byte, false, 'outline');
      expect(host.diagnostics.navigation).toMatchObject({ kind: 'animated', targetLoaded: true, duration: 260 });
      expect(host.diagnostics.navigation!.animationMs).toBeGreaterThanOrEqual(260);
    } finally { state.mockRestore(); }
  });

  it('目标正文已加载而邻接首屏未就绪，超时仍保留动画且不会转成直跳', async () => {
    const { runtime, host, target } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
    outlineGeometry(target, [0, 900]);
    await vi.waitFor(() => expect(host.writable).toBe(true));
    const getState = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(runtime), 'state')!.get!;
    const state = vi.spyOn(runtime, 'state', 'get').mockImplementation(() => ({ ...getState.call(runtime), mountMode: 'windowed', windowLimited: true }));
    const coverage = vi.spyOn(host as any, 'outlineViewportCovered').mockReturnValue(false);
    try {
      const pending = host.reveal(runtime.state.index!.outline[1].byte, false, 'outline');
      await vi.waitFor(() => expect(host.diagnostics.navigation?.animationMs).toBeGreaterThanOrEqual(260));
      expect(host.diagnostics.navigation).toMatchObject({ kind: 'animated', targetLoaded: true, deadlineMiss: true });
      expect(host.diagnostics.navigation?.reusedViewport).not.toBe(true);
      coverage.mockRestore();
      target.querySelector('.markdown-session-window')!.dispatchEvent(new Event('load', { bubbles: true }));
      await pending;
      expect(host.diagnostics.navigation?.kind).toBe('animated');
    } finally { coverage.mockRestore(); state.mockRestore(); }
  });

  it.each([140, 260])('完整已测量首屏复用窗口，%ims 动画期间不重做挂载计划，失效后恢复准备', async duration => {
    const { runtime, host, target, port } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
    const { pane } = outlineGeometry(target, [0, 900]);
    await host.reveal(0, false, 'outline');
    await new Promise(resolve => setTimeout(resolve, 60));
    const reads = vi.spyOn(port, 'readWindow');
    const plans = vi.spyOn((host as any).demand, 'range');
    const matchMedia = window.matchMedia;
    window.matchMedia = (() => ({ matches: duration === 140 })) as unknown as typeof window.matchMedia;
    try {
      const pending = host.reveal(runtime.state.index!.outline[1].byte, false, 'outline');
      await vi.waitFor(() => expect(host.diagnostics.navigation?.reusedViewport).toBe(true));
      expect(host.diagnostics.navigation).toMatchObject({ kind: 'animated', targetLoaded: true, duration });
      expect(reads).not.toHaveBeenCalled();
      expect(plans).not.toHaveBeenCalled();
      expect(pane.scrollTop).toBeGreaterThan(40);
      (host as any).onFont();
      await pending;
      expect(host.diagnostics.navigation!.animationMs).toBeGreaterThanOrEqual(duration);
      expect(plans).toHaveBeenCalled();
      expect(pane.scrollTop).toBeCloseTo(940);
      expect(host.diagnostics.navigation?.kind).toBe('animated');
    } finally { reads.mockRestore(); plans.mockRestore(); window.matchMedia = matchMedia; }
  });

  it.each(['wheel', 'touchstart', 'pointerdown', 'keydown', 'hidden', 'dispose', 'revision'])(
    '直跳准备中 %s 接管后不再定位旧目标', async action => {
      const { runtime, host, target } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
      const { pane } = outlineGeometry(target, [0, 900]);
      const getState = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(runtime), 'state')!.get!;
      const state = vi.spyOn(runtime, 'state', 'get').mockImplementation(() => ({ ...getState.call(runtime), mountMode: 'windowed', windowLimited: true }));
      const readiness = vi.spyOn(MarkdownSemanticWindow.prototype, 'bodyReady', 'get').mockReturnValue(false);
      try {
        const pending = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
        await vi.waitFor(() => expect(host.diagnostics.navigation?.kind).toBe('direct'));
        expect(pane.scrollTop).toBe(0);
        if (action === 'hidden') await host.setHidden(true);
        else if (action === 'dispose') await host.dispose();
        else if (action === 'revision') await runtime.session.applyEdits({
          viewGeneration: runtime.session.state.viewGeneration, historyGroupId: 'cancel-direct',
          edits: [{ fromByte: runtime.state.status.byteLength, toByte: runtime.state.status.byteLength, insertedText: '\n追加' }],
        });
        else if (action === 'keydown') target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        else pane.dispatchEvent(new Event(action));
        readiness.mockRestore();
        await pending;
        target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
        await new Promise(resolve => setTimeout(resolve, 50));
        expect(host.diagnostics.navigation).toBeUndefined();
        expect(pane.scrollTop).toBeLessThan(200);
      } finally { readiness.mockRestore(); state.mockRestore(); }
    },
  );

  it('直跳准备中再次点击已加载标题，只保留最后一次动画导航', async () => {
    const { runtime, host, target } = await setup('# 一\n\n正文\n\n# 二\n\n尾声');
    const { pane } = outlineGeometry(target, [0, 900]);
    const getState = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(runtime), 'state')!.get!;
    const state = vi.spyOn(runtime, 'state', 'get').mockImplementation(() => ({ ...getState.call(runtime), mountMode: 'windowed', windowLimited: true }));
    const readiness = vi.spyOn(MarkdownSemanticWindow.prototype, 'bodyReady', 'get').mockReturnValue(false);
    try {
      const first = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
      await vi.waitFor(() => expect(host.diagnostics.navigation?.kind).toBe('direct'));
      readiness.mockRestore();
      const last = host.reveal(0, true, 'outline');
      await Promise.all([first, last]);
      expect(host.diagnostics.navigation).toMatchObject({ byte: 0, kind: 'animated', targetLoaded: true });
      expect(host.diagnostics.navigations.at(-2)?.status).toBe('cancelled');
      expect(pane.scrollTop).toBeCloseTo(40);
    } finally { readiness.mockRestore(); state.mockRestore(); }
  });

  it('直跳目标读取失败保留原位置和正文，明确记录失败且不跳估算位置', async () => {
    const { runtime, host, port, target } = await setup('# 起点\n\n' + '正文\n\n'.repeat(6500) + '# 终点\n\n尾声');
    const pane = target.parentElement!;
    Object.defineProperty(pane, 'clientHeight', { value: 800 });
    Object.defineProperty(pane, 'scrollHeight', { value: 100000 });
    expect(runtime.state.windowLimited).toBe(true);
    const reads = vi.spyOn(port, 'readWindow').mockRejectedValueOnce(new Error('直跳读取失败'));
    try {
      await expect(host.reveal(runtime.state.index!.outline.at(-1)!.byte, true, 'outline')).rejects.toThrow('直跳读取失败');
      expect(pane.scrollTop).toBe(0);
      expect(target.textContent).toContain('起点');
      expect(target.querySelector('[role="alert"]')?.textContent).toContain('直跳读取失败');
      expect(host.diagnostics.navigations.at(-1)).toMatchObject({ kind: 'direct', status: 'failed' });
    } finally { reads.mockRestore(); }
  });

  it('直跳首屏邻接受限结构时保留原位置并结束等待，不把提示判为正文', async () => {
    const text = '# 起点\n\n正文\n\n' + '巨'.repeat(20000) + '\n\n# 终点\n\n' + '后续正文\n\n'.repeat(6500);
    const { host, target, runtime, failures } = await setup(text);
    const { pane } = outlineGeometry(target, [0, 900]);
    for (const view of target.querySelectorAll<HTMLElement>('.markdown-session-window')) {
      Object.defineProperty(view, 'offsetHeight', { value: 96, configurable: true });
      view.dispatchEvent(new Event('load', { bubbles: true }));
    }
    await new Promise(resolve => setTimeout(resolve, 50));
    const readiness = vi.spyOn(host as any, 'isOutlineTargetLoaded').mockReturnValue(false);
    const geometry = vi.spyOn(ProseMirrorEditorCore.prototype, 'headingAnchorTop').mockReturnValue(300);
    try {
      await host.reveal(0, false, 'outline');
      expect(runtime.state.windowLimited).toBe(true);
      expect(pane.scrollTop).toBe(0);
      expect(host.diagnostics.navigations.at(-1)).toMatchObject({ kind: 'direct', status: 'restricted' });
      expect(failures.some(error => String(error).includes('目标首屏'))).toBe(true);
      expect(host.diagnostics.navigation).toBeUndefined();
    } finally { readiness.mockRestore(); geometry.mockRestore(); }
  });

  it.each(['wheel', 'touchstart', 'pointerdown', 'keydown', 'hidden', 'dispose', 'revision'])(
    '大纲动画中 %s 接管会释放旧动画，迟到的资源事件不会重新拉回', async action => {
      const { runtime, host, target } = await setup('# 第一章\n\n正文\n\n# 第二章\n\n结尾');
      const positions = [0, 900];
      const { pane } = outlineGeometry(target, positions);
      const pending = host.reveal(runtime.state.index!.outline[1].byte, true, 'outline');
      await vi.waitFor(() => expect(pane.scrollTop).toBeGreaterThan(0));
      const stoppedAt = pane.scrollTop;
      if (action === 'hidden') await host.setHidden(true);
      else if (action === 'dispose') await host.dispose();
      else if (action === 'revision') await runtime.session.applyEdits({
        viewGeneration: runtime.session.state.viewGeneration, historyGroupId: 'cancel-outline',
        edits: [{ fromByte: runtime.state.status.byteLength, toByte: runtime.state.status.byteLength, insertedText: '\n追加' }],
      });
      else if (action === 'keydown') target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      else pane.dispatchEvent(new Event(action));
      await pending;
      positions[1] += 200;
      target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
      await new Promise(resolve => setTimeout(resolve, 180));
      expect(pane.scrollTop).toBeCloseTo(stoppedAt);
    },
  );

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
    for (const view of target.querySelectorAll<HTMLElement>('.markdown-session-window')) Object.defineProperty(view, 'offsetHeight', { value: 5000, configurable: true });
  target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
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
    target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
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
    for (const view of target.querySelectorAll<HTMLElement>('.markdown-session-window')) Object.defineProperty(view, 'offsetHeight', { value: 1200 });
    target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
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
    for (const view of target.querySelectorAll<HTMLElement>('.markdown-session-window')) Object.defineProperty(view, 'offsetHeight', { value: 3000 });
    target.querySelector('.markdown-session-window')?.dispatchEvent(new Event('load', { bubbles: true }));
    vi.spyOn(host.activeSource!, 'byteTop').mockImplementation(byte => 100 + (byte === second.byte ? 900 : 0) - pane.scrollTop);
    await host.reveal(second.byte, true, 'outline');
    expect(pane.scrollTop).toBeCloseTo(740);
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
  it('调低比例收缩现有视图，保留可见 DOM 与完整正文，再调高后空闲恢复', async () => {
    const text = ('x'.repeat(1022) + '\n\n').repeat(900);
    const { runtime, host, target, failures } = await setup(text);
    const budget = markdownResidentBudget(document);
    await vi.waitFor(() => expect(host.diagnostics.mounted).toBeGreaterThan(2));
    const before = host.diagnostics.mounted;
    const first = target.querySelector('.markdown-session-window');
    const smaller = resolveMarkdownWindowingLimits(75);
    budget.updateLimits(smaller.resident, smaller.mount);
    runtime.updateMountLimits(smaller.mount);
    await vi.waitFor(() => expect(host.diagnostics.mounted).toBeLessThan(before));
    const shrunk = host.diagnostics.mounted;
    expect(runtime.state.mountMode).toBe('windowed');
    expect(target.querySelector('.markdown-session-window')).toBe(first);
    expect(await runtime.readMarkdown()).toBe(text);
    const larger = resolveMarkdownWindowingLimits(200);
    budget.updateLimits(larger.resident, larger.mount);
    runtime.updateMountLimits(larger.mount);
    await vi.waitFor(() => expect(runtime.state.mountMode).toBe('progressive-full'), { timeout: 6500 });
    await vi.waitFor(() => expect(host.diagnostics.mounted).toBeGreaterThan(shrunk));
    expect(target.querySelector('.markdown-session-window')).toBe(first);
    expect(runtime.state.status.revision).toBe(0);
    expect(failures).toEqual([]);
  }, 15000);

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
    const text = '# 重复标题\n\n' + '段落内容\n\n'.repeat(6500) + '# 重复标题\n\n结尾';
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

  it('共享压力下全选的中文组字提交替换完整正文，组字结束前不提交中间文本', async () => {
    const text = '原始正文\n\n'.repeat(3500);
    const { runtime, host, target, failures } = await setup(text, 'source');
    const budget = markdownResidentBudget(document), other = {};
    budget.register(other, 'composition-pressure', () => {});
    cleanups.push(async () => budget.unregister(other));
    budget.reserve(other, { views: 33, sourceBytes: 0, domNodes: 0, imageBytes: 0 }, true);
    await vi.waitFor(() => expect(runtime.state.mountRestriction).toBe('resource-pressure'));
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
