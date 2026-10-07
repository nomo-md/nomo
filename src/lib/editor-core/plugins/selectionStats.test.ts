import { describe, expect, it, vi } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { createEditorCore } from '../createEditorCore';
import { parseMarkdown } from '../markdown';
import { inlineCodeSelectionBridgePlugin } from './inlineCodeSelectionBridge';

describe('selection statistics hot path', () => {
  it('does not construct legacy Markdown selection events for snapshot subscribers', () => {
    const onSelectionSnapshotChange = vi.fn();
    const editor = createEditorCore({
      markdown: '# 中文\n\n`hello`',
      target: document.createElement('div'),
      onSelectionSnapshotChange,
    });
    const internals = editor as unknown as { view: EditorView; createSelectionEvent(): unknown };
    const legacy = vi.spyOn(internals, 'createSelectionEvent');
    const json = vi.spyOn(internals.view.state.doc, 'toJSON');
    internals.view.dispatch(
      internals.view.state.tr.setSelection(TextSelection.create(internals.view.state.doc, 1, 2)),
    );
    expect(onSelectionSnapshotChange).toHaveBeenLastCalledWith({
      selection: { anchor: 1, head: 2 },
      contentRevision: 0,
    });
    expect(legacy).not.toHaveBeenCalled();
    expect(json).not.toHaveBeenCalled();
    editor.destroy();
  });
  it('only scans inline code ranges when the document changes', () => {
    const doc = parseMarkdown('正文 `code`\n\n第二段 `another`');
    const scan = vi.spyOn(doc, 'descendants');
    let state = EditorState.create({ doc, plugins: [inlineCodeSelectionBridgePlugin()] });
    expect(scan).toHaveBeenCalledOnce();
    scan.mockClear();
    for (let to = 2; to < 8; to++)
      state = state.apply(state.tr.setSelection(TextSelection.create(doc, 1, to)));
    expect(scan).not.toHaveBeenCalled();
    const transaction = state.tr.insertText('新增', 1);
    const newScan = vi.spyOn(transaction.doc, 'descendants');
    state.apply(transaction);
    expect(newScan).toHaveBeenCalledOnce();
  });
});
