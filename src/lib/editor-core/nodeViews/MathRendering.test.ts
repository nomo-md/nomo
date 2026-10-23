// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { EditorView } from 'prosemirror-view';
import { schema } from '../schema';
import * as renderers from '../renderers';
import { MathBlockNodeView } from './MathBlockNodeView';
import { MathInlineNodeView } from './MathInlineNodeView';

afterEach(() => vi.restoreAllMocks());
describe('公式节点渲染更新', () => {
  it.each(['block', 'inline'] as const)('%s 内容未变保持 DOM，变化后只接受最新异步结果', async kind => {
    const completions: Array<(value: { html: string }) => void> = [];
    const render = vi.fn(() => new Promise<{ html: string }>(resolve => completions.push(resolve)));
    vi.spyOn(renderers, 'getMathRenderer').mockReturnValue({ render });
    const type = schema.nodes[kind === 'block' ? 'math_block' : 'math_inline'];
    const node = type.create({ tex: 'x' });
    const View = kind === 'block' ? MathBlockNodeView : MathInlineNodeView;
    const view = new View(node, { editable: false, isDestroyed: false } as EditorView, () => 0, true);
    try {
      view.update(type.create({ tex: 'x' }));
      expect(render).toHaveBeenCalledOnce();
      view.update(type.create({ tex: 'y' }));
      expect(render).toHaveBeenCalledTimes(2);
      completions[1]({ html: '<span>new</span>' }); await Promise.resolve();
      const rendered = view.dom.firstChild;
      view.update(type.create({ tex: 'y' }));
      expect(view.dom.firstChild).toBe(rendered);
      completions[0]({ html: '<span>old</span>' }); await Promise.resolve();
      expect(view.dom.textContent).toBe('new');
    } finally { view.destroy(); }
  });
});
