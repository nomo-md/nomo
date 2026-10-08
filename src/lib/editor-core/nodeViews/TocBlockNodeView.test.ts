import { describe, expect, it, vi } from 'vitest';
import { EditorState, type Transaction } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { parseMarkdown } from '../markdown';
import { TocBlockNodeView } from './TocBlockNodeView';
import { createTocBlock } from '../../toc/tocService';

describe('TocBlockNodeView', () => {
  it('places selection at the end of the target heading after toc jump', () => {
    const doc = parseMarkdown('<!-- toc -->\n- [标题](#标题)\n<!-- /toc -->\n\n# 标题');
    const state = EditorState.create({ doc });
    let dispatched: Transaction | null = null;

    const semanticPane = document.createElement('section');
    semanticPane.className = 'semantic-pane';
    semanticPane.getBoundingClientRect = () => createRect(0);
    semanticPane.scrollTo = vi.fn();

    const editorDom = document.createElement('div');
    editorDom.className = 'ProseMirror';
    const heading = document.createElement('h1');
    heading.textContent = '标题';
    heading.getBoundingClientRect = () => createRect(120);
    editorDom.append(heading);
    semanticPane.append(editorDom);

    const view = {
      state,
      dom: editorDom,
      dispatch(transaction: Transaction) {
        dispatched = transaction;
      },
      focus: vi.fn(),
    } as unknown as EditorView;

    const nodeView = new TocBlockNodeView(doc.child(0), view, () => 0);
    nodeView.dom.querySelector<HTMLButtonElement>('.toc-link')?.click();

    const headingPos = doc.child(0).nodeSize;
    const expectedSelection = headingPos + 1 + '标题'.length;
    const selectionFrom = (dispatched as Transaction | null)?.selection.from;
    expect(selectionFrom).toBe(expectedSelection);
  });

  it('renders toc rows with leader and visual number segments', () => {
    const doc = parseMarkdown('<!-- toc -->\n- [标题](#标题)\n<!-- /toc -->\n\n# 标题');
    const state = EditorState.create({ doc });
    const view = {
      state,
      dom: document.createElement('div'),
      dispatch: vi.fn(),
      focus: vi.fn(),
    } as unknown as EditorView;

    const nodeView = new TocBlockNodeView(doc.child(0), view, () => 0);
    const link = nodeView.dom.querySelector('.toc-link');

    expect(link?.querySelector('.toc-text')?.textContent).toBe('标题');
    expect(link?.querySelector('.toc-leader')).not.toBeNull();
    expect(link?.querySelector('.toc-page')?.textContent).toBe('1');
    expect(nodeView.dom.querySelector('.toc-delete')?.getAttribute('aria-label')).toBe(
      'Delete table of contents',
    );
  });

  it('navigates to source-formatted headings and renders literal code titles without escape slashes', () => {
    const markdown = '# <u>标题</u>\n\n# `**字面**`';
    const doc = parseMarkdown(`${createTocBlock(markdown)}\n\n${markdown}`);
    const state = EditorState.create({ doc });
    const semanticPane = document.createElement('section');
    semanticPane.className = 'semantic-pane';
    semanticPane.getBoundingClientRect = () => createRect(0);
    semanticPane.scrollTo = vi.fn();
    const editorDom = document.createElement('div');
    editorDom.className = 'ProseMirror';
    const heading = document.createElement('h1');
    heading.textContent = '<u>标题</u>';
    heading.getBoundingClientRect = () => createRect(120);
    editorDom.append(heading);
    semanticPane.append(editorDom);
    let dispatched: Transaction | null = null;
    const view = {
      state, dom: editorDom,
      dispatch(transaction: Transaction) { dispatched = transaction; },
      focus: vi.fn(),
    } as unknown as EditorView;
    const nodeView = new TocBlockNodeView(doc.child(0), view, () => 0);
    const buttons = nodeView.dom.querySelectorAll<HTMLButtonElement>('.toc-link');
    expect(buttons[1].querySelector('.toc-text')?.textContent).toBe('**字面**');
    buttons[0].click();
    const headingPos = doc.child(0).nodeSize;
    expect((dispatched as Transaction | null)?.selection.from).toBe(headingPos + 1 + doc.child(1).content.size);
    expect(semanticPane.scrollTo).toHaveBeenCalledWith({ top: 88, behavior: 'smooth' });
    nodeView.destroy();
  });
});

function createRect(top: number): DOMRect {
  return {
    x: 0,
    y: top,
    top,
    left: 0,
    bottom: top + 20,
    right: 100,
    width: 100,
    height: 20,
    toJSON: () => ({}),
  };
}
