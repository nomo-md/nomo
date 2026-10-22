// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { navigationRenderAllowed, observeRenderVisibility, setNavigationRenderContext,
  trackNavigationEnhancement, type NavigationRenderContext } from './renderVisibility';

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

it('直跳准备阶段不借用动画截止时间，定位释放后恢复增强资格', () => {
  const host = document.body.appendChild(document.createElement('div'));
  host.className = 'markdown-session-host';
  const child = host.appendChild(document.createElement('div'));
  const deferred = vi.fn();
  setNavigationRenderContext(host, { revision: 0, generation: 1, preparationOnly: true,
    signal: new AbortController().signal, ready: true, targets: new Set([child]),
    viewport: () => ({ top: 0, bottom: 800 }), deferred, started: () => () => {} });
  for (const kind of ['code', 'typography', 'diagram'] as const)
    expect(navigationRenderAllowed(child, kind)).toBe(false);
  expect(deferred).not.toHaveBeenCalled();
  setNavigationRenderContext(host, undefined);
  expect(navigationRenderAllowed(child, 'code')).toBeUndefined();
});

it('宿主只给目标首屏增强资格，正文未就绪和预算不足时让步，释放后恢复可见性', () => {
  const host = document.body.appendChild(document.createElement('div'));
  host.className = 'markdown-session-host';
  const target = host.appendChild(document.createElement('div'));
  const path = host.appendChild(document.createElement('div'));
  target.getBoundingClientRect = () => new DOMRect(0, 3000, 500, 100);
  const deferred = vi.fn(), changed = vi.fn();
  const controller = new AbortController();
  const context: NavigationRenderContext = { revision: 1, generation: 2, deadline: performance.now() + 260,
    signal: controller.signal, ready: false, targets: new Set([target]), viewport: () => ({ top: 2900, bottom: 3700 }),
    deferred, started: () => () => {} };
  const stop = observeRenderVisibility(target, changed);
  setNavigationRenderContext(host, context);
  expect(changed).toHaveBeenLastCalledWith(false);
  expect(navigationRenderAllowed(path, 'code')).toBe(false);
  context.ready = true; setNavigationRenderContext(host, context);
  expect(changed).toHaveBeenLastCalledWith(true);
  expect(navigationRenderAllowed(path, 'diagram')).toBe(false);
  context.deadline = performance.now() + 10;
  expect(navigationRenderAllowed(target, 'code')).toBe(false);
  setNavigationRenderContext(host, context);
  expect(deferred).toHaveBeenCalled();
  setNavigationRenderContext(host, undefined);
  expect(navigationRenderAllowed(target, 'code')).toBeUndefined();
  stop();
});

it('取消导航中止在途增强，旧任务结算一次且不会影响其他宿主', () => {
  const host = document.body.appendChild(document.createElement('div'));
  host.className = 'markdown-session-host';
  const child = host.appendChild(document.createElement('div'));
  const controller = new AbortController(), work = new AbortController(), finished = vi.fn();
  setNavigationRenderContext(host, { revision: 0, generation: 1, deadline: performance.now() + 260,
    signal: controller.signal, ready: true, targets: new Set([child]), viewport: () => ({ top: 0, bottom: 800 }),
    deferred() {}, started: () => finished });
  const finish = trackNavigationEnhancement(child, 'diagram', work);
  controller.abort(); expect(work.signal.aborted).toBe(true);
  finish(); expect(finished).toHaveBeenCalledOnce();
  const unrelated = document.body.appendChild(document.createElement('div'));
  expect(navigationRenderAllowed(unrelated, 'diagram')).toBeUndefined();
  setNavigationRenderContext(host, undefined);
});
