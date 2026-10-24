import type { Node } from 'prosemirror-model';
import { Plugin, PluginKey } from 'prosemirror-state';
import { Decoration, DecorationSet } from 'prosemirror-view';

interface HeadingAnchors {
  ids: readonly string[] | null;
  ownedIds: readonly string[];
  decorations: DecorationSet;
}
let nextLocalHeading = 0;

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
      init: () => ({ ids: [], ownedIds: [], decorations: DecorationSet.empty }),
      apply(tr, value) {
        const ids: readonly string[] | undefined = tr.getMeta(headingAnchorsKey);
        if (ids !== undefined) return { ids, ownedIds: ids, decorations: decorate(tr.doc, ids) };
        if (!tr.docChanged) return value;
        // 编辑期间映射既有锚点；下一次索引刷新重新对齐标题（含新增/删除）。
        let decorations = value.decorations.map(tr.mapping, tr.doc);
        const added: Decoration[] = [];
        tr.doc.descendants((node, pos) => {
          if (node.type.name !== 'heading') return;
          if (!decorations.find(pos, pos + 1).some(item => item.from === pos))
            added.push(Decoration.node(pos, pos + node.nodeSize, {}, { headingId: `local-heading:${++nextLocalHeading}`, local: true }));
          return false;
        });
        decorations = decorations.add(tr.doc, added);
        return { ids: null, ownedIds: value.ownedIds, decorations };
      },
    },
    props: { decorations: state => headingAnchorsKey.getState(state)?.decorations },
  });
}
