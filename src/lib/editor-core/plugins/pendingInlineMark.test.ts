import { describe, expect, it } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import {
  isPendingMarkActive,
  pendingInlineMarkKey,
  pendingInlineMarkPlugin,
  toggleMarkPending,
} from './pendingInlineMark';
import { schema } from '../schema';

describe('pendingInlineMarkPlugin', () => {
  it('keeps pending delimiters anchored after the first typed character', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    let state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1),
      plugins: [pendingInlineMarkPlugin()],
    });

    toggleMarkPending(schema.marks.strong)(state, (tr) => {
      state = state.apply(tr);
    });
    state = state.apply(state.tr.insertText('A'));

    const pending = pendingInlineMarkKey.getState(state);
    const text = state.doc.firstChild?.firstChild;

    expect(isPendingMarkActive(state, schema.marks.strong)).toBe(true);
    expect(pending).toMatchObject({
      active: true,
      markTypeNames: ['strong'],
      anchorPos: 1,
      headPos: 2,
    });
    expect(text?.marks.some((mark) => mark.type === schema.marks.strong)).toBe(true);
  });

  it('keeps multiple pending inline marks active together', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    let state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1),
      plugins: [pendingInlineMarkPlugin()],
    });

    toggleMarkPending(schema.marks.strikethrough)(state, (tr) => {
      state = state.apply(tr);
    });
    toggleMarkPending(schema.marks.strong)(state, (tr) => {
      state = state.apply(tr);
    });
    state = state.apply(state.tr.insertText('A'));

    const text = state.doc.firstChild?.firstChild;
    const markNames = text?.marks.map((mark) => mark.type.name).sort();

    expect(isPendingMarkActive(state, schema.marks.strikethrough)).toBe(true);
    expect(isPendingMarkActive(state, schema.marks.strong)).toBe(true);
    expect(markNames).toEqual(['strikethrough', 'strong']);
  });

  it('adds a new pending mark without dropping the mark already being typed', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    let state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1),
      plugins: [pendingInlineMarkPlugin()],
    });

    toggleMarkPending(schema.marks.strikethrough)(state, (tr) => {
      state = state.apply(tr);
    });
    state = state.apply(state.tr.insertText('A'));
    toggleMarkPending(schema.marks.strong)(state, (tr) => {
      state = state.apply(tr);
    });
    state = state.apply(state.tr.insertText('B'));

    const first = state.doc.firstChild?.child(0);
    const second = state.doc.firstChild?.child(1);

    expect(first?.text).toBe('A');
    expect(first?.marks.map((mark) => mark.type.name)).toEqual(['strikethrough']);
    expect(second?.text).toBe('B');
    expect(second?.marks.map((mark) => mark.type.name).sort()).toEqual(['strikethrough', 'strong']);
  });

  it('does not put delimiter widgets on an empty pending range so IME can compose', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    toggleMarkPending(schema.marks.strikethrough)(view.state, view.dispatch);
    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);

    expect(isPendingMarkActive(view.state, schema.marks.strikethrough)).toBe(true);
    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(true);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    view.destroy();
    target.remove();
  });

  it('keeps the closing delimiter off the caret after the first pending character', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    view.dispatch(view.state.tr.insertText('测'));

    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(true);
    expect(hasDelimiterWidget(target, 'strong', 'open')).toBe(true);
    expect(hasDelimiterWidget(target, 'strong', 'close')).toBe(false);

    view.destroy();
    target.remove();
  });

  it('does not insert delimiter widgets while IME is composing a pending mark', () => {
    const plugin = pendingInlineMarkPlugin();
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1),
        plugins: [plugin],
      }),
    });

    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);
    plugin.props.handleDOMEvents?.compositionstart?.call(
      plugin,
      view,
      new CompositionEvent('compositionstart'),
    );
    view.dispatch(view.state.tr.insertText('ni'));

    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(true);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    plugin.props.handleDOMEvents?.compositionend?.call(
      plugin,
      view,
      new CompositionEvent('compositionend', { data: '你' }),
    );
    view.dispatch(view.state.tr.setMeta('nomoPendingMarkDecorationsRefresh', true));

    expect(hasDelimiterWidget(target, 'strong', 'open')).toBe(true);
    expect(hasDelimiterWidget(target, 'strong', 'close')).toBe(false);

    view.destroy();
    target.remove();
  });

  it('uses standard mark toggling when text is selected', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, schema.text('hello')),
    ]);
    let state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1, 6),
      plugins: [pendingInlineMarkPlugin()],
    });

    toggleMarkPending(schema.marks.em)(state, (tr) => {
      state = state.apply(tr);
    });

    const text = state.doc.firstChild?.firstChild;
    expect(isPendingMarkActive(state, schema.marks.em)).toBe(false);
    expect(text?.marks.some((mark) => mark.type === schema.marks.em)).toBe(true);
  });

  it('turns off an existing mark at its opening boundary instead of creating nested pending marks', () => {
    const { target, view } = createMarkedTextView(8);
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));

    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);

    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(false);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );
    expect(hasTextMark(view.state, 'strong')).toBe(false);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    view.destroy();
    target.remove();
  });

  it('turns off an existing mark in marked text instead of creating nested pending marks', () => {
    const { target, view } = createMarkedTextView(9);

    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);

    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(false);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );
    expect(hasTextMark(view.state, 'strong')).toBe(false);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    view.destroy();
    target.remove();
  });

  it('turns off an existing mark at its closing boundary instead of creating nested pending marks', () => {
    const { target, view } = createMarkedTextView(12);
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));

    toggleMarkPending(schema.marks.strong)(view.state, view.dispatch);

    expect(isPendingMarkActive(view.state, schema.marks.strong)).toBe(false);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );
    expect(hasTextMark(view.state, 'strong')).toBe(false);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget')).toHaveLength(0);

    view.destroy();
    target.remove();
  });

  it('exits pending when the cursor moves to another block', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(),
      schema.nodes.paragraph.create(),
    ]);
    let state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1),
      plugins: [pendingInlineMarkPlugin()],
    });

    toggleMarkPending(schema.marks.underline)(state, (tr) => {
      state = state.apply(tr);
    });
    state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 3)));

    expect(isPendingMarkActive(state, schema.marks.underline)).toBe(false);
  });

  it('shows edit delimiters when the cursor enters an existing mark range', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('before '),
        schema.text('bold', [schema.marks.strong.create()]),
        schema.text(' after'),
      ]),
    ]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 9),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    // 验证加粗 mark 的语法提示存在
    expect(hasMarkDelimiter(target, 'strong')).toBe(true);

    view.destroy();
    target.remove();
  });

  it('uses mark tag delimiters for pending highlight input', () => {
    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    toggleMarkPending(schema.marks.highlight)(view.state, view.dispatch);
    view.dispatch(view.state.tr.insertText('A'));

    expect(isPendingMarkActive(view.state, schema.marks.highlight)).toBe(true);
    expect(hasTextMarkForText(view.state, 'A', 'highlight')).toBe(true);
    expect(hasDelimiterWidget(target, 'highlight', 'open')).toBe(true);
    expect(hasDelimiterWidget(target, 'highlight', 'close')).toBe(false);

    view.destroy();
    target.remove();
  });

  it('shows edit delimiters for every mark on the same text range', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('both', [schema.marks.strikethrough.create(), schema.marks.strong.create()]),
      ]),
    ]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 3),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    // 验证两种 mark 类型的语法提示都存在
    expect(hasMarkDelimiter(target, 'strong')).toBe(true);
    expect(hasMarkDelimiter(target, 'strikethrough')).toBe(true);

    view.destroy();
    target.remove();
  });

  it('treats both edges of an existing mark range as edit state', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('before '),
        schema.text('bold', [schema.marks.strong.create()]),
        schema.text(' after'),
      ]),
    ]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 8),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    expect(hasDelimiterWidget(target, 'strong', 'open')).toBe(false);
    expect(hasDelimiterWidget(target, 'strong', 'close')).toBe(true);
    expect(
      target.querySelector('.pm-mark-delimiter-range')?.getAttribute('data-caret-open'),
    ).toBe('true');

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 12)));

    expect(hasDelimiterWidget(target, 'strong', 'open')).toBe(true);
    expect(hasDelimiterWidget(target, 'strong', 'close')).toBe(false);
    expect(
      target.querySelector('.pm-mark-delimiter-range')?.getAttribute('data-caret-close'),
    ).toBe('true');

    view.destroy();
    target.remove();
  });

  it('does not put a closing delimiter widget at the caret after bold text', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('ds', [schema.marks.strong.create()]),
        schema.text('的'),
      ]),
    ]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 3),
        plugins: [pendingInlineMarkPlugin()],
      }),
    });

    expect(view.state.selection.from).toBe(3);
    expect(hasDelimiterWidget(target, 'strong', 'open')).toBe(true);
    expect(hasDelimiterWidget(target, 'strong', 'close')).toBe(false);
    expect(target.querySelectorAll('.pm-mark-delimiter-widget[data-edge="close"]')).toHaveLength(0);
    expect(
      target.querySelector('.pm-mark-delimiter-range')?.getAttribute('data-caret-close'),
    ).toBe('true');

    view.destroy();
    target.remove();
  });

  it('places the cursor outside the mark when clicking before the opening edit delimiter midpoint', () => {
    const { target, view } = createMarkedTextView(9);
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 100, right: 116 });

    openWidget.dispatchEvent(createClick(104, 10));

    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );

    view.destroy();
    target.remove();
  });

  it('places the cursor inside the mark when clicking after the opening edit delimiter midpoint', () => {
    const { target, view } = createMarkedTextView(9);
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 100, right: 116 });

    openWidget.dispatchEvent(createClick(112, 10));

    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps the cursor inside the mark when clicking just after the opening edit delimiter', () => {
    const { target, view } = createMarkedTextView(9);
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 100, right: 116 });

    openWidget.dispatchEvent(createClick(116, 10));

    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('places the cursor inside the mark when clicking before the closing edit delimiter midpoint', () => {
    const { target, view } = createMarkedTextView(9);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 164, right: 180 });

    closeWidget.dispatchEvent(createClick(168, 10));

    expect(view.state.selection.from).toBe(12);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps the cursor inside the mark when clicking just before the closing edit delimiter', () => {
    const { target, view } = createMarkedTextView(9);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 164, right: 180 });

    closeWidget.dispatchEvent(createClick(164, 10));

    expect(view.state.selection.from).toBe(12);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('places the cursor outside the mark when clicking after the closing edit delimiter midpoint', () => {
    const { target, view } = createMarkedTextView(9);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 164, right: 180 });

    closeWidget.dispatchEvent(createClick(176, 10));

    expect(view.state.selection.from).toBe(12);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );

    view.destroy();
    target.remove();
  });

  it('keeps the cursor outside a code mark when clicking just after the closing backtick', () => {
    const { target, view } = createCodeTextView(2);
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.code.create()]));
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 130, right: 138 });

    const event = createClick(141, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.code)).not.toBe(true);

    view.destroy();
    target.remove();
  });

  it('locks the cursor outside a code mark when already at the closing boundary', () => {
    const { target, view } = createCodeTextView(4);

    expect(view.state.storedMarks).toBeNull();
    expect(hasDelimiterWidget(target, 'code', 'close')).toBe(false);
    expect(hasDelimiterWidget(target, 'code', 'open')).toBe(true);

    view.destroy();
    target.remove();
  });

  it('moves outside the opening backtick when clicking its outer side from inside code text', () => {
    const { target, view } = createCodeTextView(2);
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });

    openWidget.dispatchEvent(createClick(90, 10));

    expect(view.state.selection.from).toBe(1);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('moves outside the closing backtick when clicking its outer side from inside code text', () => {
    const { target, view } = createCodeTextView(2);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 130, right: 138 });

    closeWidget.dispatchEvent(createClick(138, 10));

    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('handles a closing backtick edge click even when the paragraph receives the event target', () => {
    const { target, view } = createCodeTextView(2);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 130, right: 138 });

    const event = createClick(137, 10);
    view.dom.querySelector('p')?.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('moves outside the closing backtick when clicking far outside from inside code text', () => {
    const { target, view } = createCodeTextView(2);
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });
    mockRangeRect(closeWidget, { left: 130, right: 138 });
    view.posAtCoords = () => ({ pos: 4, inside: -1 });

    const event = createClick(220, 10);
    view.dom.querySelector('p')?.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('moves outside the opening backtick when clicking far outside from inside code text', () => {
    const { target, view } = createCodeTextView(2);
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });
    mockRangeRect(closeWidget, { left: 130, right: 138 });
    view.posAtCoords = () => ({ pos: 1, inside: -1 });

    const event = createClick(20, 10);
    view.dom.querySelector('p')?.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(1);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('moves outside the opening backtick when clicking far before a code mark at textblock start', () => {
    const { target, view } = createCodeTextView(2);
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });
    mockRangeRect(closeWidget, { left: 130, right: 138 });
    view.posAtCoords = () => null;

    const event = createClick(20, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(1);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('does not intercept far-left clicks when ordinary text exists before the code mark', () => {
    const { target, view } = createPrefixedCodeTextView(4);
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 110, right: 118 });
    mockRangeRect(closeWidget, { left: 150, right: 158 });
    view.posAtCoords = () => null;

    const event = createClick(20, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
    expect(view.state.selection.from).toBe(4);

    view.destroy();
    target.remove();
  });

  it('keeps the cursor inside a code mark when clicking just before the closing backtick', () => {
    const { target, view } = createCodeTextView(2);
    view.dispatch(view.state.tr.setStoredMarks([]));
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 130, right: 138 });

    const event = createClick(127, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.code)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('moves inside the code mark when clicking the last character from the closing outside boundary', () => {
    const { target, view } = createCodeTextView(2);
    view.dispatch(view.state.tr.setStoredMarks([]));
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });
    mockRangeRect(closeWidget, { left: 130, right: 138 });
    view.posAtCoords = () => ({ pos: 4, inside: -1 });

    const event = createClick(126, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.code)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('moves inside the code mark when clicking the first character from the opening outside boundary', () => {
    const { target, view } = createCodeTextView(2);
    view.dispatch(view.state.tr.setStoredMarks([]));
    const closeWidget = getDelimiterWidget(target, 'close');
    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 90, right: 98 });
    mockRangeRect(closeWidget, { left: 130, right: 138 });
    view.posAtCoords = () => ({ pos: 1, inside: -1 });

    const event = createClick(102, 10);
    view.dom.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.selection.from).toBe(1);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.code)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('does not intercept mousedown near a code delimiter so drag selection can start', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createCodeTextView(2, [plugin]);
    const closeWidget = getDelimiterWidget(target, 'close');
    mockRangeRect(closeWidget, { left: 130, right: 138 });

    const event = createMouseDown(141, 10);
    const handled = plugin.props.handleDOMEvents?.mousedown?.call(plugin, view, event) ?? false;

    expect(handled).toBe(false);
    expect(event.defaultPrevented).toBe(false);
    expect(view.state.selection.from).toBe(2);

    view.destroy();
    target.remove();
  });

  it('does not use editor root coordinates as a fallback for delimiter clicks', () => {
    const { target, view } = createMarkedTextView(9);
    const plugin = pendingInlineMarkPlugin();

    const handled = plugin.props.handleDOMEvents?.mousedown?.call(
      plugin,
      view,
      createMouseDown(184, 10),
    );

    expect(handled).toBe(false);
    expect(view.state.selection.from).toBe(9);

    view.destroy();
    target.remove();
  });

  it('does not intercept clicks in the real marked text area', () => {
    const { target, view } = createMarkedTextView(9);
    const plugin = pendingInlineMarkPlugin();

    const handled = plugin.props.handleDOMEvents?.mousedown?.call(
      plugin,
      view,
      createMouseDown(140, 10),
    );

    expect(handled).toBe(false);
    expect(view.state.selection.from).toBe(9);

    view.destroy();
    target.remove();
  });

  it('does not snap to the opening delimiter after an interior click makes widgets appear', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createMarkedTextView(1, [plugin]);
    view.posAtCoords = () => ({ pos: 10, inside: 1 });

    plugin.props.handleDOMEvents?.mousedown?.call(plugin, view, createMouseDown(140, 10));
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 10)));

    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 130, right: 150 });

    const handled = plugin.props.handleDOMEvents?.click?.call(plugin, view, createClick(140, 10));

    expect(handled).toBe(false);
    expect(view.state.selection.from).toBe(10);

    view.destroy();
    target.remove();
  });

  it('does not snap to a highlight tag after clicking inside the highlighted text', () => {
    const plugin = pendingInlineMarkPlugin();
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('外标记, '),
        schema.text('高亮', [schema.marks.highlight.create()]),
        schema.text(' 表'),
      ]),
    ]);
    const target = document.createElement('div');
    document.body.appendChild(target);
    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1),
        plugins: [plugin],
      }),
    });
    view.posAtCoords = () => ({ pos: 7, inside: 1 });

    plugin.props.handleDOMEvents?.mousedown?.call(plugin, view, createMouseDown(220, 10));
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7)));

    const openWidget = getDelimiterWidget(target, 'open');
    mockRangeRect(openWidget, { left: 180, right: 240 });

    const handled = plugin.props.handleDOMEvents?.click?.call(plugin, view, createClick(220, 10));

    expect(handled).toBe(false);
    expect(view.state.selection.from).toBe(7);

    view.destroy();
    target.remove();
  });

  it('keeps right-arrow navigation at the opening delimiter before entering the mark', () => {
    const { target, view } = createMarkedTextView(8);
    const plugin = pendingInlineMarkPlugin();

    const handled = plugin.props.handleKeyDown?.call(plugin, view, createKeyDown('ArrowRight'));

    expect(handled).toBe(true);
    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps left-arrow navigation at the opening delimiter before leaving the mark', () => {
    const { target, view } = createMarkedTextView(8);
    const plugin = pendingInlineMarkPlugin();
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));

    const handled = plugin.props.handleKeyDown?.call(plugin, view, createKeyDown('ArrowLeft'));

    expect(handled).toBe(true);
    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );

    view.destroy();
    target.remove();
  });

  it('keeps right-arrow navigation at the closing delimiter before leaving the mark', () => {
    const { target, view } = createMarkedTextView(12);
    const plugin = pendingInlineMarkPlugin();
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));

    const handled = plugin.props.handleKeyDown?.call(plugin, view, createKeyDown('ArrowRight'));

    expect(handled).toBe(true);
    expect(view.state.selection.from).toBe(12);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );

    view.destroy();
    target.remove();
  });

  it('keeps left-arrow navigation at the closing delimiter before entering the mark', () => {
    const { target, view } = createMarkedTextView(12);
    const plugin = pendingInlineMarkPlugin();

    const handled = plugin.props.handleKeyDown?.call(plugin, view, createKeyDown('ArrowLeft'));

    expect(handled).toBe(true);
    expect(view.state.selection.from).toBe(12);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps the cursor inside the mark when pointer-clicking before the first marked character', () => {
    const { target, view } = createMarkedTextView(9);

    // 模拟浏览器处理正文点击：点在第一个加粗字符前时，ProseMirror 会把 selection 放到 from=8，
    // 并给事务标记 pointer。这个路径不会经过灰色 delimiter widget 的 mousedown。
    view.dispatch(
      view.state.tr
        .setSelection(TextSelection.create(view.state.doc, 8))
        .setMeta('pointer', true),
    );

    expect(view.state.selection.from).toBe(8);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps typed text inside the mark at the closing delimiter', () => {
    const { target, view } = createMarkedTextView(12);
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));

    view.dispatch(view.state.tr.insertText('X'));

    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('before boldX after');
    expect(hasTextMarkForText(view.state, 'X', 'strong')).toBe(true);
    expect(view.state.selection.from).toBe(13);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('keeps typed text outside the mark after the closing delimiter', () => {
    const { target, view } = createMarkedTextView(12);
    const plugin = pendingInlineMarkPlugin();

    const handled = plugin.props.handleTextInput?.call(
      plugin,
      view,
      12,
      12,
      'X',
      () => view.state.tr,
    );

    expect(handled).toBe(true);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('before boldX after');
    expect(hasTextMarkForText(view.state, 'X', 'strong')).toBe(false);
    expect(view.state.selection.from).toBe(13);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
      true,
    );

    view.destroy();
    target.remove();
  });

  it('handles beforeinput at the outer closing delimiter before native DOM insertion', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createMarkedTextView(12, [plugin]);
    const event = createBeforeInput('.');

    const handled = plugin.props.handleDOMEvents?.beforeinput?.call(plugin, view, event) ?? false;

    expect(handled).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('before bold. after');
    expect(hasTextMarkForText(view.state, '.', 'strong')).toBe(false);
    expect(view.state.selection.from).toBe(13);
    expect(view.state.storedMarks).toEqual([]);

    view.destroy();
    target.remove();
  });

  it('deduplicates handleTextInput after beforeinput handles the same boundary symbol', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createMarkedTextView(12, [plugin]);

    const beforeInputHandled =
      plugin.props.handleDOMEvents?.beforeinput?.call(plugin, view, createBeforeInput('.')) ?? false;
    const textInputHandled =
      plugin.props.handleTextInput?.call(
        plugin,
        view,
        12,
        12,
        '.',
        () => view.state.tr,
      ) ?? false;

    expect(beforeInputHandled).toBe(true);
    expect(textInputHandled).toBe(true);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('before bold. after');
    expect(view.state.selection.from).toBe(13);

    view.destroy();
    target.remove();
  });

  it('deduplicates Chinese IME punctuation reported at the updated caret after a code boundary beforeinput', () => {
    for (const punctuation of ['、', '，']) {
      const plugin = pendingInlineMarkPlugin();
      const { target, view } = createCodeTextView(4, [plugin]);

      const beforeInputHandled =
        plugin.props.handleDOMEvents?.beforeinput?.call(
          plugin,
          view,
          createBeforeInput(punctuation),
        ) ?? false;
      const textInputHandled =
        plugin.props.handleTextInput?.call(
          plugin,
          view,
          5,
          5,
          punctuation,
          () => view.state.tr.insertText(punctuation, 5, 5),
        ) ?? false;

      expect(beforeInputHandled).toBe(true);
      expect(textInputHandled).toBe(true);
      expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe(
        `asd${punctuation}`,
      );
      expect(hasTextMarkForText(view.state, punctuation, 'code')).toBe(false);
      expect(view.state.selection.from).toBe(5);

      view.destroy();
      target.remove();
    }
  });

  it('keeps Chinese IME punctuation single and marked at the inner code closing delimiter', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createCodeTextView(4, [plugin]);
    view.dispatch(view.state.tr.setStoredMarks([schema.marks.code.create()]));

    const event = createBeforeInput('、', { isComposing: true });
    const beforeInputHandled =
      plugin.props.handleDOMEvents?.beforeinput?.call(plugin, view, event) ?? false;
    const textInputHandled =
      plugin.props.handleTextInput?.call(
        plugin,
        view,
        5,
        5,
        '、',
        () => view.state.tr.insertText('、', 5, 5),
      ) ?? false;

    expect(beforeInputHandled).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(textInputHandled).toBe(true);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('asd、');
    expect(hasTextMarkForText(view.state, '、', 'code')).toBe(true);
    expect(view.state.selection.from).toBe(5);
    expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.code)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('lets ProseMirror own composing text input when no boundary beforeinput was handled', () => {
    const plugin = pendingInlineMarkPlugin();
    const { target, view } = createCodeTextView(4, [plugin]);
    setViewComposing(view, true);

    const handled =
      plugin.props.handleTextInput?.call(
        plugin,
        view,
        4,
        4,
        '，',
        () => view.state.tr.insertText('，', 4, 4),
      ) ?? false;

    expect(handled).toBe(false);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('asd');

    setViewComposing(view, false);
    view.destroy();
    target.remove();
  });

  describe('boundary click mirrors arrow keys (current side aware)', () => {
    // 场景：光标已在 mark 内侧（storedMarks 带 strong），点击 close widget 右半（目标=外）。
    // 旧逻辑硬塞 strong，导致光标看起来没动/反向跳；新逻辑应清空 storedMarks 正常出到外侧。
    it('clears storedMarks when clicking the outer half of the close delimiter while inside the mark', () => {
      const { target, view } = createMarkedTextView(9);
      view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));
      const closeWidget = getDelimiterWidget(target, 'close');
      mockRangeRect(closeWidget, { left: 164, right: 180 });

      closeWidget.dispatchEvent(createClick(176, 10));

      expect(view.state.selection.from).toBe(12);
      expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
        true,
      );

      view.destroy();
      target.remove();
    });

    it('keeps storedMarks when clicking the inner half of the open delimiter while inside the mark', () => {
      const { target, view } = createMarkedTextView(12);
      view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));
      const openWidget = getDelimiterWidget(target, 'open');
      mockRangeRect(openWidget, { left: 100, right: 116 });

      openWidget.dispatchEvent(createClick(112, 10));

      expect(view.state.selection.from).toBe(8);
      expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

      view.destroy();
      target.remove();
    });

    it('enters the mark when clicking the inner half of the close delimiter while outside', () => {
      const { target, view } = createMarkedTextView(9);
      view.dispatch(view.state.tr.setStoredMarks([]));
      const closeWidget = getDelimiterWidget(target, 'close');
      mockRangeRect(closeWidget, { left: 164, right: 180 });

      closeWidget.dispatchEvent(createClick(168, 10));

      expect(view.state.selection.from).toBe(12);
      expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

      view.destroy();
      target.remove();
    });

    it('stays outside when clicking the outer half of the open delimiter while outside', () => {
      const { target, view } = createMarkedTextView(9);
      view.dispatch(view.state.tr.setStoredMarks([]));
      const openWidget = getDelimiterWidget(target, 'open');
      mockRangeRect(openWidget, { left: 100, right: 116 });

      openWidget.dispatchEvent(createClick(104, 10));

      expect(view.state.selection.from).toBe(8);
      expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).not.toBe(
        true,
      );

      view.destroy();
      target.remove();
    });

    // 场景：光标在 mark 内部（pos 9，storedMarks 带 strong），点 open widget 右半（目标=内）。
    // 此时 pos 9 不在边界（8 或 12），cursorOnBoundary=false → currentlyInside=false，
    // 走 fallback 物理半边硬编码：open 右半 = 内 → setStoredMarks(strong)。验证不回归。
    it('falls back to physical-half logic when cursor is inside the mark but not on its boundary', () => {
      const { target, view } = createMarkedTextView(9);
      view.dispatch(view.state.tr.setStoredMarks([schema.marks.strong.create()]));
      const openWidget = getDelimiterWidget(target, 'open');
      mockRangeRect(openWidget, { left: 100, right: 116 });

      openWidget.dispatchEvent(createClick(112, 10));

      expect(view.state.selection.from).toBe(8);
      expect(view.state.storedMarks?.some((mark) => mark.type === schema.marks.strong)).toBe(true);

      view.destroy();
      target.remove();
    });
  });
});

