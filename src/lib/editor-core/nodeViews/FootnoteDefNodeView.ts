import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import type { EditorView, ViewMutationRecord } from 'prosemirror-view';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import type { EditorCoreOptions } from '../types';

/**
 * 底部脚注定义 NodeView：展示定义标记、返回正文入口，并保留内容区原生编辑能力。
 */
export class FootnoteDefNodeView {
  dom: HTMLElement;
  contentDOM: HTMLElement;

  private node: ProseMirrorNode;
  private view: EditorView;
  private marker: HTMLButtonElement;
  private unsubscribeLocale: () => void = () => undefined;
  private navigation?: AbortController;

  constructor(node: ProseMirrorNode, view: EditorView,
    private footnotes?: EditorCoreOptions['footnotes'], private onFailure?: (error: unknown) => void) {
    this.node = node;
    this.view = view;

    this.dom = document.createElement('div');
    this.dom.className = 'footnote-def';

    this.marker = document.createElement('button');
    this.marker.type = 'button';
    this.marker.className = 'footnote-def-marker';
    this.marker.contentEditable = 'false';
    this.marker.addEventListener('mousedown', (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    this.marker.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      this.jumpToFirstReference();
    });

    this.contentDOM = document.createElement('span');
    this.contentDOM.className = 'footnote-def-content';

    this.dom.append(this.marker, this.contentDOM);
    this.unsubscribeLocale = onInterfaceLocaleChanged(() => this.render());
    this.render();
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) return false;
    this.node = node;
    this.render();
    return true;
  }

  stopEvent(event: Event): boolean {
    return this.marker.contains(event.target as Node);
  }

  ignoreMutation(mutation: ViewMutationRecord): boolean {
    return !this.contentDOM.contains(mutation.target);
  }

  destroy(): void {
    this.navigation?.abort();
    this.unsubscribeLocale();
  }

  private render(): void {
    const id = this.id;
    this.dom.setAttribute('data-footnote-id', id);
    this.contentDOM.dataset.placeholder = t.footnoteContentPlaceholder();
    this.marker.setAttribute('aria-label', t.returnToFootnoteReference({ id }));
    this.marker.title = t.returnToFootnoteReference({ id });
    this.marker.textContent = id;
  }

  private jumpToFirstReference(): void {
    if (this.view.composing) return;
    if (this.footnotes?.returnToReference) {
      this.navigation?.abort(); const request = this.navigation = new AbortController();
      void this.footnotes.returnToReference(this.id, request.signal).catch(error => {
        if (!request.signal.aborted) this.onFailure?.(error);
      });
      return;
    }
    let refEndPos: number | null = null;
    this.view.state.doc.descendants((node, pos) => {
      if (refEndPos !== null) return false;
      if (node.type.name === 'footnote_ref' && node.attrs.id === this.id) {
        refEndPos = pos + node.nodeSize;
        return false;
      }
      return true;
    });
    if (refEndPos === null) return;

    const tr = this.view.state.tr.setSelection(
      TextSelection.near(this.view.state.doc.resolve(refEndPos), 1),
    );
    this.view.dispatch(tr.scrollIntoView());
    this.view.focus();
  }

  private get id(): string {
    return String(this.node.attrs.id ?? '');
  }
}
