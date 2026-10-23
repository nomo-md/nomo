import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEditorCore } from '../../lib/editor-core';
import { extractOutline } from '../../lib/outline/outlineService';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import { createOutlineInteractionController } from './outlineInteractionController';
import * as applicationSessions from './markdownApplicationSession';

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('outlineInteractionController', () => {
  it('requires a complete matching revision and rejects an edit while a chapter move is queued', () => {
    const markdown = '# One\n\none\n\n# Two\n\ntwo';
    const outline = extractOutline(markdown);
    const editor = createEditorCore({ markdown });
    const state = { status: { revision: 2 }, index: { revision: 2, progress: { state: 'partial' }, outline } };
    let apply: ((text: string) => string) | undefined;
    const transformMarkdown = vi.fn((transform: (text: string) => string) => {
      apply = transform;
      return Promise.resolve();
    });
    vi.spyOn(applicationSessions, 'getMarkdownApplicationSession').mockReturnValue({
      document: { state }, transformMarkdown,
    } as unknown as applicationSessions.MarkdownApplicationSession);
    const controller = createOutlineInteractionController({
      getMode: () => 'semantic', getMarkdown: () => markdown, getOutline: () => outline,
      getCollapsedOutlineIds: () => new Set(), setCollapsedOutlineIds: vi.fn(),
      getOutlineVisible: () => true, setOutlineVisible: vi.fn(), setActiveOutlineId: vi.fn(),
      getSuppressOutlineScrollUntil: () => 0, setSuppressOutlineScrollUntil: vi.fn(),
      getSemanticPane: () => document.createElement('section'),
      getSourcePane: () => document.createElement('section'), getSourceEditor: () => undefined,
      getEditor: () => editor, getReadonly: () => false, setStatusMessage: vi.fn(),
    });
    const request = { sourceIndex: 0, targetIndex: 1, placement: 'after' } as const;
    expect(controller.moveOutlineSection(request)).toBe(false);
    state.index.progress.state = 'ready';
    state.index.revision = 1;
    expect(controller.moveOutlineSection(request)).toBe(false);
    expect(transformMarkdown).not.toHaveBeenCalled();
    state.index.revision = 2;
    expect(controller.moveOutlineSection(request)).toBe(true);
    expect(extractOutline(apply!(markdown)).map(item => item.title)).toEqual(['Two', 'One']);
    state.status.revision = 3;
    expect(() => apply!(markdown)).toThrow();
    editor.destroy();
  });

  it('restores collapsed and active identity after the deferred source analysis', async () => {
    vi.useFakeTimers();
    const markdown = '# Same\n## Child\n# Same\n';
    let outline = extractOutline(markdown);
    const textarea = document.createElement('textarea');
    textarea.value = markdown;
    const editor = createEditorCore({ markdown });
    const setMarkdown = vi.spyOn(editor, 'setMarkdown');
    let collapsed = new Set([outline[0].id]);
    let activeId = '';
    const setStatusMessage = vi.fn();
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
    const sourceEditor = createSourceEditorStub(textarea, (value) => {
      editor.setMarkdown(textarea.value, { reason: 'source-input', sourceInput: true });
      outline = extractOutline(value);
    });
    const controller = createOutlineInteractionController({
      getMode: () => 'source',
      getMarkdown: () => markdown,
      getOutline: () => outline,
      getCollapsedOutlineIds: () => collapsed,
      setCollapsedOutlineIds: (value) => {
        collapsed = value;
      },
      getOutlineVisible: () => true,
      setOutlineVisible: vi.fn(),
      setActiveOutlineId: (value) => {
        activeId = value;
      },
      getSuppressOutlineScrollUntil: () => 0,
      setSuppressOutlineScrollUntil: vi.fn(),
      getSemanticPane: () => document.createElement('section'),
      getSourcePane: () => document.createElement('section'),
      getSourceEditor: () => sourceEditor,
      getEditor: () => editor,
      getReadonly: () => false,
      setStatusMessage,
    });

    expect(
      controller.moveOutlineSection({ sourceIndex: 0, targetIndex: 2, placement: 'after' }),
    ).toBe(true);
    expect(extractOutline(textarea.value).map((item) => item.title)).toEqual([
      'Same',
      'Same',
      'Child',
    ]);
    expect(activeId).toBe('');
    await vi.advanceTimersByTimeAsync(150);
    expect(activeId).toBe('same-2');
    expect(collapsed).toEqual(new Set(['same-2']));
    expect(setStatusMessage).toHaveBeenCalled();
    expect(setMarkdown).toHaveBeenCalledTimes(1);

    editor.destroy();
  });

  it('flushes a semantic move immediately so document and outline update together', () => {
    let markdown = '# One\n\none\n\n# Two\n\ntwo';
    let outline = extractOutline(markdown);
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown, mode: 'semantic', target });
    editor.subscribe((event) => {
      if (event.reason !== 'content-sync') return;
      markdown = event.markdown;
      outline = extractOutline(markdown);
    });
    let activeId = '';
    const controller = createOutlineInteractionController({
      getMode: () => 'semantic',
      getMarkdown: () => markdown,
      getOutline: () => outline,
      getCollapsedOutlineIds: () => new Set(),
      setCollapsedOutlineIds: vi.fn(),
      getOutlineVisible: () => true,
      setOutlineVisible: vi.fn(),
      setActiveOutlineId: (value) => {
        activeId = value;
      },
      getSuppressOutlineScrollUntil: () => 0,
      setSuppressOutlineScrollUntil: vi.fn(),
      getSemanticPane: () => document.createElement('section'),
      getSourcePane: () => document.createElement('section'),
      getSourceEditor: () => createSourceEditorStub(document.createElement('textarea')),
      getEditor: () => editor,
      getReadonly: () => false,
      setStatusMessage: vi.fn(),
    });

    expect(
      controller.moveOutlineSection({ sourceIndex: 0, targetIndex: 1, placement: 'after' }),
    ).toBe(true);
    expect(outline.map((item) => item.title)).toEqual(['Two', 'One']);
    expect(activeId).toBe('one');
    expect([...target.querySelectorAll('h1')].map((heading) => heading.textContent)).toEqual([
      'Two',
      'One',
    ]);

    editor.destroy();
  });
});