/**
 * 检查是否存在指定 mark 类型的语法提示装饰。
 * 支持两种装饰模式：
 * - Decoration.inline：检查 .pm-mark-delimiter-range 标记正文范围
 * - Decoration.widget：检查 .pm-mark-delimiter-widget 显示真实占位标签
 *
 * 使用 *= (contains) 选择器，因为多 mark 叠加时 data-open 会拼接（如 "**~~"）。
 */
function hasDelimiterWidget(
  target: HTMLElement,
  markTypeName: string,
  edge: 'open' | 'close',
): boolean {
  const syntax = MARK_SYNTAX_MAP[markTypeName];
  if (!syntax) return false;
  const expected = edge === 'open' ? syntax.open : syntax.close;

  return Array.from(target.querySelectorAll<HTMLElement>('.pm-mark-delimiter-widget')).some(
    (widget) =>
      (widget.textContent ?? '').includes(expected) &&
      widget.dataset.edge === edge &&
      widget.contentEditable === 'false',
  );
}

function hasMarkDelimiter(target: HTMLElement, markTypeName: string): boolean {
  return (
    hasDelimiterWidget(target, markTypeName, 'open') &&
    hasDelimiterWidget(target, markTypeName, 'close')
  );
}

// 测试用的 mark 语法映射
const MARK_SYNTAX_MAP: Record<string, { open: string; close: string }> = {
  strong: { open: '**', close: '**' },
  em: { open: '*', close: '*' },
  code: { open: '`', close: '`' },
  strikethrough: { open: '~~', close: '~~' },
  underline: { open: '<u>', close: '</u>' },
  highlight: { open: '<mark>', close: '</mark>' },
};

