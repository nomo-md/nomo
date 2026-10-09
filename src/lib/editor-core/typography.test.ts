import { afterEach, expect, it, vi } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import { Decoration, DecorationSet, EditorView } from 'prosemirror-view';
import { createEditorCore } from './createEditorCore';
import { getTypographyOptions, setTypographyOptions } from '../typography/options';
import { cleanEditorArtifacts } from '../../app/services/exportService';
import type { EditorChangeEvent } from './types';
import { schema } from './schema';
import {
  inlineSourceEditingPlugin,
  inlineSourceEditingPluginKey,
} from './plugins/inlineSourceEditing';
import { flushEditorTypography, typographyPlugin } from './plugins/typography';
import * as measurement from '../typography/measure';
import { createParagraphInput } from '../typography/rules';
import { DEFAULT_TYPOGRAPHY, type LayoutItem } from '../typography/types';

let core: ReturnType<typeof createEditorCore> | undefined;
let standaloneView: EditorView | undefined;
const originalOptions = getTypographyOptions();
afterEach(() => {
  standaloneView?.destroy();
  standaloneView = undefined;
  core?.destroy();
  core = undefined;
  setTypographyOptions(originalOptions);
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

it('exports current source even when the semantic view has not caught up', () => {
  const host = document.createElement('div');
  document.body.append(host);
  core = createEditorCore({ target: host, markdown: 'Old ![old](old.png)' });
  core.updateOptions({ mode: 'source' });
  core.setMarkdown('Latest ![new](new.png)', { sourceInput: true });
  const html = core.getExportHtml!();
  expect(html).toContain('Latest');
  expect(html).toContain('new.png');
  expect(html).not.toContain('old.png');
});

it('typography toggles preserve exact Markdown, dirty state and semantic export', () => {
  const original = 'hello\nworld\n\n中文\n换行 **粗体** 和 $x^2$。\n';
  const host = document.createElement('div');
  document.body.append(host);
  core = createEditorCore({ target: host, markdown: original });
  const before = core.getSnapshot();
  const events: EditorChangeEvent[] = [];
  const unsubscribe = core.subscribe((event) => events.push(event));
  const previous = getTypographyOptions();
  setTypographyOptions({ enabled: !previous.enabled });
  setTypographyOptions(previous);
  expect(core.flushMarkdown()).toBe(original);
  expect(core.getSnapshot().version).toBe(before.version);
  expect(events.every((event) => event.dirty === false)).toBe(true);
  expect(core.execute({ type: 'undo' })).toBe(false);
  expect(core.flushMarkdown()).toBe(original);
  unsubscribe();
  const exported = cleanEditorArtifacts(core.getExportHtml!());
  const doc = new DOMParser().parseFromString(exported, 'text/html');
  expect(doc.querySelectorAll('br[data-nomo-break="soft"]')).toHaveLength(2);
  expect(doc.querySelector('strong')?.textContent).toBe('粗体');
  expect(doc.querySelector('.math-inline .katex')).not.toBeNull();
  expect(doc.querySelector('.kp-unit,[data-kp-owned]')).toBeNull();
});

function typographyView(texts: string[]) {
  const doc = schema.nodes.doc.create(
    null,
    texts.map((text) =>
      schema.nodes.paragraph.create(null, schema.text(text, [schema.marks.inline_source.create()])),
    ),
  );
  const host = document.body.appendChild(document.createElement('div'));
  standaloneView = new EditorView(host, {
    state: EditorState.create({ doc, plugins: [inlineSourceEditingPlugin(), typographyPlugin()] }),
  });
  return standaloneView;
}

function measureVisibleCharacters(
  element: HTMLElement,
  options: Parameters<typeof measurement.measureParagraph>[1],
  breakAfter?: string,
): measurement.MeasuredParagraph {
  const bindings: measurement.DomBinding[] = [];
  const units: LayoutItem[] = [];
  const forced = new Set<number>();
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  let from = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.parentElement?.closest('.pm-inline-source-marker.is-hidden,[data-kp-owned]')) continue;
    for (let offset = 0; offset < node.textContent!.length; offset++, from++) {
      const text = node.textContent![offset];
      bindings.push({
        node,
        offset,
        endOffset: offset + 1,
        from,
        to: from + 1,
        atomic: false,
        width: 8,
      });
      units.push({
        kind: 'box',
        text,
        from,
        to: from + 1,
        width: 8,
        fontSize: 16,
        ascent: 12,
        descent: 4,
      });
      if (text === breakAfter) forced.add(from + 1);
    }
  }
  return {
    bindings,
    text: units.map((unit) => unit.text).join(''),
    input: createParagraphInput(units, 240, 28, options, forced),
  };
}

