import { afterEach, describe, expect, it, vi } from 'vitest';
import { EditorState, TextSelection, type Transaction } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { closeHistory, history, redo, undo, undoDepth } from 'prosemirror-history';
import { schema } from '../schema';
import * as codec from '../InlineSourceCodec';
import {
  clearEmptyInlineTemplates,
  findEmptyInlineSourceTemplate,
  getEmptyInlineSourceTemplates,
  getEmptyInlineTemplateFormats,
  inlineSourceEditingPlugin,
  inlineSourceEditingPluginKey,
  isInlineSourceComposing,
  type InlineSourceEditingOptions,
} from './inlineSourceEditing';

const views: EditorView[] = [];

afterEach(() => {
  for (const view of views.splice(0)) view.destroy();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function source(text: string) {
  return text ? schema.text(text, [schema.marks.inline_source.create()]) : undefined;
}

function createState(texts: string[], options?: InlineSourceEditingOptions) {
  return EditorState.create({
    doc: schema.nodes.doc.create(
      null,
      texts.map((text) => schema.nodes.paragraph.create(null, source(text))),
    ),
    plugins: [history(), inlineSourceEditingPlugin(options)],
  });
}

function apply(state: EditorState, tr: Transaction) {
  return state.applyTransaction(tr).state;
}

function markers(state: EditorState) {
  return inlineSourceEditingPluginKey
    .getState(state)!
    .decorations.find()
    .filter((decoration) => decoration.spec.marker);
}

function hiddenMarkers(state: EditorState) {
  return markers(state).filter((decoration) =>
    (decoration as unknown as { type: { attrs: { class: string } } }).type.attrs.class.includes(
      'is-hidden',
    ),
  );
}

function visibleMarkers(state: EditorState) {
  return markers(state).filter(
    (decoration) =>
      !(decoration as unknown as { type: { attrs: { class: string } } }).type.attrs.class.includes(
        'is-hidden',
      ),
  );
}

function blockStart(state: EditorState, index: number) {
  let start = 1;
  for (let i = 0; i < index; i++) start += state.doc.child(i).nodeSize;
  return start;
}

function addTemplate(state: EditorState, type = 'strong', open = '**', close = '**') {
  const from = state.selection.from;
  const tr = state.tr.insertText(open + close, from);
  const cursor = from + open.length;
  tr.setSelection(TextSelection.create(tr.doc, cursor));
  tr.setMeta('inlineSourceTemplate', {
    action: 'add',
    template: { from, to: from + open.length + close.length, cursor, type },
  });
  return apply(state, tr);
}

function createView(state: EditorState) {
  const host = document.body.appendChild(document.createElement('div'));
  const view = new EditorView(host, { state });
  views.push(view);
  return view;
}

function applyHistory(state: EditorState, command: typeof undo) {
  let next = state;
  expect(
    command(state, (tr) => {
      next = apply(state, tr);
    }),
  ).toBe(true);
  return next;
}

describe('inlineSourceEditingPlugin', () => {
  it('decorates all six source formats while keeping unrelated syntax hidden', () => {
    const text = 'before **bold** *em* `code` ~~del~~ <u>under</u> <mark>hi</mark> after';
    let state = createState([text, '**resting**']);
    const formats = inlineSourceEditingPluginKey
      .getState(state)!
      .decorations.find()
      .map((decoration) => decoration.spec.format)
      .filter(Boolean);
    expect(new Set(formats)).toEqual(
      new Set(['strong', 'em', 'code', 'strikethrough', 'underline', 'highlight']),
    );
    expect(markers(state)).toHaveLength(14);
    expect(hiddenMarkers(state)).toHaveLength(14);
    expect(state.doc.firstChild!.textContent).toBe(text);
    for (let pos = 1; pos <= text.length + 1; pos++) {
      state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, pos)));
      expect(state.selection.from).toBe(pos);
    }
    expect(hiddenMarkers(state)).toHaveLength(14);
    for (const body of ['bold', 'em', 'code', 'del', 'under', 'hi']) {
      state = apply(
        state,
        state.tr.setSelection(TextSelection.create(state.doc, text.indexOf(body) + 2)),
      );
      expect(visibleMarkers(state)).toHaveLength(2);
    }
  });

  it('reveals only the active format in one paragraph, including every delimiter boundary', () => {
    const text = '**one** gap **two**';
    let state = createState([text]);
    for (let pos = 1; pos <= 8; pos++) {
      state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, pos)));
      expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual([
        [1, 3],
        [6, 8],
      ]);
      expect(state.selection.from).toBe(pos);
    }
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 10)));
    expect(visibleMarkers(state)).toHaveLength(0);
    const second = text.indexOf('**two**') + 1;
    for (let pos = second; pos <= second + 7; pos++) {
      state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, pos)));
      expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual([
        [second, second + 2],
        [second + 5, second + 7],
      ]);
    }
    expect(state.doc.textContent).toBe(text);
  });

  it('reveals intersected source spans across paragraphs and keeps source characters intact', () => {
    let state = createState(['**one**', '*two*', '`three`']);
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, 3, blockStart(state, 2) + 3)),
    );
    expect(hiddenMarkers(state)).toHaveLength(0);
    expect(state.doc.textContent).toBe('**one***two*`three`');
  });

  it('reveals nested ancestors locally and leaves unrelated formats hidden', () => {
    const text = '~~***字***~~ and **other**';
    let state = createState([text]);
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, text.indexOf('字') + 1)),
    );
    expect(
      visibleMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['~~***', '***~~']);
    expect(
      hiddenMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['**', '**']);
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 2)));
    expect(
      visibleMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['~~', '~~']);
  });

  it('reveals only spans intersecting a nonempty selection within one paragraph', () => {
    const text = '**one** gap **two** tail **three**';
    let state = createState([text]);
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, 4, text.indexOf('two') + 2)),
    );
    expect(visibleMarkers(state)).toHaveLength(4);
    expect(hiddenMarkers(state)).toHaveLength(2);
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 8, 12)));
    expect(visibleMarkers(state)).toHaveLength(0);
  });

  it.each(['~~***字***~~', '~~**字**~~', '**\\*字\\***'])(
    'keeps nested syntax outside every formatting wrapper: %s',
    (text) => {
      const view = createView(createState([text]));
      const markerElements = view.dom.querySelectorAll('.pm-inline-source-marker');
      expect(markerElements.length).toBeGreaterThan(0);
      for (const marker of markerElements) {
        expect(marker.closest('.pm-inline-source-format')).toBeNull();
        expect(marker.parentElement?.closest('.pm-inline-source-marker')).toBeNull();
        expect(
          marker.querySelector('.pm-inline-source-format,.pm-inline-source-marker'),
        ).toBeNull();
      }
      expect(view.dom.textContent).toBe(text);
      expect(view.dom.querySelector('.pm-inline-source-strong')?.textContent).toContain('字');
      if (text.startsWith('~~'))
        expect(view.dom.querySelector('.pm-inline-source-strikethrough')?.textContent).toBe('字');
      if (text.includes('***字***'))
        expect(view.dom.querySelector('.pm-inline-source-em')?.textContent).toBe('字');
    },
  );

  it('keeps visited format ranges across blocks expanded during drag until release', () => {
    let state = createState(['**one**', '**two**', '**three**']);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: true }));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, 3, blockStart(state, 1) + 2)),
    );
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 2) + 2)),
    );
    expect(hiddenMarkers(state)).toHaveLength(0);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: false }));
    expect(hiddenMarkers(state)).toHaveLength(4);
  });

  it('retains visited format ranges during drag without expanding the rest of their paragraph', () => {
    const text = '**one** gap **two** tail **three**';
    let state = createState([text]);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: true }));
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 3, 4)));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, text.indexOf('three') + 2)),
    );
    expect(visibleMarkers(state)).toHaveLength(4);
    expect(hiddenMarkers(state).map(({ from, to }) => [from, to])).toEqual([
      [13, 15],
      [18, 20],
    ]);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: false }));
    expect(visibleMarkers(state)).toHaveLength(2);
    expect(hiddenMarkers(state)).toHaveLength(4);
  });

  it('switches directly to the clicked format before mouseup without retaining old syntax', () => {
    const text = '**one** gap **two**';
    let state = createState([text]);
    const doc = state.doc;
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: true }));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, text.indexOf('two') + 2)),
    );
    const duringPress = visibleMarkers(state).map(({ from, to }) => [from, to]);
    expect(duringPress).toEqual([
      [13, 15],
      [18, 20],
    ]);
    expect(inlineSourceEditingPluginKey.getState(state)!.dragSelectionStarted).toBe(false);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: false }));
    expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual(duringPress);
    expect(state.doc).toBe(doc);
  });

  it('does not treat a preexisting selection as a new drag when clicking another format', () => {
    const text = '**one** gap **two**';
    let state = createState([text]);
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 3, 5)));
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: true }));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, text.indexOf('two') + 2)),
    );
    expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual([
      [13, 15],
      [18, 20],
    ]);
    expect(inlineSourceEditingPluginKey.getState(state)!.dragSelectionStarted).toBe(false);
  });

  it('uses only resting decorations in readonly mode, including when the option changes', () => {
    let readonly = false;
    const state = createState(['**one**'], { readonly: () => readonly });
    const view = createView(state);
    expect(view.dom.querySelectorAll('.pm-inline-source-marker.is-hidden')).toHaveLength(0);
    readonly = true;
    view.updateState(view.state);
    expect(view.dom.querySelectorAll('.pm-inline-source-marker.is-hidden')).toHaveLength(2);
    expect(view.dom.querySelector('[contenteditable="false"]')).toBeNull();
    expect(view.dom.textContent).toBe('**one**');
  });

  it('maps existing decorations while composing and rebuilds after committed text', () => {
    let state = createState(['**one**']);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { composing: true }));
    expect(isInlineSourceComposing(state)).toBe(true);
    state = apply(state, state.tr.delete(6, 8));
    expect(
      inlineSourceEditingPluginKey
        .getState(state)!
        .decorations.find()
        .some((decoration) => decoration.spec.format === 'strong'),
    ).toBe(true);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { composing: false }));
    expect(isInlineSourceComposing(state)).toBe(false);
    expect(inlineSourceEditingPluginKey.getState(state)!.decorations.find()).toHaveLength(0);
    expect(state.doc.textContent).toBe('**one');
  });

  it('freezes local visibility during composition and switches it after composition ends', () => {
    const text = '**one** gap **two**';
    let state = createState([text]);
    const initial = visibleMarkers(state).map(({ from, to }) => [from, to]);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { composing: true }));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, text.indexOf('two') + 2)),
    );
    expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual(initial);
    state = apply(state, state.tr.setMeta(inlineSourceEditingPluginKey, { composing: false }));
    expect(visibleMarkers(state).map(({ from, to }) => [from, to])).toEqual([
      [13, 15],
      [18, 20],
    ]);
  });

  it('tags new input with inline_source without converting or removing delimiters', () => {
    let state = createState(['']);
    state = apply(state, state.tr.insertText('**new**'));
    expect(state.doc.textContent).toBe('**new**');
    expect(state.doc.firstChild!.firstChild!.marks.map((mark) => mark.type.name)).toEqual([
      'inline_source',
    ]);
    expect(markers(state)).toHaveLength(2);
  });

  it('reveals escapes near their characters and code padding only with its active code span', () => {
    let state = createState(['active', '\\*literal\\* and `` `code` ``']);
    const inactiveMarkers = hiddenMarkers(state).map((decoration) =>
      state.doc.textBetween(decoration.from, decoration.to),
    );
    expect(inactiveMarkers.filter((text) => text === '\\')).toHaveLength(2);
    expect(inactiveMarkers.filter((text) => text === '`` ' || text === ' ``')).toHaveLength(2);
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(
      visibleMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['\\']);
    state = apply(
      state,
      state.tr.setSelection(
        TextSelection.create(
          state.doc,
          blockStart(state, 1) + state.doc.lastChild!.textContent.indexOf('code'),
        ),
      ),
    );
    expect(
      visibleMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['`` ', ' ``']);
    expect(
      hiddenMarkers(state).map((range) => state.doc.textBetween(range.from, range.to)),
    ).toEqual(['\\', '\\']);
    expect(state.doc.lastChild!.textContent).toBe('\\*literal\\* and `` `code` ``');
  });

  it('does not misrender decoded entities whose displayed glyph is absent from source', () => {
    const state = createState(['active', '&lt; &amp; &#20013;']);
    const hidden = hiddenMarkers(state).map((decoration) =>
      state.doc.textBetween(decoration.from, decoration.to),
    );
    expect(hidden).toEqual(['amp;']);
  });

  it('does not analyze or source-tag code and HTML blocks', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.code_block.create(null, schema.text('**code**')),
      schema.nodes.html_block.create(null, schema.text('**html**')),
    ]);
    let state = EditorState.create({ doc, plugins: [inlineSourceEditingPlugin()] });
    state = apply(state, state.tr.insertText('x', 2));
    expect(inlineSourceEditingPluginKey.getState(state)!.decorations.find()).toHaveLength(0);
    expect(state.doc.firstChild!.firstChild!.marks).toEqual([]);
    expect(state.doc.lastChild!.firstChild!.marks).toEqual([]);
  });

  it('reuses block analyses on selection changes and only analyzes a changed block', () => {
    const analyze = vi.spyOn(codec, 'analyzeInlineSource');
    let state = createState(['**one**', '**two**']);
    expect(analyze).toHaveBeenCalledTimes(2);
    analyze.mockClear();
    state = apply(state, state.tr.setSelection(TextSelection.create(state.doc, 4)));
    expect(analyze).not.toHaveBeenCalled();
    state = apply(state, state.tr.insertText('x'));
    expect(analyze).toHaveBeenCalledTimes(1);
  });

  it('drops a never-edited empty command pair after leaving it', () => {
    let state = addTemplate(createState(['', 'tail']));
    expect(state.doc.firstChild!.textContent).toBe('****');
    expect(getEmptyInlineTemplateFormats(state)).toEqual(new Set(['strong']));
    expect(findEmptyInlineSourceTemplate(state, 'strong')).toMatchObject({
      from: 1,
      to: 5,
      cursor: 3,
    });
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(state.doc.firstChild!.textContent).toBe('');
    expect(state.doc.lastChild!.textContent).toBe('tail');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
  });

  it('preserves text and delimiters once a template has been edited, even if it becomes empty again', () => {
    let state = addTemplate(createState(['', 'tail']));
    state = apply(state, state.tr.insertText('A'));
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
    state = apply(state, state.tr.delete(3, 4));
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(state.doc.firstChild!.textContent).toBe('****');
  });

  it('cleans nested untouched templates together, with each pair owning real delimiters', () => {
    let state = addTemplate(createState(['', 'tail']));
    state = addTemplate(state, 'em', '*', '*');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(2);
    expect(getEmptyInlineTemplateFormats(state)).toEqual(new Set(['strong', 'em']));
    expect(markers(state)).toHaveLength(1);
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(state.doc.firstChild!.textContent).toBe('');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
  });

  it('maps an untouched template through changes outside its range', () => {
    let state = addTemplate(createState(['', 'tail']));
    state = apply(state, state.tr.insertText('before', 1));
    expect(findEmptyInlineSourceTemplate(state)).toMatchObject({ from: 7, to: 11, cursor: 9 });
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(state.doc.firstChild!.textContent).toBe('before');
  });

  it('flushes untouched templates only on an explicit call and waits for composition', () => {
    const view = createView(addTemplate(createState([''])));
    expect(getEmptyInlineSourceTemplates(view.state)).toHaveLength(1);
    view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { composing: true }));
    expect(clearEmptyInlineTemplates(view)).toBe(false);
    expect(view.state.doc.textContent).toBe('****');
    view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { composing: false }));
    expect(clearEmptyInlineTemplates(view)).toBe(true);
    expect(view.state.doc.textContent).toBe('');
    expect(clearEmptyInlineTemplates(view)).toBe(false);
  });

  it('restores command provenance through undo/redo and clears the unused pair on save', () => {
    let state = addTemplate(createState(['']));
    expect(undoDepth(state)).toBe(1);
    state = applyHistory(state, undo);
    expect(state.doc.textContent).toBe('');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
    expect(undoDepth(state)).toBe(0);
    state = applyHistory(state, redo);
    expect(state.doc.textContent).toBe('****');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(1);
    const view = createView(state);
    expect(clearEmptyInlineTemplates(view)).toBe(true);
    expect(view.state.doc.textContent).toBe('');
  });

  it('preserves template provenance over repeated history cycles before leaving it', () => {
    let state = addTemplate(createState(['', 'tail']));
    for (let cycle = 0; cycle < 4; cycle++) {
      state = applyHistory(state, undo);
      state = applyHistory(state, redo);
      expect(getEmptyInlineSourceTemplates(state)).toHaveLength(1);
      expect(undoDepth(state)).toBe(1);
    }
    state = apply(
      state,
      state.tr.setSelection(TextSelection.create(state.doc, blockStart(state, 1))),
    );
    expect(state.doc.firstChild!.textContent).toBe('');
  });

  it('replays nested command templates without treating their inserted delimiters as user text', () => {
    let state = addTemplate(createState(['']));
    state = addTemplate(state, 'em', '*', '*');
    expect(undoDepth(state)).toBe(1);
    state = applyHistory(state, undo);
    state = applyHistory(state, redo);
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(2);
    const view = createView(state);
    expect(clearEmptyInlineTemplates(view)).toBe(true);
    expect(view.state.doc.textContent).toBe('');
  });

  it('does not reactivate a command template when its grouped redo also replays entered text', () => {
    let state = addTemplate(createState(['']));
    state = apply(state, state.tr.insertText('A'));
    expect(undoDepth(state)).toBe(1);
    state = applyHistory(state, undo);
    state = applyHistory(state, redo);
    expect(state.doc.textContent).toBe('**A**');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
  });

  it('maps command provenance through changes outside the undo history', () => {
    let state = addTemplate(createState(['']));
    state = apply(state, state.tr.insertText('prefix', 1).setMeta('addToHistory', false));
    state = applyHistory(state, undo);
    expect(state.doc.textContent).toBe('prefix');
    state = applyHistory(state, redo);
    expect(findEmptyInlineSourceTemplate(state)).toMatchObject({ from: 7, to: 11, cursor: 9 });
    const view = createView(state);
    expect(clearEmptyInlineTemplates(view)).toBe(true);
    expect(view.state.doc.textContent).toBe('prefix');
  });

  it('does not recover temporary status when entered content alone is undone', () => {
    let state = addTemplate(createState(['']));
    state = apply(state, closeHistory(state.tr.insertText('A')));
    expect(undoDepth(state)).toBe(2);
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
    state = applyHistory(state, undo);
    expect(state.doc.textContent).toBe('****');
    expect(getEmptyInlineSourceTemplates(state)).toHaveLength(0);
    const view = createView(state);
    expect(clearEmptyInlineTemplates(view)).toBe(false);
    expect(view.state.doc.textContent).toBe('****');
  });

  it('never infers template provenance from manually typed delimiters replayed by history', () => {
    let state = createState(['']);
    state = apply(state, state.tr.insertText('****'));
    state = applyHistory(state, undo);
    state = applyHistory(state, redo);
    const view = createView(state);
    expect(getEmptyInlineSourceTemplates(view.state)).toHaveLength(0);
    expect(clearEmptyInlineTemplates(view)).toBe(false);
    expect(view.state.doc.textContent).toBe('****');
  });
});
