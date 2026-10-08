import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setTypographyOptions } from '../typography/options';
import { DEFAULT_TYPOGRAPHY } from '../typography/types';
import { Slice, type Node as ProseMirrorNode } from 'prosemirror-model';
import { AllSelection, TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { reorderOutlineSection } from '../outline/outlineReorder';
import { extractOutline } from '../outline/outlineService';
import { createEditorCore } from './createEditorCore';
import { parseMarkdown } from './markdown';

beforeEach(() => { setTypographyOptions({ ...DEFAULT_TYPOGRAPHY, enabled: false }); });
afterEach(() => {
  setTypographyOptions(DEFAULT_TYPOGRAPHY);
  vi.useRealTimers();
});

function findFirstNode(
  doc: ProseMirrorNode,
  typeName: string,
): { node: ProseMirrorNode; pos: number } {
  let found: { node: ProseMirrorNode; pos: number } | null = null;
  doc.descendants((node, pos) => {
    if (node.type.name === typeName) {
      found = { node, pos };
      return false;
    }
    return true;
  });
  if (!found) {
    throw new Error(`Node ${typeName} not found`);
  }
  return found;
}

function findNodeByText(
  doc: ProseMirrorNode,
  typeName: string,
  text: string,
): { node: ProseMirrorNode; pos: number } {
  let found: { node: ProseMirrorNode; pos: number } | null = null;
  doc.descendants((node, pos) => {
    if (node.type.name === typeName && node.textContent === text) {
      found = { node, pos };
      return false;
    }
    return true;
  });
  if (!found) {
    throw new Error(`Node ${typeName} with text ${text} not found`);
  }
  return found;
}

function pressEditorKey(view: EditorView, key: string, init?: KeyboardEventInit): boolean {
  const event = new KeyboardEvent('keydown', {
    key,
    ...init,
    bubbles: true,
    cancelable: true,
  });
  let handled = false;
  view.someProp('handleKeyDown', (handler) => {
    handled = handler(view, event) || handled;
    return handled;
  });
  return handled;
}

function getTopLevelNodeNames(doc: ProseMirrorNode): string[] {
  return Array.from({ length: doc.childCount }, (_, index) => doc.child(index).type.name);
}

describe('createEditorCore', () => {
  it('keeps Markdown as the observable editor state', () => {
    const editor = createEditorCore({ markdown: '# Nomo' });

    editor.setMarkdown('# Nomo\n\n阶段0');

    expect(editor.getMarkdown()).toBe('# Nomo\n\n阶段0');
    expect(editor.getSnapshot()).toMatchObject({
      markdown: '# Nomo\n\n阶段0',
      version: 1,
    });
  });

  it('renders source soft line breaks as visible semantic line breaks', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '1\n2\n3', target });

    expect(target.querySelectorAll('.ProseMirror p br')).toHaveLength(2);
    expect(editor.getMarkdown()).toBe('1\n2\n3');

    editor.destroy();
  });

  it('copies Markdown by default while retaining rich HTML', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 标题\n\n这是 **重点**', target });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dispatch(view.state.tr.setSelection(new AllSelection(view.state.doc)));

    const payload = editor.getClipboardPayload();

    expect(payload?.text).toBe('# 标题\n\n这是 **重点**');
    expect(payload?.html).toContain('<h1');
    expect(payload?.html).toContain('<strong>重点</strong>');
    editor.destroy();
  });

  it('restores plain-text copying when Markdown clipboard syntax is disabled', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '# 标题\n\n这是 **重点**',
      target,
      copyMarkdownSyntaxEnabled: false,
    });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dispatch(view.state.tr.setSelection(new AllSelection(view.state.doc)));

    expect(editor.getClipboardPayload()?.text).toBe('标题\n\n这是 重点');
    editor.destroy();
  });

  it('applies Markdown clipboard option updates immediately', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '**重点**', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findNodeByText(view.state.doc, 'paragraph', '重点');
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          paragraph.pos + 1,
          paragraph.pos + 1 + paragraph.node.content.size,
        ),
      ),
    );

    expect(editor.getClipboardPayload()?.text).toBe('**重点**');
    editor.updateOptions({ copyMarkdownSyntaxEnabled: false });
    expect(editor.getClipboardPayload()?.text).toBe('重点');
    editor.updateOptions({ copyMarkdownSyntaxEnabled: true });
    expect(editor.getClipboardPayload()?.text).toBe('**重点**');
    editor.destroy();
  });

  it('keeps block syntax only when the full visible block content is selected', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# **标题文字**', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const heading = findNodeByText(view.state.doc, 'heading', '标题文字');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          heading.pos + 1,
          heading.pos + 1 + heading.node.content.size,
        ),
      ),
    );
    expect(editor.getClipboardPayload()?.text).toBe('# **标题文字**');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, heading.pos + 2, heading.pos + 4),
      ),
    );
    expect(editor.getClipboardPayload()?.text).toBe('**题文**');
    editor.destroy();
  });

  it('copies a single list item textblock without its outer list marker', () => {
    const listTarget = document.createElement('div');
    const listEditor = createEditorCore({ markdown: '- 第一项\n- 第二项', target: listTarget });
    const listView = (listEditor as unknown as { view: EditorView }).view;
    const firstItemParagraph = findNodeByText(listView.state.doc, 'paragraph', '第一项');
    listView.dispatch(
      listView.state.tr.setSelection(
        TextSelection.create(
          listView.state.doc,
          firstItemParagraph.pos + 1,
          firstItemParagraph.pos + 1 + firstItemParagraph.node.content.size,
        ),
      ),
    );
    expect(listEditor.getClipboardPayload()?.text).toBe('第一项');
    listEditor.destroy();

    const orderedTarget = document.createElement('div');
    const orderedEditor = createEditorCore({
      markdown: '3. **标题**和`代码`',
      target: orderedTarget,
    });
    const orderedView = (orderedEditor as unknown as { view: EditorView }).view;
    const orderedParagraph = findNodeByText(orderedView.state.doc, 'paragraph', '标题和代码');
    orderedView.dispatch(
      orderedView.state.tr.setSelection(
        TextSelection.create(
          orderedView.state.doc,
          orderedParagraph.pos + 1,
          orderedParagraph.pos + 1 + orderedParagraph.node.content.size,
        ),
      ),
    );
    expect(orderedEditor.getClipboardPayload()?.text).toBe('**标题**和`代码`');

    orderedView.dispatch(
      orderedView.state.tr.setSelection(
        TextSelection.create(
          orderedView.state.doc,
          orderedParagraph.pos + 2,
          orderedParagraph.pos + 1 + orderedParagraph.node.content.size,
        ),
      ),
    );
    expect(orderedEditor.getClipboardPayload()?.text).toBe('**题**和`代码`');
    orderedEditor.destroy();
  });

  it('keeps selected task markers but removes the outer list marker', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '- [x] 已完成', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findNodeByText(view.state.doc, 'paragraph', '[x] 已完成');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          paragraph.pos + 1,
          paragraph.pos + 1 + paragraph.node.content.size,
        ),
      ),
    );

    expect(editor.getClipboardPayload()?.text).toBe('[x] 已完成');
    editor.destroy();
  });

  it('keeps list structure when copying across items, nested content, or the full document', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '3. 第一项\n4. 第二项\n5. 第三项',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;
    const firstParagraph = findNodeByText(view.state.doc, 'paragraph', '第一项');
    const secondParagraph = findNodeByText(view.state.doc, 'paragraph', '第二项');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          firstParagraph.pos + 1,
          secondParagraph.pos + 1 + secondParagraph.node.content.size,
        ),
      ),
    );
    expect(editor.getClipboardPayload()?.text).toBe('3. 第一项\n4. 第二项');

    view.dispatch(view.state.tr.setSelection(new AllSelection(view.state.doc)));
    expect(editor.getClipboardPayload()?.text).toBe('3. 第一项\n4. 第二项\n5. 第三项');
    editor.destroy();

    const nestedTarget = document.createElement('div');
    const nestedEditor = createEditorCore({ markdown: '1. 父项\n   - 子项', target: nestedTarget });
    const nestedView = (nestedEditor as unknown as { view: EditorView }).view;
    const parentParagraph = findNodeByText(nestedView.state.doc, 'paragraph', '父项');
    const childParagraph = findNodeByText(nestedView.state.doc, 'paragraph', '子项');
    nestedView.dispatch(
      nestedView.state.tr.setSelection(
        TextSelection.create(
          nestedView.state.doc,
          parentParagraph.pos + 1,
          childParagraph.pos + 1 + childParagraph.node.content.size,
        ),
      ),
    );
    expect(nestedEditor.getClipboardPayload()?.text).toContain('1. 父项');
    expect(nestedEditor.getClipboardPayload()?.text).toContain('- 子项');
    nestedEditor.destroy();
  });

  it('keeps complete code blocks as Markdown structures', () => {
    const codeTarget = document.createElement('div');
    const codeEditor = createEditorCore({
      markdown: '```ts\nconst value = 1;\n```',
      target: codeTarget,
    });
    const codeView = (codeEditor as unknown as { view: EditorView }).view;
    const codeBlock = findFirstNode(codeView.state.doc, 'code_block');
    codeView.dispatch(
      codeView.state.tr.setSelection(
        TextSelection.create(
          codeView.state.doc,
          codeBlock.pos + 1,
          codeBlock.pos + 1 + codeBlock.node.content.size,
        ),
      ),
    );
    expect(codeEditor.getClipboardPayload()?.text).toBe('```ts\nconst value = 1;\n```');

    codeView.dispatch(
      codeView.state.tr.setSelection(
        TextSelection.create(codeView.state.doc, codeBlock.pos + 1, codeBlock.pos + 6),
      ),
    );
    expect(codeEditor.getClipboardPayload()?.text).toBe('const');
    codeEditor.destroy();
  });

  it('does not copy the synthetic empty paragraph after a terminal special block', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '```ts\nconst value = 1;\n```', target });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dispatch(view.state.tr.setSelection(new AllSelection(view.state.doc)));

    expect(editor.getClipboardPayload()?.text).toBe('```ts\nconst value = 1;\n```');
    editor.destroy();
  });

  it('serializes complete tables and falls back to plain text for partial tables', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '| A | B |\n| :--- | :--- |\n| \\[raw\\] | value |',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;
    const firstCell = findNodeByText(view.state.doc, 'paragraph', 'A');
    const lastCell = findNodeByText(view.state.doc, 'paragraph', 'value');
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          firstCell.pos + 1,
          lastCell.pos + 1 + lastCell.node.content.size,
        ),
      ),
    );
    expect(editor.getClipboardPayload()?.text).toContain('| A | B |');
    expect(editor.getClipboardPayload()?.text).toContain('| [raw] | value |');

    const rawCell = findNodeByText(view.state.doc, 'paragraph', '[raw]');
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(
          view.state.doc,
          rawCell.pos + 1,
          rawCell.pos + 1 + rawCell.node.content.size,
        ),
      ),
    );
    expect(editor.getClipboardPayload()?.text).toBe('[raw]');
    editor.destroy();
  });

  it('opens a code-only document with a safe trailing paragraph selection', () => {
    const markdown = '```java\n1`111\n```';
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown, target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['code_block', 'paragraph']);
    expect(view.state.selection).toBeInstanceOf(TextSelection);
    expect(view.state.selection.$from.parent.type.name).toBe('paragraph');
    expect(editor.getMarkdown()).toBe(markdown);
    expect(dirtyEvents.at(-1)).toBe(false);

    view.dispatch(view.state.tr.insertText('下方正文'));

    expect(view.state.doc.child(0).textContent).toBe('1`111');
    expect(view.state.doc.child(1).textContent).toBe('下方正文');
    expect(dirtyEvents.at(-1)).toBe(true);

    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.getMarkdown()).toBe(markdown);
    expect(dirtyEvents.at(-1)).toBe(false);

    view.dispatch(view.state.tr.insertText('X', 1));
    expect(editor.getMarkdown()).toBe('```java\nX1`111\n```');

    editor.destroy();
  });

  it('keeps a Mermaid-only document when typing from its initial selection', () => {
    const markdown = '```mermaid\nflowchart TD\n  A --> B\n```';
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown, target });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['mermaid_block', 'paragraph']);
    expect(view.state.selection).not.toBeInstanceOf(AllSelection);
    expect(view.state.selection.$from.parent.type.name).toBe('paragraph');

    view.dispatch(view.state.tr.insertText('图表说明'));

    expect(view.state.doc.child(0).type.name).toBe('mermaid_block');
    expect(view.state.doc.child(0).attrs.code).toBe('flowchart TD\n  A --> B');
    expect(view.state.doc.child(1).textContent).toBe('图表说明');

    editor.destroy();
  });

  it('opens a math-only document with a safe trailing paragraph selection', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '$$\nE = mc^2\n$$', target });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['math_block', 'paragraph']);
    expect(view.state.selection.$from.parent.type.name).toBe('paragraph');

    editor.destroy();
  });

  it('normalizes a trailing special block when source mode returns to semantic mode', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# Old', mode: 'source', target });
    const view = (editor as unknown as { view: EditorView }).view;

    editor.setMarkdown('```mermaid\nflowchart LR\n  A --> B\n```', { sourceInput: true });
    expect(view.state.doc.textContent).toBe('Old');

    editor.updateOptions({ mode: 'semantic' });

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['mermaid_block', 'paragraph']);
    expect(view.state.selection.$from.parent.type.name).toBe('paragraph');

    editor.destroy();
  });

  it('does not add paragraphs when a special block is not the final document node', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '```ts\nconst value = 1;\n```\n\n# 后续标题',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['code_block', 'heading']);

    editor.destroy();
  });

  it('does not duplicate an existing paragraph after a special block', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '```ts\nconst value = 1;\n```\n\n后续正文',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['code_block', 'paragraph']);
    expect(view.state.doc.child(1).textContent).toBe('后续正文');

    editor.destroy();
  });

  it('inserts a source soft line break with Shift+Enter in semantic paragraphs', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '1', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findFirstNode(view.state.doc, 'paragraph');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, paragraph.pos + paragraph.node.nodeSize - 1),
      ),
    );

    expect(pressEditorKey(view, 'Enter', { shiftKey: true })).toBe(true);
    view.dispatch(view.state.tr.insertText('2'));

    expect(target.querySelectorAll('.ProseMirror p br')).toHaveLength(1);
    expect(editor.getMarkdown()).toBe('1\n2');

    editor.destroy();
  });

  it('replaces a paragraph selection with a soft line break on Shift+Enter', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: 'abcd', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findFirstNode(view.state.doc, 'paragraph');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, paragraph.pos + 2, paragraph.pos + 4),
      ),
    );

    expect(pressEditorKey(view, 'Enter', { shiftKey: true })).toBe(true);

    expect(target.querySelectorAll('.ProseMirror p br')).toHaveLength(1);
    expect(editor.getMarkdown()).toBe('a\nd');

    editor.destroy();
  });

  it('falls back to heading split behavior on Shift+Enter in headings', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 一二三', target });
    const view = (editor as unknown as { view: EditorView }).view;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));

    expect(pressEditorKey(view, 'Enter', { shiftKey: true })).toBe(true);

    expect(view.state.doc.childCount).toBe(2);
    expect(view.state.doc.child(0).type.name).toBe('heading');
    expect(view.state.doc.child(0).textContent).toBe('一');
    expect(view.state.doc.child(1).type.name).toBe('paragraph');
    expect(view.state.doc.child(1).textContent).toBe('二三');
    expect(editor.getMarkdown()).toBe('# 一\n\n二三');

    editor.destroy();
  });

  it('serializes Shift+Enter inside table cells as br to keep pipe tables valid', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '| A |\n| :--- |', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const cellParagraph = findNodeByText(view.state.doc, 'paragraph', 'A');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, cellParagraph.pos + cellParagraph.node.nodeSize - 1),
      ),
    );

    expect(pressEditorKey(view, 'Enter', { shiftKey: true })).toBe(true);
    view.dispatch(view.state.tr.insertText('B'));

    expect(editor.getMarkdown()).toBe('| A<br>B |\n| :--- |');

    editor.destroy();
  });

  it('emits immutable change events through subscribe', () => {
    const editor = createEditorCore({ markdown: '' });
    const events: string[] = [];

    editor.subscribe((event) => {
      events.push(`${event.version}:${event.reason}:${event.mode}`);
    });

    editor.updateOptions({ mode: 'source' });
    editor.setMarkdown('正文');

    expect(events).toEqual([
      '0:subscribe:semantic',
      '0:runtime-options:source',
      '1:programmatic-update:source',
    ]);
  });

  it('defers ProseMirror state rebuild for source input until semantic mode resumes', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# Old', mode: 'source', target });
    const view = (editor as unknown as { view: EditorView }).view;

    editor.setMarkdown('# New', { sourceInput: true });

    expect(editor.getMarkdown()).toBe('# New');
    expect(view.state.doc.textContent).toBe('Old');

    editor.updateOptions({ mode: 'semantic' });

    expect(view.state.doc.textContent).toBe('New');
  });

  it('emits pending inline mark snapshots for toolbar state', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const events: boolean[] = [];

    editor.subscribe((event) => {
      events.push(event.pendingInlineMarks.strong);
    });

    editor.execute({ type: 'toggleBold' });

    expect(events).toEqual([false, true]);
  });

  it('emits pending highlight snapshots for toolbar state', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const events: boolean[] = [];

    editor.subscribe((event) => {
      events.push(event.pendingInlineMarks.highlight);
    });

    editor.execute({ type: 'toggleHighlight' });

    expect(events).toEqual([false, true]);
  });

  it('does not enter pending inline marks in source mode', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });

    editor.updateOptions({ mode: 'source' });

    expect(editor.execute({ type: 'toggleBold' })).toBe(false);
    expect(editor.isPendingMarkActive?.('strong')).toBe(false);
  });

  it('clears pending inline marks before text is typed', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });

    expect(editor.execute({ type: 'toggleBold' })).toBe(true);
    expect(editor.isPendingMarkActive?.('strong')).toBe(true);

    expect(editor.execute({ type: 'clearInlineStyles' })).toBe(true);
    expect(editor.isPendingMarkActive?.('strong')).toBe(false);
    expect(editor.getMarkdown()).toBe('');
  });

  it('serializes ProseMirror edits back to Markdown through EditorCore', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 标题\n\n正文', target });

    editor.execute({ type: 'setHeading', level: 2 });

    expect(editor.getMarkdown()).toContain('##');
  });

  it('pressing Backspace below a code block deletes the block instead of entering edit mode', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '```ts\nconst a = 1;\n```\n\n后续正文',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findFirstNode(view.state.doc, 'paragraph');

    view.dispatch(
      view.state.tr.setSelection(TextSelection.create(view.state.doc, paragraph.pos + 1)),
    );

    const event = new KeyboardEvent('keydown', {
      key: 'Backspace',
      bubbles: true,
      cancelable: true,
    });
    let handled = false;
    view.someProp('handleKeyDown', (handler) => {
      handled = handler(view, event) || handled;
      return handled;
    });

    expect(handled).toBe(true);
    expect(editor.getMarkdown()).toBe('后续正文');
    expect(target.querySelector('.code-card')).toBeNull();
  });

  it('pressing Tab in body text inserts a tab character instead of leaving the editor', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));

    const event = new KeyboardEvent('keydown', {
      key: 'Tab',
      bubbles: true,
      cancelable: true,
    });
    let handled = false;
    view.someProp('handleKeyDown', (handler) => {
      handled = handler(view, event) || handled;
      return handled;
    });

    expect(handled).toBe(true);
    expect(editor.getMarkdown()).toBe('正\t文');
  });

  it('exposes the active link snapshot through EditorCore', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '[链接](https://example.com "说明")',
      target,
    });

    expect(editor.getActiveLink()).toMatchObject({
      href: 'https://example.com',
      title: '说明',
      text: '链接',
      active: true,
    });
  });

  it('keeps front matter when semantic edits serialize Markdown', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '---\ntitle: Demo\n---\n# 标题', target });

    editor.execute({ type: 'setHeading', level: 2 });

    expect(editor.getMarkdown().startsWith('---\ntitle: Demo\n---\n')).toBe(true);
  });

  it('inserts technical document snippets as Markdown', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 技术文档', target });

    editor.execute({ type: 'toggleTaskList' });
    editor.execute({ type: 'insertTable', rows: 1, columns: 2 });
    editor.execute({ type: 'insertMathBlock', tex: 'E = mc^2' });
    editor.execute({ type: 'insertMermaidBlock', code: 'flowchart TD\n  A --> B' });

    expect(editor.getMarkdown()).toContain('- [ ]');
    expect(editor.getMarkdown()).toContain('技术文档');
    expect(editor.getMarkdown()).toContain('| :--- | :--- |');
    expect(editor.getMarkdown()).toContain('$$\nE = mc^2\n$$');
    expect(editor.getMarkdown()).toContain('```mermaid');
  });

  it('inserts a toc block as Markdown at the semantic selection', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 标题1\n\n## 标题2', target });

    expect(editor.execute({ type: 'insertToc' })).toBe(true);
    expect(editor.getMarkdown()).toContain('<!-- toc -->');
    expect(editor.getMarkdown()).toContain('- [标题1](#标题1)');
    expect(editor.getMarkdown()).toContain('  - [标题2](#标题2)');
    expect(target.querySelector('.toc-block')).not.toBeNull();
  });

  it('updates toc block content when headings change', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '<!-- toc -->\n- [旧标题](#旧标题)\n<!-- /toc -->\n\n# 新标题',
      target,
    });

    expect(editor.getMarkdown()).toContain('- [新标题](#新标题)');
    expect(editor.getMarkdown()).not.toContain('旧标题');
  });

  it('preserves undo and redo history while syncing toc after heading changes', () => {
    const titleMarkdown = '<!-- toc -->\n- [原标题](#原标题)\n<!-- /toc -->\n\n# 原标题';
    const titleTarget = document.createElement('div');
    const titleEditor = createEditorCore({ markdown: titleMarkdown, target: titleTarget });
    const titleView = (titleEditor as unknown as { view: EditorView }).view;
    const titleDirtyEvents: boolean[] = [];
    titleEditor.subscribe((event) => titleDirtyEvents.push(event.dirty));

    const originalHeading = findNodeByText(titleView.state.doc, 'heading', '原标题');
    titleView.dispatch(
      titleView.state.tr.insertText(
        '新标题',
        originalHeading.pos + 1,
        originalHeading.pos + 1 + originalHeading.node.content.size,
      ),
    );

    expect(titleTarget.querySelector('.toc-text')?.textContent).toBe('新标题');
    expect(titleEditor.execute({ type: 'undo' })).toBe(true);
    expect(titleTarget.querySelector('.toc-text')?.textContent).toBe('原标题');
    expect(titleEditor.getMarkdown()).toBe(titleMarkdown);
    expect(titleDirtyEvents.at(-1)).toBe(false);
    expect(titleEditor.execute({ type: 'redo' })).toBe(true);
    expect(titleTarget.querySelector('.toc-text')?.textContent).toBe('新标题');

    const levelMarkdown = '<!-- toc -->\n  - [标题](#标题)\n<!-- /toc -->\n\n## 标题';
    const levelTarget = document.createElement('div');
    const levelEditor = createEditorCore({ markdown: levelMarkdown, target: levelTarget });
    const levelView = (levelEditor as unknown as { view: EditorView }).view;
    const levelDirtyEvents: boolean[] = [];
    levelEditor.subscribe((event) => levelDirtyEvents.push(event.dirty));

    const levelHeading = findNodeByText(levelView.state.doc, 'heading', '标题');
    levelView.dispatch(
      levelView.state.tr.setSelection(TextSelection.create(levelView.state.doc, levelHeading.pos + 1)),
    );
    expect(levelEditor.execute({ type: 'setHeading', level: 3 })).toBe(true);
    expect(levelTarget.querySelector<HTMLElement>('.toc-link')?.dataset.level).toBe('3');
    expect(levelEditor.execute({ type: 'undo' })).toBe(true);
    expect(levelTarget.querySelector<HTMLElement>('.toc-link')?.dataset.level).toBe('2');
    expect(levelEditor.getMarkdown()).toBe(levelMarkdown);
    expect(levelDirtyEvents.at(-1)).toBe(false);
    expect(levelEditor.execute({ type: 'redo' })).toBe(true);
    expect(levelTarget.querySelector<HTMLElement>('.toc-link')?.dataset.level).toBe('3');

    const bodyMarkdown = '<!-- toc -->\n- [标题](#标题)\n<!-- /toc -->\n\n# 标题\n\n正文';
    const bodyTarget = document.createElement('div');
    const bodyEditor = createEditorCore({ markdown: bodyMarkdown, target: bodyTarget });
    const bodyView = (bodyEditor as unknown as { view: EditorView }).view;
    const bodyParagraph = findNodeByText(bodyView.state.doc, 'paragraph', '正文');
    bodyView.dispatch(bodyView.state.tr.insertText('补充', bodyParagraph.pos + 1));
    expect(bodyTarget.querySelector('.toc-text')?.textContent).toBe('标题');
    expect(bodyEditor.execute({ type: 'undo' })).toBe(true);
    expect(bodyEditor.getMarkdown()).toBe(bodyMarkdown);

    titleEditor.destroy();
    levelEditor.destroy();
    bodyEditor.destroy();
  });

  it('renders an empty toc placeholder when no headings exist', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });

    editor.execute({ type: 'insertToc' });

    expect(editor.getMarkdown()).toContain('<!-- toc -->\n<!-- /toc -->');
    expect(target.querySelector('.toc-empty')?.textContent).toContain(
      'This document has no headings yet',
    );
  });

  it('keeps toc marker examples in inline code as ordinary Markdown text', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const markdown = '正文 `<!-- toc --><!-- /toc -->` 后续';

    editor.setMarkdown(markdown);

    expect(editor.getMarkdown()).toBe(markdown);
    expect(target.querySelector('.toc-block')).toBeNull();
  });

  it('does not rewrite adjacent toc marker text during semantic transactions', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;

    view.dispatch(view.state.tr.insertText('正文 '));
    view.dispatch(view.state.tr.insertText('<!-- toc --><!-- /toc -->'));
    view.dispatch(view.state.tr.insertText(' 后续'));

    expect(editor.getMarkdown()).toBe('正文 <!-- toc --><!-- /toc --> 后续');
    expect(editor.getMarkdown()).not.toContain('-- >');
    expect(target.querySelector('.toc-block')).toBeNull();
  });

  it('keeps an existing toc block stable when typing above it in semantic mode', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '<!-- toc -->\n<!-- /toc -->', target });
    const view = (editor as unknown as { view: EditorView }).view;

    view.dispatch(view.state.tr.insertText('上方文字', 0));

    expect(editor.getMarkdown()).toBe('上方文字\n\n<!-- toc -->\n<!-- /toc -->\n');
    expect(target.querySelector('.toc-block')).not.toBeNull();
  });

  it('inserts default YAML front matter without duplicating existing metadata', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '# 正文', target });

    expect(editor.execute({ type: 'insertFrontMatter' })).toBe(true);
    expect(editor.getMarkdown()).toContain('title: 文档标题');
    expect(editor.getMarkdown()).toContain('tags:\n  - 笔记\n  - Markdown');
    expect(editor.getMarkdown()).toContain('# 正文');

    const withFrontMatter = editor.getMarkdown();
    expect(editor.execute({ type: 'insertFrontMatter' })).toBe(true);
    expect(editor.getMarkdown()).toBe(withFrontMatter);
  });

  it('jumps to the heading matched by toc link id instead of row position only', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '<!-- toc -->\n<!-- /toc -->\n\n# Same\n\n## Same',
      target,
    });

    const secondTocLink = target.querySelectorAll<HTMLButtonElement>('.toc-link')[1];
    secondTocLink.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    editor.execute({ type: 'setHeading', level: 3 });

    expect(editor.getMarkdown()).toContain('# Same\n\n### Same');
    expect(editor.getMarkdown()).not.toContain('### Same\n\n## Same');
  });

  it('toggles bullet and ordered lists back to plain paragraphs', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '列表项', target });

    editor.execute({ type: 'toggleBulletList' });
    expect(editor.getMarkdown()).toBe('- 列表项');

    editor.execute({ type: 'toggleBulletList' });
    expect(editor.getMarkdown()).toBe('列表项');

    editor.execute({ type: 'toggleOrderedList' });
    expect(editor.getMarkdown()).toBe('1. 列表项');

    editor.execute({ type: 'toggleOrderedList' });
    expect(editor.getMarkdown()).toBe('列表项');
  });

  it('converts between ordered and bullet lists while preserving task markers', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '1. [ ] 待办事项', target });

    editor.execute({ type: 'toggleBulletList' });
    expect(editor.getMarkdown()).toBe('- [ ] 待办事项');

    editor.execute({ type: 'toggleOrderedList' });
    expect(editor.getMarkdown()).toBe('1. [ ] 待办事项');
  });

  it('converts typed Markdown list shortcuts with Space or Tab', () => {
    const orderedTarget = document.createElement('div');
    const orderedEditor = createEditorCore({ markdown: '', target: orderedTarget });
    const orderedView = (orderedEditor as unknown as { view: EditorView }).view;

    orderedView.dispatch(orderedView.state.tr.insertText('1.'));
    expect(pressEditorKey(orderedView, 'Tab')).toBe(true);
    orderedView.dispatch(orderedView.state.tr.insertText('第一项'));

    expect(orderedEditor.getMarkdown()).toBe('1. 第一项');

    const bulletTarget = document.createElement('div');
    const bulletEditor = createEditorCore({ markdown: '', target: bulletTarget });
    const bulletView = (bulletEditor as unknown as { view: EditorView }).view;

    bulletView.dispatch(bulletView.state.tr.insertText('*'));
    expect(pressEditorKey(bulletView, ' ')).toBe(true);
    bulletView.dispatch(bulletView.state.tr.insertText('无序项'));

    expect(bulletEditor.getMarkdown()).toBe('- 无序项');
  });

  it('continues task list items with an unchecked task marker on Enter', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '- [ ] 待办事项', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findFirstNode(view.state.doc, 'paragraph');

    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, paragraph.pos + paragraph.node.nodeSize - 1),
      ),
    );

    expect(pressEditorKey(view, 'Enter')).toBe(true);
    view.dispatch(view.state.tr.insertText('下一项'));

    expect(editor.getMarkdown()).toBe('- [ ] 待办事项\n- [ ] 下一项');
  });

  it('deletes the task item with Backspace at the start of a task item', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '- [ ] 待办事项', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findFirstNode(view.state.doc, 'paragraph');

    view.dispatch(
      view.state.tr.setSelection(TextSelection.create(view.state.doc, paragraph.pos + 4)),
    );

    expect(pressEditorKey(view, 'Backspace')).toBe(true);
    expect(editor.getMarkdown()).toBe('');
  });

  it('deletes only the current task item when Backspace is pressed in a multi-item task list', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '- [ ] 第一项\n- [ ] 第二项',
      target,
    });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraphs: Array<{ node: ProseMirrorNode; pos: number }> = [];
    view.state.doc.descendants((node, pos) => {
      if (node.type.name === 'paragraph') {
        paragraphs.push({ node, pos });
      }
      return true;
    });

    view.dispatch(
      view.state.tr.setSelection(TextSelection.create(view.state.doc, paragraphs[1].pos + 5)),
    );

    expect(pressEditorKey(view, 'Backspace')).toBe(true);
    expect(editor.getMarkdown()).toBe('- [ ] 第一项');
  });

  it('removes task markers when cancelling task lists through the same list shortcut', () => {
    const bulletTarget = document.createElement('div');
    const bulletEditor = createEditorCore({ markdown: '- [ ] 待办事项', target: bulletTarget });

    bulletEditor.execute({ type: 'toggleBulletList' });
    expect(bulletEditor.getMarkdown()).toBe('待办事项');

    const orderedTarget = document.createElement('div');
    const orderedEditor = createEditorCore({ markdown: '1. [x] 已完成', target: orderedTarget });

    orderedEditor.execute({ type: 'toggleOrderedList' });
    expect(orderedEditor.getMarkdown()).toBe('已完成');
  });

  it('removes task markers through the task shortcut while preserving the list type', () => {
    const bulletTarget = document.createElement('div');
    const bulletEditor = createEditorCore({ markdown: '- [ ] 待办事项', target: bulletTarget });

    bulletEditor.execute({ type: 'toggleTaskList' });
    expect(bulletEditor.getMarkdown()).toBe('- 待办事项');

    const orderedTarget = document.createElement('div');
    const orderedEditor = createEditorCore({ markdown: '1. [x] 已完成', target: orderedTarget });

    orderedEditor.execute({ type: 'toggleTaskList' });
    expect(orderedEditor.getMarkdown()).toBe('1. 已完成');
  });

  it('toggles task lists from and back to plain paragraphs', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '待办事项', target });

    editor.execute({ type: 'toggleTaskList' });
    expect(editor.getMarkdown()).toBe('- [ ] 待办事项');

    editor.execute({ type: 'toggleTaskList' });
    expect(editor.getMarkdown()).toBe('- 待办事项');
  });

  it('renders editable HTML blocks with their original root element and attributes', () => {
    const target = document.createElement('div');
    createEditorCore({
      markdown:
        '<section class="demo-html-block" id="demo"><strong>HTML 块：</strong><span>允许渲染内联 HTML 内容。</span></section>',
      target,
    });

    const htmlCard = target.querySelector('.html-card');
    const contentRoot = target.querySelector(
      '.html-card > section.html-card-content.demo-html-block#demo',
    );

    expect(htmlCard).not.toBeNull();
    expect(contentRoot).not.toBeNull();
    expect(contentRoot?.textContent?.replace(/\*\*/g, '')).toContain(
      'HTML 块：允许渲染内联 HTML 内容。',
    );
  });

  it('renders fallback aligned paragraph HTML as one inline widget row', () => {
    const target = document.createElement('div');
    createEditorCore({
      markdown:
        '<p align="center">\n  <a href="./README.md"><strong>简体中文</strong></a>\n  ·\n  <a href="./README.en.md">English</a>\n</p>',
      target,
    });

    const widget = target.querySelector<HTMLElement>('.html-widget');

    expect(widget).not.toBeNull();
    expect(widget?.classList.contains('html-widget-aligned-paragraph')).toBe(true);
    expect(widget?.style.display).toBe('block');
    expect(widget?.style.textAlign).toBe('center');
    expect(widget?.style.whiteSpace).toBe('normal');
    expect(widget?.querySelector('p')).toBeNull();
    expect(widget?.querySelectorAll('a')).toHaveLength(2);
    expect(widget?.textContent?.replace(/\s+/g, ' ').trim()).toBe('简体中文 · English');
  });

  it('scrolls to the nth heading via scrollToHeading command', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '# First\n## Second\n### Third\n\n正文内容',
      target,
    });

    // scrollToHeading 命令应成功执行（返回 true）
    const result = editor.execute({
      type: 'scrollToHeading',
      headingIndex: 0,
      text: 'First',
      level: 1,
    });
    expect(result).toBe(true);

    // 执行第二个标题
    const result2 = editor.execute({
      type: 'scrollToHeading',
      headingIndex: 1,
      text: 'Second',
      level: 2,
    });
    expect(result2).toBe(true);

    // 执行第三个标题
    const result3 = editor.execute({
      type: 'scrollToHeading',
      headingIndex: 2,
      text: 'Third',
      level: 3,
    });
    expect(result3).toBe(true);
  });

  it('moves an outline subtree in one undoable transaction and follows its root heading', () => {
    const markdown = '# Alpha\n\nalpha body\n\n## Alpha child\n\nchild body\n\n# Beta\n\nbeta body';
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown, target });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(
      editor.execute({
        type: 'moveOutlineSection',
        sourceIndex: 0,
        targetIndex: 2,
        placement: 'inside',
      }),
    ).toBe(true);
    expect(editor.getMarkdown()).toBe(
      '# Beta\n\nbeta body\n\n## Alpha\n\nalpha body\n\n### Alpha child\n\nchild body',
    );
    const sourceResult = reorderOutlineSection(markdown, extractOutline(markdown), {
      sourceIndex: 0,
      targetIndex: 2,
      placement: 'inside',
    });
    expect(sourceResult.ok).toBe(true);
    if (sourceResult.ok) {
      expect(
        extractOutline(editor.getMarkdown()).map(({ title, level }) => ({ title, level })),
      ).toEqual(
        extractOutline(sourceResult.markdown).map(({ title, level }) => ({ title, level })),
      );
    }
    expect(view.state.selection.$from.parent.textContent).toBe('Alpha');

    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.getMarkdown()).toBe(markdown);
    expect(editor.execute({ type: 'redo' })).toBe(true);
    expect(editor.getMarkdown()).toContain('## Alpha\n\nalpha body\n\n### Alpha child');

    editor.destroy();
  });

  it('keeps derived TOC synchronization inside the section move undo step', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '<!-- toc -->\n<!-- /toc -->\n\n# One\n\none\n\n# Two\n\ntwo',
      target,
    });
    const initialMarkdown = editor.getMarkdown();

    expect(
      editor.execute({
        type: 'moveOutlineSection',
        sourceIndex: 0,
        targetIndex: 1,
        placement: 'after',
      }),
    ).toBe(true);
    expect(extractOutline(editor.getMarkdown()).map((item) => item.title)).toEqual(['Two', 'One']);

    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.getMarkdown()).toBe(initialMarkdown);
    expect(editor.execute({ type: 'undo' })).toBe(false);
    expect(editor.execute({ type: 'redo' })).toBe(true);
    expect(extractOutline(editor.getMarkdown()).map((item) => item.title)).toEqual(['Two', 'One']);

    editor.destroy();
  });

  it('returns false for out-of-range headingIndex', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({
      markdown: '# Only One Heading',
      target,
    });

    const result = editor.execute({
      type: 'scrollToHeading',
      headingIndex: 5,
      text: 'Nonexistent',
      level: 1,
    });
    expect(result).toBe(false);
  });

  it('setDirty resets internal dirty state so selection-only transactions stay clean', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    view.dispatch(view.state.tr.insertText('编辑'));
    expect(dirtyEvents.at(-1)).toBe(true);

    editor.setDirty(false);

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1)));
    expect(dirtyEvents.at(-1)).toBe(false);
  });

  it('defers semantic markdown serialization until the content sync debounce fires', () => {
    vi.useFakeTimers();
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const events: Array<{ reason: string; markdown: string; dirty: boolean }> = [];
    editor.subscribe((event) =>
      events.push({ reason: event.reason, markdown: event.markdown, dirty: event.dirty }),
    );

    view.dispatch(view.state.tr.insertText('编辑'));

    expect(events.at(-1)).toEqual({
      reason: 'content-pending',
      markdown: '正文',
      dirty: true,
    });

    vi.advanceTimersByTime(119);
    expect(events.at(-1)?.reason).toBe('content-pending');

    vi.advanceTimersByTime(1);
    expect(events.at(-1)?.reason).toBe('content-sync');
    expect(events.at(-1)?.markdown).toBe('编辑正文');
    expect(editor.getMarkdown()).toBe('编辑正文');
  });

  it('flushes pending semantic markdown synchronously through getMarkdown', () => {
    vi.useFakeTimers();
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const events: string[] = [];
    editor.subscribe((event) => events.push(event.reason));

    view.dispatch(view.state.tr.insertText('同步'));
    expect(events.at(-1)).toBe('content-pending');

    expect(editor.getMarkdown()).toBe('同步正文');
    expect(events.at(-1)).toBe('content-sync');

    const syncCount = events.filter((reason) => reason === 'content-sync').length;
    vi.advanceTimersByTime(120);
    expect(events.filter((reason) => reason === 'content-sync')).toHaveLength(syncCount);
  });

  it('limits search highlight decorations while keeping the active match visible', () => {
    const markdown = Array.from({ length: 1505 }, () => 'x').join(' ');
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown, target });
    const matches = editor.findSearchMatches('x', { caseSensitive: true });

    expect(matches).toHaveLength(1505);

    editor.setSearchHighlights(matches, 1500);

    expect(target.querySelectorAll('.search-match, .search-match-active')).toHaveLength(1000);
    expect(target.querySelectorAll('.search-match-active')).toHaveLength(1);
  });

  it('setDirty(false) makes the current markdown the clean baseline', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    view.dispatch(view.state.tr.insertText('已保存'));
    expect(dirtyEvents.at(-1)).toBe(true);

    editor.setDirty(false);
    const tempFrom = view.state.doc.content.size - 1;
    view.dispatch(view.state.tr.insertText('临时', tempFrom));
    expect(dirtyEvents.at(-1)).toBe(true);

    view.dispatch(view.state.tr.delete(tempFrom, tempFrom + '临时'.length));
    expect(editor.getMarkdown()).toBe('已保存正文');
    expect(dirtyEvents.at(-1)).toBe(false);
  });

  it('clears dirty state when undo restores original content', () => {
    vi.useFakeTimers();
    const target = document.createElement('div');
    const originalMarkdown = '正文\n\n';
    const editor = createEditorCore({ markdown: originalMarkdown, target });
    const view = (editor as unknown as { view: EditorView }).view;
    const events: Array<{ reason: string; dirty: boolean }> = [];
    editor.subscribe((event) => events.push({ reason: event.reason, dirty: event.dirty }));

    // 输入内容触发 dirty
    view.dispatch(view.state.tr.insertText('1'));
    vi.advanceTimersByTime(120);
    expect(events.at(-1)).toEqual({ reason: 'content-sync', dirty: true });

    // 撤销回到原始内容，dirty 应恢复为 false
    editor.execute({ type: 'undo' });
    expect(events.at(-1)).toEqual({ reason: 'content-pending', dirty: false });
    vi.advanceTimersByTime(120);
    expect(events.at(-1)).toEqual({ reason: 'content-sync', dirty: false });
    expect(editor.getMarkdown()).toBe(originalMarkdown);
  });

  it('clears dirty state when deleting typed text restores original content', () => {
    vi.useFakeTimers();
    const target = document.createElement('div');
    const originalMarkdown = '正文\n\n';
    const editor = createEditorCore({ markdown: originalMarkdown, target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    // 在末尾输入 '1'
    const endPos = view.state.doc.content.size;
    view.dispatch(view.state.tr.insertText('1', endPos - 1));
    expect(dirtyEvents.at(-1)).toBe(true);
    vi.advanceTimersByTime(120);

    // 删除输入的 '1' 回到原始内容
    view.dispatch(view.state.tr.delete(endPos - 1, endPos));
    expect(dirtyEvents.at(-1)).toBe(false);
    vi.advanceTimersByTime(120);
    expect(editor.getMarkdown()).toBe(originalMarkdown);
    expect(dirtyEvents.at(-1)).toBe(false);
  });

  it('preserves unsaved front matter when body content returns to the saved baseline', () => {
    vi.useFakeTimers();
    const originalMarkdown = '---\ntitle: saved\n---\n\n正文\n\n';
    const changedFrontMatterMarkdown = '---\ntitle: draft\n---\n\n正文\n\n';
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: originalMarkdown, target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    editor.setMarkdown(changedFrontMatterMarkdown);
    expect(dirtyEvents.at(-1)).toBe(true);

    const endPos = view.state.doc.content.size;
    view.dispatch(view.state.tr.insertText('1', endPos - 1));
    vi.advanceTimersByTime(120);
    view.dispatch(view.state.tr.delete(endPos - 1, endPos));
    vi.advanceTimersByTime(120);

    expect(editor.getMarkdown()).toBe(changedFrontMatterMarkdown);
    expect(dirtyEvents.at(-1)).toBe(true);

    const bodyOnlyMarkdown = '正文\n\n';
    const addedFrontMatterMarkdown = '---\ntitle: draft\n---\n\n正文\n\n';
    const secondTarget = document.createElement('div');
    const secondEditor = createEditorCore({ markdown: bodyOnlyMarkdown, target: secondTarget });
    const secondView = (secondEditor as unknown as { view: EditorView }).view;
    const secondDirtyEvents: boolean[] = [];
    secondEditor.subscribe((event) => secondDirtyEvents.push(event.dirty));

    secondEditor.setMarkdown(addedFrontMatterMarkdown);
    const secondEndPos = secondView.state.doc.content.size;
    secondView.dispatch(secondView.state.tr.insertText('1', secondEndPos - 1));
    vi.advanceTimersByTime(120);
    secondView.dispatch(secondView.state.tr.delete(secondEndPos - 1, secondEndPos));
    vi.advanceTimersByTime(120);

    expect(secondEditor.getMarkdown()).toBe(addedFrontMatterMarkdown);
    expect(secondDirtyEvents.at(-1)).toBe(true);
  });

  it('clears dirty state when source input restores original markdown', () => {
    const editor = createEditorCore({ markdown: '正文' });
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    editor.setMarkdown('正文1', { sourceInput: true });
    expect(dirtyEvents.at(-1)).toBe(true);

    editor.setMarkdown('正文', { sourceInput: true });
    expect(dirtyEvents.at(-1)).toBe(false);
  });

  it('remains dirty after undo if content still differs from original', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    // 先输入 '1'，再移动光标（改变选区但不改变 doc），再输入 '2'
    // 中间加入 selection-only 事务可以打断 ProseMirror history 的合并
    const pos1 = view.state.doc.content.size - 1;
    view.dispatch(view.state.tr.insertText('1', pos1));
    expect(dirtyEvents.at(-1)).toBe(true);

    // 移动选区以打断 history 合并
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1)));

    // 在另一个位置输入 '2'
    view.dispatch(view.state.tr.insertText('2', 1));
    expect(dirtyEvents.at(-1)).toBe(true);

    // 撤销一步只删除 '2'，内容仍与原始不同（还剩 '1'）
    editor.execute({ type: 'undo' });
    expect(dirtyEvents.at(-1)).toBe(true);
    expect(editor.getMarkdown()).toContain('1');
  });

  it('resets original baseline on open-file and stays clean', () => {
    const editor = createEditorCore({ markdown: '旧内容' });
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    editor.setMarkdown('新内容', { reason: 'open-file' });
    expect(dirtyEvents.at(-1)).toBe(false);

    // 编辑后再撤销应能回到 clean 状态
    const target = document.createElement('div');
    const editor2 = createEditorCore({ markdown: '基准', target });
    const view2 = (editor2 as unknown as { view: EditorView }).view;
    const dirtyEvents2: boolean[] = [];
    editor2.subscribe((event) => dirtyEvents2.push(event.dirty));

    editor2.setMarkdown('基准', { reason: 'open-file' });
    view2.dispatch(view2.state.tr.insertText('1'));
    expect(dirtyEvents2.at(-1)).toBe(true);
    editor2.execute({ type: 'undo' });
    expect(dirtyEvents2.at(-1)).toBe(false);
  });

  it('uses savedMarkdown as the clean baseline when switching to a dirty tab', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '其他文档', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const dirtyEvents: boolean[] = [];
    editor.subscribe((event) => dirtyEvents.push(event.dirty));

    editor.setMarkdown('已保存正文1', {
      reason: 'switch-tab',
      dirty: true,
      savedMarkdown: '已保存正文',
    });
    expect(dirtyEvents.at(-1)).toBe(true);

    const tempFrom = view.state.doc.content.size - 1;
    view.dispatch(view.state.tr.insertText('临时', tempFrom));
    expect(dirtyEvents.at(-1)).toBe(true);

    view.dispatch(view.state.tr.delete(tempFrom, tempFrom + '临时'.length));
    expect(editor.getMarkdown()).toBe('已保存正文1');
    expect(dirtyEvents.at(-1)).toBe(true);

    const suffixFrom = view.state.doc.content.size - 2;
    view.dispatch(view.state.tr.delete(suffixFrom, suffixFrom + 1));
    expect(editor.getMarkdown()).toBe('已保存正文');
    expect(dirtyEvents.at(-1)).toBe(false);
  });

  it.each([
    ['# 标题', 'heading'],
    ['- 第一项\n- 第二项', 'bullet_list'],
    ['> 引用', 'blockquote'],
    ['---', 'horizontal_rule'],
    ['```ts\nconst value = 1;\n```', 'code_block'],
    ['| A | B |\n| --- | --- |\n| 1 | 2 |', 'table'],
  ])('renders pasted Markdown immediately: %s', (clipboard, expectedNodeType) => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(editor.pasteClipboard({ text: clipboard })).toMatchObject({
      status: 'inserted',
      format: 'markdown',
    });
    expect(getTopLevelNodeNames(view.state.doc)).toContain(expectedNodeType);
    expect(getTopLevelNodeNames(parseMarkdown(editor.flushMarkdown()))).toContain(expectedNodeType);
    editor.destroy();
  });

  it('prefers detected Markdown text over HTML and otherwise keeps HTML priority', () => {
    const markdownTarget = document.createElement('div');
    const markdownEditor = createEditorCore({ markdown: '', target: markdownTarget });
    const markdownView = (markdownEditor as unknown as { view: EditorView }).view;
    expect(markdownEditor.pasteClipboard({ text: '# 文本', html: '<p>HTML</p>' })).toMatchObject({
      format: 'markdown',
    });
    expect(markdownView.state.doc.firstChild?.type.name).toBe('heading');
    markdownEditor.destroy();

    const htmlTarget = document.createElement('div');
    const htmlEditor = createEditorCore({ markdown: '', target: htmlTarget });
    expect(htmlEditor.pasteClipboard({ text: '普通文本', html: '<strong>HTML</strong>' })).toMatchObject({
      format: 'html',
    });
    expect(htmlEditor.flushMarkdown()).toContain('**HTML**');
    htmlEditor.destroy();
  });

  it('splits a paragraph around structured Markdown pasted in the middle', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '前后', target });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));

    editor.pasteClipboard({ text: '- 第一项\n- 第二项' });

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['paragraph', 'bullet_list', 'paragraph']);
    expect(view.state.doc.firstChild?.textContent).toBe('前');
    expect(view.state.doc.lastChild?.textContent).toBe('后');
    editor.destroy();
  });

  it('falls back to literal text when a table cell cannot contain pasted block structure', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '| A |\n| --- |\n| 单元格 |', target });
    const view = (editor as unknown as { view: EditorView }).view;
    const paragraph = findNodeByText(view.state.doc, 'paragraph', '单元格');
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, paragraph.pos + 1)));

    expect(editor.pasteClipboard({ text: '# 标题' })).toMatchObject({ format: 'plain' });
    expect(findFirstNode(view.state.doc, 'table').node.textContent).toContain('# 标题');
    expect(parseMarkdown(editor.flushMarkdown()).textContent).toContain('# 标题单元格');
    editor.destroy();
  });

  it('pastes syntax literally without triggering inline mark or math plugins', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;

    expect(
      editor.pasteClipboard({ text: '**重点** $x$ ~~删除~~' }, { mode: 'plain' }),
    ).toMatchObject({ format: 'plain' });
    expect(view.state.doc.textContent).toBe('**重点** $x$ ~~删除~~');
    expect(findFirstNode(view.state.doc, 'paragraph').node.firstChild?.marks).toHaveLength(0);
    expect(editor.flushMarkdown()).toBe('\\*\\*重点\\*\\* \\$x\\$ \\~\\~删除\\~\\~');
    expect(parseMarkdown(editor.getMarkdown()).textContent).toBe('**重点** $x$ ~~删除~~');
    editor.destroy();
  });

  it('uses Ctrl/Cmd+Shift+V as a one-shot plain-text paste request', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dom.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'V',
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );

    const pasteEvent = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(pasteEvent, 'clipboardData', {
      value: {
        files: [],
        getData: (type: string) => (type === 'text/plain' ? '# 字面标题' : ''),
      },
    });
    const handled = view.someProp('handlePaste', (handler) =>
      handler(view, pasteEvent as ClipboardEvent, Slice.empty),
    );

    expect(handled).toBe(true);
    expect(pasteEvent.defaultPrevented).toBe(true);
    expect(view.state.doc.firstChild?.type.name).toBe('paragraph');
    expect(view.state.doc.textContent).toBe('# 字面标题');
    expect(editor.flushMarkdown()).toBe('\\# 字面标题');
    editor.destroy();
  });

  it('keeps pasted display-math delimiters literal after save and reopen', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    editor.pasteClipboard({ text: '$$\nE = mc^2\n$$' }, { mode: 'plain' });

    const serialized = editor.flushMarkdown();
    const reopened = parseMarkdown(serialized);
    expect(serialized).toContain('\\$\\$');
    expect(reopened.firstChild?.type.name).toBe('paragraph');
    expect(reopened.textContent).toBe('$$E = mc^2$$');
    editor.destroy();
  });

  it.each([
    '# 标题',
    '- 列表',
    '1. 列表',
    '> 引用',
    '---',
    '```ts\nconst value = 1;\n```',
    '**重点** ~~删除~~',
    '[链接](https://example.com)',
    '![图片](./image.png)',
    '$x^2$',
    '$$\nE = mc^2\n$$',
    '| A | B |\n| --- | --- |\n| 1 | 2 |',
    '<u>下划线</u>',
    '> [!NOTE]\n> 内容',
  ])('preserves plain-pasted Markdown syntax across serialization: %s', (text) => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;
    editor.pasteClipboard({ text }, { mode: 'plain' });

    const pastedText = view.state.doc.textContent;
    const reopened = parseMarkdown(editor.flushMarkdown());
    expect(reopened.firstChild?.type.name).toBe('paragraph');
    expect(reopened.textContent).toBe(pastedText);
    editor.destroy();
  });

  it('promotes front matter only in an empty document and keeps it in one history event', () => {
    const clipboard = '---\ntitle: 测试\n---\n\n# 正文';
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const view = (editor as unknown as { view: EditorView }).view;

    editor.pasteClipboard({ text: clipboard });
    expect(view.state.doc.attrs.frontMatterPrefix).toBe('---\ntitle: 测试\n---\n\n');
    expect(view.state.doc.firstChild?.type.name).toBe('heading');
    expect(editor.flushMarkdown()).toBe(clipboard);

    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.flushMarkdown()).toBe('');
    expect(view.state.doc.attrs.frontMatterPrefix).toBe('');
    expect(editor.execute({ type: 'redo' })).toBe(true);
    expect(editor.flushMarkdown()).toBe(clipboard);
    editor.destroy();
  });

  it('keeps pasted front matter literal in a non-empty document', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '已有正文', target });
    const view = (editor as unknown as { view: EditorView }).view;
    view.dispatch(view.state.tr.setSelection(new AllSelection(view.state.doc)));

    expect(editor.pasteClipboard({ text: '---\ntitle: 新标题\n---\n\n# 新正文' })).toMatchObject({
      format: 'plain',
    });
    expect(view.state.doc.attrs.frontMatterPrefix).toBe('');
    expect(view.state.doc.textContent).toContain('title: 新标题');
    editor.destroy();
  });

  it('allows a paste larger than the semantic document limit', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });
    const largeText = 'a'.repeat(300_001);

    expect(editor.pasteClipboard({ text: largeText })).toMatchObject({
      status: 'inserted',
      format: 'plain',
    });
    expect(editor.flushMarkdown()).toBe(largeText);
    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.flushMarkdown()).toBe('');
    editor.destroy();
  });
});
