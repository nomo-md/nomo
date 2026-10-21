import type { Node } from 'prosemirror-model';
import { Plugin, PluginKey } from 'prosemirror-state';
import { Decoration, DecorationSet } from 'prosemirror-view';

interface HeadingAnchors {
  ids: readonly string[] | null;
  decorations: DecorationSet;
}

export const headingAnchorsKey = new PluginKey<HeadingAnchors>('headingAnchors');

function decorate(doc: Node, ids: readonly string[]): DecorationSet {
  const decorations: Decoration[] = [];
  let index = 0;
  doc.descendants((node, pos) => {
    if (node.type.name !== 'heading') return;
    const id = ids[index++];
    if (id) decorations.push(Decoration.node(pos, pos + node.nodeSize, { id }, { headingId: id }));
    return false;
  });
  return DecorationSet.create(doc, decorations);
}

/** 全局索引提供窗口内标题 ID；通过 decoration 写 DOM，避免触发外部修改修复循环。 */
export function headingAnchorsPlugin(): Plugin<HeadingAnchors> {
  return new Plugin<HeadingAnchors>({
    key: headingAnchorsKey,
    state: {
      init: () => ({ ids: [], decorations: DecorationSet.empty }),
      apply(tr, value) {
        const ids: readonly string[] | undefined = tr.getMeta(headingAnchorsKey);
        if (ids !== undefined) return { ids, decorations: decorate(tr.doc, ids) };
        if (!tr.docChanged) return value;
        // 编辑期间映射既有锚点；下一次索引刷新重新对齐标题（含新增/删除）。
        return { ids: null, decorations: value.decorations.map(tr.mapping, tr.doc) };
      },
    },
    props: { decorations: state => headingAnchorsKey.getState(state)?.decorations },
  });
}