it('keeps every real source character reachable with typography arrow navigation', () => {
  setTypographyOptions({ enabled: true });
  const text = '**粗体** `ab`';
  const view = typographyView([text]);
  view.dispatch(
    view.state.tr.setMeta(
      'nomo:typography',
      DecorationSet.create(view.state.doc, [
        Decoration.node(
          0,
          view.state.doc.firstChild!.nodeSize,
          {},
          { paragraph: view.state.doc.firstChild },
        ),
      ]),
    ),
  );
  for (let position = 2; position <= text.length + 1; position++) {
    const event = new KeyboardEvent('keydown', { key: 'ArrowRight' });
    expect(view.someProp('handleKeyDown', (handle) => handle(view, event))).toBe(true);
    expect(view.state.selection.from).toBe(position);
  }
  for (let position = text.length; position >= 1; position--) {
    const event = new KeyboardEvent('keydown', { key: 'ArrowLeft' });
    expect(view.someProp('handleKeyDown', (handle) => handle(view, event))).toBe(true);
    expect(view.state.selection.from).toBe(position);
  }
  expect(view.state.doc.textContent).toBe(text);
});

it('remeasures only paragraphs whose source visibility or code selection geometry changed', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  const measure = vi.fn(measureVisibleCharacters);
  vi.spyOn(measurement, 'measureParagraph').mockImplementation(async (element, options) =>
    measure(element, options),
  );
  vi.spyOn(measurement, 'measureParagraphSync').mockImplementation((element, options) =>
    measure(element, options),
  );
  const secondText = '`two` and **more**';
  const view = typographyView(['**one**', secondText, 'untouched']);
  await flushEditorTypography(view);
  expect(measure).toHaveBeenCalledTimes(3);
  measure.mockClear();
  const second = view.state.doc.firstChild!.nodeSize + 1;
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, second + 1)));
  await flushEditorTypography(view);
  expect(measure.mock.calls.map(([element]) => element.textContent)).toEqual([
    '**one**',
    secondText,
  ]);
  measure.mockClear();
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, second + 2)));
  await flushEditorTypography(view);
  expect(measure).not.toHaveBeenCalled();
  view.dispatch(
    view.state.tr.setSelection(TextSelection.create(view.state.doc, second + 1, second + 3)),
  );
  await flushEditorTypography(view);
  expect(measure).toHaveBeenCalledTimes(1);
  expect(measure.mock.calls[0][0].textContent).toBe(secondText);
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, second + 2)));
  await flushEditorTypography(view);
  measure.mockClear();
  const docBeforeMove = view.state.doc;
  const boldPosition = second + secondText.indexOf('more');
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, boldPosition)));
  await flushEditorTypography(view);
  expect(measure.mock.calls.map(([element]) => element.textContent)).toEqual([secondText]);
  expect(view.state.doc).toBe(docBeforeMove);
  measure.mockClear();
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, boldPosition + 1)));
  await flushEditorTypography(view);
  expect(measure).not.toHaveBeenCalled();
  expect(view.state.doc.textContent).toBe(`**one**${secondText}untouched`);
});

it('replaces source layouts without an intermediate native frame or stale break positions', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  const measure = vi.fn(
    (element: HTMLElement, options: Parameters<typeof measurement.measureParagraph>[1]) =>
      measureVisibleCharacters(element, options, '粗'),
  );
  vi.spyOn(measurement, 'measureParagraph').mockImplementation(async (element, options) =>
    measure(element, options),
  );
  vi.spyOn(measurement, 'measureParagraphSync').mockImplementation((element, options) =>
    measure(element, options),
  );
  const text = 'S01 普通开头 —— **第一处加粗** 中段 __第二处__ 结尾';
  const view = typographyView([text]);
  const doc = view.state.doc;
  await flushEditorTypography(view);
  const paragraph = view.dom.querySelector('p')!;
  const expectedBreak = text.indexOf('粗') + 2;
  for (const position of [
    text.indexOf('第一') + 1,
    2,
    text.indexOf('第二') + 1,
    2,
    text.indexOf('第一') + 1,
  ]) {
    expect(paragraph.dataset.kpLayout).toBe('ready');
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, position)));
    const flushing = flushEditorTypography(view);
    await Promise.resolve();
    expect(paragraph.dataset.kpLayout).toBe('ready');
    expect(paragraph.querySelector('.kp-unit')).not.toBeNull();
    await flushing;
    expect(paragraph.dataset.kpLayout).toBe('ready');
    expect([...paragraph.querySelectorAll('.kp-break')].map((br) => view.posAtDOM(br, 0))).toEqual([
      expectedBreak,
    ]);
    expect(paragraph.querySelector('.kp-unit .kp-unit')).toBeNull();
    expect(view.state.doc).toBe(doc);
  }
  expect(measure).toHaveBeenCalledTimes(6);
  // Input preparation cancels stale work without exposing native spacing.
  const beforeInput = paragraph.innerHTML;
  view.dom.dispatchEvent(new Event('beforeinput', { bubbles: true }));
  expect(paragraph.dataset.kpLayout).toBe('ready');
  expect(paragraph.innerHTML).toBe(beforeInput);
  view.dispatch(view.state.tr.insertText('追加', text.length + 1));
  await Promise.resolve();
  expect(paragraph.dataset.kpLayout).toBe('ready');
  expect(view.state.doc.textContent).toBe(text + '追加');
});

