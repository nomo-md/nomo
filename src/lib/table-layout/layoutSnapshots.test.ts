// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { TableLayoutController } from './dom';

describe('表格纯数据布局恢复', () => {
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
