// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { TableLayoutController } from './dom';
import { setNavigationRenderContext, type NavigationRenderContext } from '../editor-core/nodeViews/renderVisibility';

describe('表格纯数据布局恢复', () => {
  it('导航提前准备目标表格列宽，沿途让路，强制导出不受导航门禁限制', async () => {
    const root = document.body.appendChild(document.createElement('div'));
    root.className = 'markdown-session-host';
    const controller = new TableLayoutController(root);
    const wrappers = [0, 3000].map(top => {
      const wrapper = root.appendChild(document.createElement('div'));
      Object.defineProperty(wrapper, 'clientWidth', { value: 800 });
      wrapper.getBoundingClientRect = () => new DOMRect(0, top, 800, 200);
      const table = wrapper.appendChild(document.createElement('table'));
      table.style.font = '14px Arial';
      const colgroup = table.appendChild(document.createElement('colgroup'));
      controller.register({ table, wrapper, colgroup, columns: () => 2, cells: () => [] });
      return { wrapper, colgroup };
    });
    const context: NavigationRenderContext = { revision: 0, generation: 1, deadline: performance.now() + 2000,
      signal: new AbortController().signal, ready: false, targets: new Set([wrappers[1].wrapper]),
      viewport: () => ({ top: 2900, bottom: 3700 }), deferred() {}, started: () => () => {} };
    try {
      setNavigationRenderContext(root, context);
      await new Promise(resolve => setTimeout(resolve, 30));
      expect(wrappers[1].colgroup.childElementCount).toBe(0);
      context.ready = true; setNavigationRenderContext(root, context);
      await vi.waitFor(() => expect(wrappers[1].colgroup.childElementCount).toBe(2));
      expect(wrappers[0].colgroup.childElementCount).toBe(0);
      await controller.flush();
      expect(wrappers[0].colgroup.childElementCount).toBe(2);
    } finally { setNavigationRenderContext(root, undefined); controller.destroy(); root.remove(); }
  });
  it('同组远处表格未就绪时仍保留已完成列宽，并按原序号恢复', async () => {
    const root = document.body.appendChild(document.createElement('div'));
    const controller = new TableLayoutController(root);
    const add = (width: number) => {
      const wrapper = root.appendChild(document.createElement('div'));
      Object.defineProperty(wrapper, 'clientWidth', { configurable: true, value: width });
      const table = wrapper.appendChild(document.createElement('table'));
      table.style.font = '14px Arial';
      const colgroup = table.appendChild(document.createElement('colgroup'));
      controller.register({ table, wrapper, colgroup, columns: () => 2, cells: () => [] });
      return colgroup;
    };
    try {
      add(0);
      const ready = add(800);
      await controller.flush();
      const saved = controller.snapshot()!;
      expect(saved.map(table => table.index)).toEqual([1]);
      const widths = ready.innerHTML;
      ready.replaceChildren();
      expect(controller.restore(saved)).toBe(true);
      expect(ready.innerHTML).toBe(widths);
      expect(root.querySelector('colgroup')!.childElementCount).toBe(0);
    } finally { controller.destroy(); root.remove(); }
  });
  it('释放控制器后可恢复列宽，宽度或内容变化不导出旧结果', async () => {
    const create = () => {
      const root = document.createElement('div');
      document.body.append(root);
      const wrapper = root.appendChild(document.createElement('div'));
      Object.defineProperty(wrapper, 'clientWidth', { configurable: true, value: 800 });
      const table = wrapper.appendChild(document.createElement('table'));
      table.style.font = '14px Arial';
      const colgroup = table.appendChild(document.createElement('colgroup'));
      const controller = new TableLayoutController(root);
      const registration = controller.register({ table, wrapper, colgroup, columns: () => 2, cells: () => [] });
      return { root, wrapper, table, colgroup, controller, registration };
    };
    const first = create();
    await first.controller.flush();
    const saved = first.controller.snapshot()!;
    expect(saved).toHaveLength(1);
    first.controller.destroy(); first.root.remove();
    const next = create();
    try {
      expect(next.controller.restore(saved)).toBe(true);
      expect(Array.from(next.colgroup.children, col => (col as HTMLElement).style.width)).toEqual(saved[0].widths.map(width => `${width}px`));
      Object.defineProperty(next.wrapper, 'clientWidth', { configurable: true, value: 500 });
      expect(next.controller.snapshot()).toBeUndefined();
      await next.controller.flush();
      expect(next.controller.snapshot()?.[0].availableWidth).toBe(500);
      next.table.style.font = '20px Arial';
      expect(next.controller.snapshot()).toBeUndefined();
      await next.controller.flush();
      expect(next.controller.snapshot()?.[0].signature).toContain('20px');
      next.registration.update();
      expect(next.controller.snapshot()).toBeUndefined();
    } finally { next.controller.destroy(); next.root.remove(); }
  });
});