it.each(['detached', 'outside-paragraph', 'obsolete-offset'] as const)(
  'falls back without publishing invalid DOM binding coordinates: %s',
  async (corruption) => {
    setTypographyOptions({ enabled: true });
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
    vi.spyOn(measurement, 'measureParagraph').mockImplementation(async (element, options) => {
      const measured = measureVisibleCharacters(element, options);
      if (element.textContent === 'first') {
        if (corruption === 'obsolete-offset') measured.bindings[0].endOffset = 100;
        else if (corruption === 'outside-paragraph')
          measured.bindings[0].node =
            element.nextElementSibling!.querySelector('[data-inline-source]')!.firstChild!;
        else measured.bindings[0].node = document.createTextNode('f');
      }
      return measured;
    });
    const view = typographyView(['first', 'second']);
    await flushEditorTypography(view);
    const paragraph = view.dom.querySelector('p')!;
    expect(paragraph.dataset.kpLayout).toBe('fallback');
    expect(paragraph.dataset.kpReason).toBe('stale-dom-bindings');
    expect(paragraph.querySelector('.kp-unit,.kp-break')).toBeNull();
    expect(view.state.doc.textContent).toBe('firstsecond');
  },
);

it('finishes a clicked caret reveal before paint even while pointerdown is held, without a timer rerender', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  const asyncMeasure = vi
    .spyOn(measurement, 'measureParagraph')
    .mockImplementation(async (element, options) => measureVisibleCharacters(element, options));
  const syncMeasure = vi
    .spyOn(measurement, 'measureParagraphSync')
    .mockImplementation((element, options) => measureVisibleCharacters(element, options));
  const text = 'start **bold** tail';
  const view = typographyView([text, 'untouched']);
  await flushEditorTypography(view);
  asyncMeasure.mockClear();
  // A pre-existing selection must not make the next plain click a drag.
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1, 4)));
  await Promise.resolve();
  view.dom.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: true }));
  view.dispatch(
    view.state.tr.setSelection(TextSelection.create(view.state.doc, text.indexOf('bold') + 1)),
  );
  await Promise.resolve();
  expect(syncMeasure).toHaveBeenCalledTimes(1);
  expect(view.dom.querySelector('p')!.dataset.kpLayout).toBe('ready');
  const rendered = view.dom.innerHTML;
  view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { dragging: false }));
  window.dispatchEvent(new Event('pointerup'));
  await new Promise((resolve) => setTimeout(resolve, 180));
  await flushEditorTypography(view);
  expect(syncMeasure).toHaveBeenCalledTimes(1);
  expect(asyncMeasure).not.toHaveBeenCalled();
  expect(view.dom.innerHTML).toBe(rendered);
});

it('freezes the fast source layout during a real selection drag and source composition', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  vi.spyOn(measurement, 'measureParagraph').mockImplementation(async (element, options) =>
    measureVisibleCharacters(element, options),
  );
  const syncMeasure = vi
    .spyOn(measurement, 'measureParagraphSync')
    .mockImplementation((element, options) => measureVisibleCharacters(element, options));
  const view = typographyView(['start **bold** tail']);
  await flushEditorTypography(view);
  view.dom.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 9, 11)));
  await Promise.resolve();
  expect(syncMeasure).not.toHaveBeenCalled();
  window.dispatchEvent(new Event('pointerup'));
  await Promise.resolve();
  expect(syncMeasure).toHaveBeenCalledTimes(1);
  syncMeasure.mockClear();
  view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { composing: true }));
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
  await Promise.resolve();
  expect(syncMeasure).not.toHaveBeenCalled();
  view.dispatch(view.state.tr.setMeta(inlineSourceEditingPluginKey, { composing: false }));
  await Promise.resolve();
  expect(syncMeasure).toHaveBeenCalledTimes(1);
});