function createMarkedTextView(
  selectionPos: number,
  plugins = [pendingInlineMarkPlugin()],
): { target: HTMLElement; view: EditorView } {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, [
      schema.text('before '),
      schema.text('bold', [schema.marks.strong.create()]),
      schema.text(' after'),
    ]),
  ]);
  const target = document.createElement('div');
  document.body.appendChild(target);

  const view = new EditorView(target, {
    state: EditorState.create({
      doc,
      selection: TextSelection.create(doc, selectionPos),
      plugins,
    }),
  });

  return { target, view };
}

function createCodeTextView(
  selectionPos: number,
  plugins = [pendingInlineMarkPlugin()],
): { target: HTMLElement; view: EditorView } {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, [schema.text('asd', [schema.marks.code.create()])]),
  ]);
  const target = document.createElement('div');
  document.body.appendChild(target);

  const view = new EditorView(target, {
    state: EditorState.create({
      doc,
      selection: TextSelection.create(doc, selectionPos),
      plugins,
    }),
  });

  return { target, view };
}

function createPrefixedCodeTextView(selectionPos: number): { target: HTMLElement; view: EditorView } {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, [
      schema.text('x '),
      schema.text('asd', [schema.marks.code.create()]),
    ]),
  ]);
  const target = document.createElement('div');
  document.body.appendChild(target);

  const view = new EditorView(target, {
    state: EditorState.create({
      doc,
      selection: TextSelection.create(doc, selectionPos),
      plugins: [pendingInlineMarkPlugin()],
    }),
  });

  return { target, view };
}