function createSourceEditorStub(
  textarea: HTMLTextAreaElement,
  onChange: (value: string) => void = () => undefined,
): MarkdownSourceEditorHandle {
  return {
    getMarkdown: () => textarea.value,
    placeCaretAtCoordinates: () => false,
    setMarkdown: (value) => {
      textarea.value = value;
      onChange(value);
    },
    getSelection: () => ({ from: textarea.selectionStart, to: textarea.selectionEnd }),
    setSelection: (from, to = from) => textarea.setSelectionRange(from, to),
    getSelectedMarkdown: () => textarea.value.slice(textarea.selectionStart, textarea.selectionEnd),
    focus: () => textarea.focus(),
    revealRange: (from, to = from) => textarea.setSelectionRange(from, to),
    undo: () => false,
    redo: () => false,
    lineAtOffset: (offset) => textarea.value.slice(0, offset).split(/\r?\n/).length,
    offsetAtLine: () => 0,
    getLineCount: () => textarea.value.split(/\r?\n/).length,
    getLineTop: (line) => (line - 1) * 20,
    lineAtHeight: (height) => Math.floor(height / 20) + 1,
    getLineHeight: () => 20,
    getScrollElement: () => textarea,
    getContentElement: () => textarea,
    getContentHeight: () => textarea.scrollHeight,
    getBlockGeometry: () => [],
    applyBlockGaps: () => undefined,
    clearBlockGaps: () => undefined,
    requestMeasure: () => undefined,
  };
}
