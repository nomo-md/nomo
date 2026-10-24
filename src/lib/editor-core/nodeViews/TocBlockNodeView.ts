import { TextSelection } from 'prosemirror-state';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import type { EditorView } from 'prosemirror-view';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import { createHeadingIdAllocator, extractDocumentTocItems, normalizeHeadingTitle } from '../../toc/tocService';

interface TocRow {
  title: string;
  id: string;
  level: number;
  number: number;
}

/** 正文目录块 NodeView：目录自身不可编辑，目录项只负责导航到正文标题。 */
export class TocBlockNodeView {
  dom: HTMLElement;
  private node: ProseMirrorNode;
  private unsubscribeLocale: () => void = () => undefined;

  constructor(
    node: ProseMirrorNode,
    private readonly view: EditorView,
    private readonly getPos: () => number,
    private readonly openHeading?: (href: string) => void,
  ) {
    this.node = node;
    this.dom = document.createElement('section');
    this.dom.className = 'toc-block';
    this.dom.setAttribute('contenteditable', 'false');
    this.unsubscribeLocale = onInterfaceLocaleChanged(() => this.render());
    this.render();
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) return false;
    this.node = node;
    this.render();
    return true;
  }

  selectNode(): void {
    // 目录块不可选中，不添加选中样式
  }

  deselectNode(): void {
    // 目录块不可选中，无需移除选中样式
  }

  stopEvent(event: Event): boolean {
    // 阻止所有点击事件，防止目录块被选中
    return event.type === 'click' || event.type === 'mousedown';
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.unsubscribeLocale();
  }

  private render(): void {
    this.dom.replaceChildren();

    const header = document.createElement('header');
    const title = document.createElement('strong');
    title.textContent = t.toc();
    header.appendChild(title);

    const deleteButton = document.createElement('button');
    deleteButton.type = 'button';
    deleteButton.className = 'toc-delete';
    deleteButton.title = t.deleteToc();
    deleteButton.setAttribute('aria-label', t.deleteToc());
    deleteButton.addEventListener('click', () => this.deleteBlock());
    header.appendChild(deleteButton);
    this.dom.appendChild(header);

    const rows = parseTocRows(String(this.node.attrs.content ?? ''));
    if (rows.length === 0) {
      const empty = document.createElement('p');
      empty.className = 'toc-empty';
      empty.textContent = t.documentHasNoHeadings();
      this.dom.appendChild(empty);
      return;
    }

    const list = document.createElement('div');
    list.className = 'toc-list';
    for (const row of rows) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'toc-link';
      button.dataset.level = String(row.level);
      button.style.setProperty('--toc-indent', `${(row.level - 1) * 28}px`);
      button.title = row.title;
      button.setAttribute('aria-label', t.jumpToHeading({ title: row.title }));

      const bullet = document.createElement('span');
      bullet.className = 'toc-bullet';
      bullet.setAttribute('aria-hidden', 'true');
      button.appendChild(bullet);

      const text = document.createElement('span');
      text.className = 'toc-text';
      text.textContent = row.title;
      button.appendChild(text);

      const leader = document.createElement('span');
      leader.className = 'toc-leader';
      leader.setAttribute('aria-hidden', 'true');
      button.appendChild(leader);

      const page = document.createElement('span');
      page.className = 'toc-page';
      page.setAttribute('aria-hidden', 'true');
      page.textContent = String(row.number);
      button.appendChild(page);

      button.addEventListener('click', () => this.scrollToHeading(row.id));
      list.appendChild(button);
    }
    this.dom.appendChild(list);
  }

  private deleteBlock(): void {
    if (!this.view.editable) return;
    const pos = this.getPos();
    const tr = this.view.state.tr.delete(pos, pos + this.node.nodeSize);
    this.view.dispatch(tr.scrollIntoView());
    this.view.focus();
  }

  private scrollToHeading(headingId: string): void {
    if (this.view.composing) return;
    if (this.openHeading) { this.openHeading(`#${encodeURIComponent(headingId)}`); return; }
    const headings = extractDocumentTocItems(this.view.state.doc);
    const heading = headings.find((item) => item.id === headingId);
    if (!heading) return;

    const targetNode = this.view.state.doc.nodeAt(heading.pos);
    if (!targetNode) return;
    const selectionPos = heading.pos + 1 + targetNode.content.size;
    const selection = TextSelection.near(this.view.state.doc.resolve(selectionPos));
    this.view.dispatch(this.view.state.tr.setSelection(selection));
    const targetDom = this.view.nodeDOM?.(heading.pos);
    const target = targetDom instanceof HTMLElement
      ? targetDom
      : findSemanticHeadingById(this.view.dom, headingId);
    smoothScrollToSemanticHeading(this.view.dom, target);
    this.view.focus();
  }
}

function parseTocRows(content: string): TocRow[] {
  let number = 0;
  return content
    .split(/\r?\n/)
    .map((line) => {
      const match = /^(\s*)-\s+\[(.*?)\]\(#(.+?)\)\s*$/.exec(line);
      if (!match) {
        return null;
      }

      number += 1;
      return {
        title: normalizeHeadingTitle(match[2]),
        id: safeDecodeURIComponent(match[3]),
        level: Math.floor(match[1].length / 2) + 1,
        number,
      };
    })
    .filter((row): row is TocRow => row !== null);
}

function safeDecodeURIComponent(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function smoothScrollToSemanticHeading(editorDom: HTMLElement, target: HTMLElement | null): void {
  const scrollContainer = editorDom.closest<HTMLElement>('.semantic-pane');
  if (!scrollContainer || !target) {
    return;
  }

  const targetTop =
    target.getBoundingClientRect().top -
    scrollContainer.getBoundingClientRect().top +
    scrollContainer.scrollTop;
  const top = Math.max(0, targetTop - 32);
  if (typeof scrollContainer.scrollTo === 'function') {
    scrollContainer.scrollTo({ top, behavior: 'smooth' });
  } else {
    scrollContainer.scrollTop = top;
  }
}

function findSemanticHeadingById(editorDom: HTMLElement, headingId: string): HTMLElement | null {
  const allocateId = createHeadingIdAllocator();
  const headings = Array.from(editorDom.querySelectorAll<HTMLElement>('h1, h2, h3, h4, h5, h6'))
    .filter((element) => !element.closest('.html-card'));
  for (let index = 0; index < headings.length; index++) {
    const heading = headings[index];
    const fallbackId = allocateId(normalizeHeadingTitle(heading.textContent ?? ''), index + 1);
    if ((heading.id || fallbackId) === headingId) return heading;
  }
  return null;
}