function getDelimiterWidget(target: HTMLElement, edge: 'open' | 'close'): HTMLElement {
  const widget = target.querySelector<HTMLElement>(
    `.pm-mark-delimiter-widget[data-edge="${edge}"]`,
  );
  expect(widget).not.toBeNull();
  return widget!;
}

function hasTextMark(state: EditorState, markTypeName: string): boolean {
  let found = false;
  state.doc.descendants((node) => {
    if (found || !node.isText) return false;
    found = node.marks.some((mark) => mark.type.name === markTypeName);
    return !found;
  });
  return found;
}

function hasTextMarkForText(state: EditorState, text: string, markTypeName: string): boolean {
  let found = false;
  state.doc.descendants((node) => {
    if (found || !node.isText || !node.text?.includes(text)) return !found;
    found = node.marks.some((mark) => mark.type.name === markTypeName);
    return !found;
  });
  return found;
}

function mockRangeRect(element: HTMLElement, rect: { left: number; right: number }): void {
  element.getBoundingClientRect = () =>
    ({
      left: rect.left,
      right: rect.right,
      top: 0,
      bottom: 20,
      width: rect.right - rect.left,
      height: 20,
      x: rect.left,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
}

function createMouseDown(clientX: number, clientY = 0): MouseEvent {
  return new MouseEvent('mousedown', {
    bubbles: true,
    cancelable: true,
    clientX,
    clientY,
  });
}

function createClick(clientX: number, clientY = 0): PointerEvent {
  // jsdom 25 lacks PointerEvent; keep a dispatchable mouse event with the full pointer contract.
  return Object.assign(
    new MouseEvent('click', {
      bubbles: true,
      cancelable: true,
      clientX,
      clientY,
    }),
    {
      pointerId: 1,
      pointerType: 'mouse',
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: 0,
      tangentialPressure: 0,
      tiltX: 0,
      tiltY: 0,
      twist: 0,
      altitudeAngle: Math.PI / 2,
      azimuthAngle: 0,
      getCoalescedEvents: () => [],
      getPredictedEvents: () => [],
    },
  );
}

function createKeyDown(key: 'ArrowLeft' | 'ArrowRight'): KeyboardEvent {
  return new KeyboardEvent('keydown', {
    bubbles: true,
    cancelable: true,
    key,
  });
}

function createBeforeInput(text: string, init: { isComposing?: boolean } = {}): InputEvent {
  const event = new InputEvent('beforeinput', {
    bubbles: true,
    cancelable: true,
    data: text,
    inputType: 'insertText',
  });
  if (init.isComposing !== undefined) {
    Object.defineProperty(event, 'isComposing', {
      configurable: true,
      value: init.isComposing,
    });
  }
  return event;
}

function setViewComposing(view: EditorView, composing: boolean): void {
  (view as unknown as { input: { composing: boolean } }).input.composing = composing;
}
