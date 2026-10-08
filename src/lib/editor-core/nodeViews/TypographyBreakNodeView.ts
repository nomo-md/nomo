import type { Node as ProseMirrorNode } from 'prosemirror-model';
import type { EditorView, NodeView } from 'prosemirror-view';
import { getTypographyOptions, subscribeTypography } from '../../typography/options';
import { softBreakText } from '../../typography/rules';
const views = new WeakMap<EditorView, Set<TypographyBreakNodeView>>();
export function refreshTypographyBreaks(view: EditorView) {
  for (const item of views.get(view) ?? []) item.render();
}

/** Retains the source hard_break node while changing only its visual projection. */
export class TypographyBreakNodeView implements NodeView {
  dom: HTMLElement;
  private unsubscribe: () => void;
  constructor(
    private node: ProseMirrorNode,
    private view: EditorView,
    private getPos: () => number | undefined,
  ) {
    this.dom = document.createElement('span');
    this.dom.dataset.nomoBreak = node.attrs.soft ? 'soft' : 'hard';
    if (!views.has(view)) views.set(view, new Set());
    views.get(view)!.add(this);
    this.unsubscribe = subscribeTypography(() => this.render());
    this.render();
  }
  update(node: ProseMirrorNode) {
    if (node.type !== this.node.type) return false;
    this.node = node;
    this.render();
    return true;
  }
  render() {
    if (this.view.composing) return;
    this.dom.dataset.nomoBreak = this.node.attrs.soft ? 'soft' : 'hard';
    if (this.node.attrs.soft && getTypographyOptions().enabled) {
      const pos = this.getPos();
      if (pos === undefined) return;
      const before = this.view.state.doc.resolve(pos).nodeBefore?.textContent ?? '';
      const after =
        this.view.state.doc.resolve(pos + this.node.nodeSize).nodeAfter?.textContent ?? '';
      const text = softBreakText(before, after);
      if (this.dom.textContent !== text || this.dom.childElementCount) this.dom.textContent = text;
    } else {
      if (this.dom.childNodes.length !== 1 || this.dom.firstChild?.nodeName !== 'BR')
        this.dom.replaceChildren(document.createElement('br'));
    }
  }
  ignoreMutation() {
    return true;
  }
  destroy() {
    this.unsubscribe();
    views.get(this.view)?.delete(this);
  }
}
