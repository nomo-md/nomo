import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { createEditorCore } from './createEditorCore';
import { projectInlineSource } from './InlineSourceCodec';
import { setTypographyOptions } from '../typography/options';
import { DEFAULT_TYPOGRAPHY } from '../typography/types';
import { schema } from './schema';

beforeEach(() => setTypographyOptions({ ...DEFAULT_TYPOGRAPHY, enabled: false }));
afterEach(() => {
  setTypographyOptions(DEFAULT_TYPOGRAPHY);
  vi.useRealTimers();
});

function setup(markdown: string) {
  const target = document.createElement('div');
  document.body.append(target);
  const editor = createEditorCore({ markdown, target });
  const view = (editor as unknown as { view: EditorView }).view;
  return {
    editor,
    view,
    target,
    destroy: () => {
      editor.destroy();
      target.remove();
    },
  };
}

describe('inline source editor integration', () => {
  it('retains every delimiter caret position and deletes just one asterisk', () => {
    const { editor, view, destroy } = setup('**正文**');
    for (let position = 1; position <= 7; position++) {
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, position)));
      expect(view.state.selection.from).toBe(position);
      let intercepted = false;
      view.someProp('handleKeyDown', (handler) => {
        intercepted ||= Boolean(handler(view, new KeyboardEvent('keydown', { key: 'ArrowRight' })));
        return intercepted;
      });
      expect(intercepted).toBe(false);
    }
    view.dispatch(view.state.tr.delete(1, 2));
    expect(editor.flushMarkdown()).toBe('*正文**');
    expect(editor.execute({ type: 'undo' })).toBe(true);
    expect(editor.flushMarkdown()).toBe('**正文**');
    destroy();
  });

  it('changes only decorations when a different block becomes active', () => {
    const { editor, view, target, destroy } = setup('**甲**\n\n__乙__');
    const original = view.state.doc;
    let dirty = false;
    let revision = 0;
    editor.subscribe((event) => {
      dirty = event.dirty;
      revision = event.contentRevision ?? 0;
    });
    const originalRevision = revision;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 8)));
    expect(view.state.doc).toBe(original);
    expect(revision).toBe(originalRevision);
    expect(dirty).toBe(false);
    expect(target.querySelectorAll('.pm-inline-source-marker.is-hidden')).toHaveLength(2);
    destroy();
  });

  it('reveals only the current format inside one paragraph and hides all while in plain text', () => {
    const source = '**甲** 普通 __乙__';
    const { editor, view, target, destroy } = setup(source);
    const original = view.state.doc;
    const visibleMarkers = () =>
      Array.from(target.querySelectorAll('.pm-inline-source-marker:not(.is-hidden)')).map(
        (element) => element.textContent,
      );
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
    expect(visibleMarkers()).toEqual(['**', '**']);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 8)));
    expect(visibleMarkers()).toEqual([]);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 12)));
    expect(visibleMarkers()).toEqual(['__', '__']);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1, 15)));
    expect(visibleMarkers()).toEqual(['**', '**', '__', '__']);
    expect(view.state.doc).toBe(original);
    expect(editor.getMarkdown()).toBe(source);
    destroy();
  });

  it('cleans only untouched generated empty pairs on flush and when leaving', () => {
    const { editor, view, destroy } = setup('');
    editor.execute({ type: 'toggleBold' });
    expect(view.state.doc.textContent).toBe('****');
    expect(editor.getMarkdown()).toBe('****');
    expect(editor.flushMarkdown()).toBe('');
    editor.execute({ type: 'toggleItalic' });
    view.dispatch(view.state.tr.insertText('x'));
    view.dispatch(view.state.tr.delete(2, 3));
    // An edited-then-emptied pair is user content, never an untouched template.
    expect(editor.flushMarkdown()).toBe('**');
    destroy();
  });

  it('awaits composition before cleaning and serializing an empty template', async () => {
    const { editor, view, destroy } = setup('');
    editor.execute({ type: 'toggleBold' });
    view.dom.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    let finished = false;
    const pending = editor.awaitCompositionEnd!().then(() => {
      finished = true;
    });
    editor.commitPendingEdits();
    expect(view.state.doc.textContent).toBe('****');
    await Promise.resolve();
    expect(finished).toBe(false);
    view.dispatch(view.state.tr.insertText('中文'));
    view.dom.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '中文' }));
    await pending;
    expect(editor.flushMarkdown()).toBe('**中文**');
    destroy();
  });

  it('preserves source through snapshots, source mode, unmount and remount', () => {
    const source =
      '__粗体__、_斜体_、~~删除~~、<u>下划线</u>、<mark>高亮</mark>、`` `代码` ``、\\*字面\\*、**未闭合';
    const { editor, view, target, destroy } = setup(source);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
    const snapshot = editor.getSnapshot();
    editor.updateOptions({ mode: 'source' });
    editor.setMarkdown(source, { sourceInput: true });
    editor.updateOptions({ mode: 'semantic' });
    editor.unmount();
    editor.mount(target);
    editor.restoreSnapshot(snapshot);
    expect(editor.flushMarkdown()).toBe(source);
    expect(editor.getSnapshot().selection).toEqual({ anchor: 2, head: 2 });
    destroy();
  });

  it('projects partial rich selections using the complete source context', () => {
    const { editor, view, destroy } = setup('**重点**');
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 3, 5)));
    expect(editor.getClipboardPayload()).toMatchObject({ text: '重点' });
    expect(editor.getClipboardPayload()?.html).toContain('<strong>重点</strong>');
    editor.updateOptions({ copyMarkdownSyntaxEnabled: false });
    expect(editor.getClipboardPayload()?.text).toBe('重点');
    destroy();
  });

  it('maps legacy snapshot coordinates to real source while preserving selected text', () => {
    const { editor, destroy } = setup('');
    editor.restoreSnapshot({
      markdown: '**hello** tail',
      version: 3,
      selection: { anchor: 2, head: 5 },
    });
    expect(editor.getSnapshot().selection).toEqual({ anchor: 4, head: 7 });
    expect(editor.getClipboardPayload()?.text).toBe('ell');
    expect(editor.getClipboardPayload()?.html).toContain('<strong>ell</strong>');
    destroy();
  });

  it('materializes a restored legacy PM state and maps its selection at the entry point', () => {
    const editor = createEditorCore({ markdown: '**hello**' });
    const doc = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, schema.text('hello', [schema.marks.strong.create()])),
    );
    (editor as unknown as { suspendedState: EditorState }).suspendedState = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 2, 5),
    });
    const target = document.body.appendChild(document.createElement('div'));
    editor.mount(target);
    expect(editor.getSnapshot().selection).toEqual({ anchor: 4, head: 7 });
    expect(editor.getClipboardPayload()?.text).toBe('ell');
    expect(editor.getClipboardPayload()?.html).toContain('<strong>ell</strong>');
    editor.destroy();
    target.remove();
  });

  it('materializes HTML paste and projects all six formats for export', () => {
    const { editor, view, destroy } = setup('');
    editor.pasteClipboardHtml(
      '<p><strong>粗</strong> <em>斜</em> <code>x*y</code> <s>删</s> <u>线</u> <mark>亮</mark></p>',
    );
    const marks = new Set<string>();
    view.state.doc.descendants((node) => node.marks.forEach((mark) => marks.add(mark.type.name)));
    expect([...marks]).toEqual(['inline_source']);
    const html = editor.getExportHtml!();
    expect(html).toContain('<strong>粗</strong>');
    expect(html).toContain('<em>斜</em>');
    expect(html).toContain('x*y</code>');
    expect(html).toContain('<u>线</u>');
    expect(html).toContain('<mark>亮</mark>');
    expect(html).not.toContain('**');
    expect(projectInlineSource(view.state.doc).textContent).toBe('粗 斜 x*y 删 线 亮');
    destroy();
  });

  it('keeps source when copying part of a table and projects the plain-text option', () => {
    const { editor, view, destroy } = setup('| A |\n| --- |\n| **粗体** |');
    let from = 0;
    view.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === '**粗体**') from = pos;
    });
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, from, from + 6)));
    expect(editor.getClipboardPayload()?.text).toBe('**粗体**');
    expect(editor.getClipboardPayload()?.html).toContain('<strong>粗体</strong>');
    editor.updateOptions({ copyMarkdownSyntaxEnabled: false });
    expect(editor.getClipboardPayload()?.text).toBe('粗体');
    destroy();
  });
});