it('caches a budget fallback for its visibility signature and retries after a document change', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  const asyncMeasure = vi
    .spyOn(measurement, 'measureParagraph')
    .mockImplementation(async (element, options) => measureVisibleCharacters(element, options));
  const syncMeasure = vi.spyOn(measurement, 'measureParagraphSync').mockReturnValue(null);
  const view = typographyView(['start **bold** tail', 'untouched']);
  await flushEditorTypography(view);
  asyncMeasure.mockClear();
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 10)));
  await Promise.resolve();
  const paragraph = view.dom.querySelector('p')!;
  expect(paragraph.dataset.kpLayout).toBe('fallback');
  expect(paragraph.dataset.kpReason).toBe('source-visibility-fallback');
  const fallback = view.dom.innerHTML;
  await new Promise((resolve) => setTimeout(resolve, 180));
  await flushEditorTypography(view);
  expect(syncMeasure).toHaveBeenCalledTimes(1);
  expect(asyncMeasure).not.toHaveBeenCalled();
  expect(view.dom.innerHTML).toBe(fallback);
  view.dispatch(view.state.tr.insertText('x'));
  await flushEditorTypography(view);
  expect(asyncMeasure.mock.calls.map(([element]) => element.textContent)).toEqual([
    'start **bxold** tail',
  ]);
  expect(paragraph.dataset.kpLayout).toBe('ready');
});

it('does not mark pending work in another paragraph complete when the local fast layout publishes', async () => {
  setTypographyOptions({ enabled: true });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(240);
  const asyncMeasure = vi
    .spyOn(measurement, 'measureParagraph')
    .mockImplementation(async (element, options) => measureVisibleCharacters(element, options));
  const syncMeasure = vi
    .spyOn(measurement, 'measureParagraphSync')
    .mockImplementation((element, options) =>
      element.textContent === 'xother' ? null : measureVisibleCharacters(element, options),
    );
  const view = typographyView(['start **bold** tail', 'other']);
  await flushEditorTypography(view);
  asyncMeasure.mockClear();
  const secondStart = view.state.doc.firstChild!.nodeSize + 1;
  view.dispatch(view.state.tr.insertText('x', secondStart));
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 10)));
  await Promise.resolve();
  expect(syncMeasure).toHaveBeenCalledTimes(2);
  expect(view.dom.querySelectorAll('p')[1].dataset.kpReason).toBe('editing-budget-fallback');
  expect(asyncMeasure).not.toHaveBeenCalled();
  await flushEditorTypography(view);
  expect(asyncMeasure.mock.calls.map(([element]) => element.textContent)).toEqual(['xother']);
});

it('measures a source code span as a layout box while retaining all source offsets', async () => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
    measureText: (text: string) => ({
      width: text.length * 8,
      actualBoundingBoxAscent: 12,
      actualBoundingBoxDescent: 4,
    }),
  } as CanvasRenderingContext2D);
  vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(function (this: Range) {
    return new DOMRect(0, 0, (this.endOffset - this.startOffset) * 8, 16);
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement,
  ) {
    const style = getComputedStyle(this);
    const padding = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
    const border =
      (parseFloat(style.borderLeftWidth) || 0) + (parseFloat(style.borderRightWidth) || 0);
    return new DOMRect(0, 0, (this.textContent?.length ?? 0) * 8 + padding + border, 16);
  });
  const paragraph = document.body.appendChild(document.createElement('p'));
  paragraph.style.cssText =
    'writing-mode:horizontal-tb;text-align:left;font-size:16px;line-height:28px;';
  Object.defineProperty(paragraph, 'clientWidth', { value: 240 });
  paragraph.innerHTML =
    '<span class="pm-inline-source-code" style="padding:2px 4px;border:1px solid;font-size:14px">' +
    '<span class="pm-inline-source-marker is-hidden" style="display:none"> </span>code' +
    '<span class="pm-inline-source-marker is-hidden" style="display:none"> </span></span>';
  const measured = await measurement.measureParagraph(paragraph, DEFAULT_TYPOGRAPHY);
  expect(measured?.bindings).toHaveLength(1);
  expect(measured?.bindings[0]).toMatchObject({
    atomic: true,
    from: 0,
    to: 6,
    sourceSize: 6,
    width: 42,
  });
  expect(paragraph.textContent).toBe(' code ');
});
