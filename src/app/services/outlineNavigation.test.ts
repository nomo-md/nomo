import { afterEach, describe, expect, it, vi } from 'vitest';
import type { OutlineItem } from '../../lib/outline/outlineService';
import type { MarkdownSourceEditorHandle } from '../components/markdownSourceEditor';
import {
  getActiveOutlineIdFromSemantic,
  getSemanticScrollAnchor,
  getSemanticScrollAnchorForBlock,
  getSourceScrollAnchor,
  getSourceScrollAnchorAtLine,
  restoreSemanticReadingPosition,
  restoreSourceReadingPosition,
  scrollSemanticToAnchor,
  scrollSourceToAnchor,
} from './outlineNavigation';

describe('outlineNavigation', () => {
  const originalRequestAnimationFrame = window.requestAnimationFrame;
  const originalCancelAnimationFrame = window.cancelAnimationFrame;

  afterEach(() => {
    window.requestAnimationFrame = originalRequestAnimationFrame;
    window.cancelAnimationFrame = originalCancelAnimationFrame;
    vi.restoreAllMocks();
  });

  it('matches semantic headings by generated id without native instant scrolling', () => {
    const semanticPane = document.createElement('section');
    semanticPane.className = 'semantic-pane';
    Object.defineProperty(semanticPane, 'scrollHeight', { value: 800, configurable: true });
    semanticPane.getBoundingClientRect = () => createRect(0);

    const editor = document.createElement('div');
    editor.className = 'ProseMirror';
    const first = document.createElement('h1');
    first.textContent = 'Same';
    const second = document.createElement('h2');
    second.textContent = 'Same';
    first.getBoundingClientRect = () => createRect(80);
    second.getBoundingClientRect = () => createRect(240);
    editor.append(first, second);
    semanticPane.append(editor);

    const scrollTo = vi.fn();
    semanticPane.scrollTo = scrollTo;
    const outline: OutlineItem[] = [
      { id: 'same', level: 1, title: 'Same', line: 1 },
      { id: 'same-2', level: 2, title: 'Same', line: 3 },
    ];
    window.requestAnimationFrame = undefined as never;
    window.cancelAnimationFrame = vi.fn();

    scrollSemanticToAnchor(outline, semanticPane, {
      outlineId: 'same-2',
      sectionProgress: 0,
    });

    expect(scrollTo).not.toHaveBeenCalled();
    expect(semanticPane.scrollTop).toBe(208);
  });

  it('reports the active semantic heading by generated id', () => {
    const semanticPane = document.createElement('section');
    semanticPane.className = 'semantic-pane';
    semanticPane.getBoundingClientRect = () => createRect(0);

    const editor = document.createElement('div');
    editor.className = 'ProseMirror';
    const first = document.createElement('h1');
    first.textContent = 'Same';
    const second = document.createElement('h2');
    second.textContent = 'Same';
    first.getBoundingClientRect = () => createRect(20);
    second.getBoundingClientRect = () => createRect(60);
    editor.append(first, second);
    semanticPane.append(editor);

    expect(
      getActiveOutlineIdFromSemantic(
        [
          { id: 'same', level: 1, title: 'Same', line: 1 },
          { id: 'same-2', level: 2, title: 'Same', line: 3 },
        ],
        semanticPane,
      ),
    ).toBe('same-2');
  });

  it('matches the outline slug when a semantic heading contains real HTML delimiters', () => {
    const pane = document.createElement('section');
    pane.className = 'semantic-pane';
    pane.getBoundingClientRect = () => createRect(0);
    const editor = document.createElement('div');
    editor.className = 'ProseMirror';
    const heading = document.createElement('h1');
    heading.textContent = '<mark>标题</mark>';
    heading.getBoundingClientRect = () => createRect(20);
    editor.append(heading);
    pane.append(editor);
    expect(getActiveOutlineIdFromSemantic([
      { id: '标题', level: 1, title: '标题', line: 1 },
    ], pane)).toBe('标题');
  });

  it('maps a source section anchor into the matching semantic heading section', () => {
    useInstantScroll();
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 1200,
      clientHeight: 400,
      scrollTop: 500,
    });
    const sourceTextarea = createTextarea(80);
    sourcePane.append(sourceTextarea);
    const anchor = getSourceScrollAnchor(createOutline(), sourcePane.scrollTop, 20, sourceTextarea);
    const semanticPane = createSemanticPane([
      { tag: 'h1', title: '第一章', top: 40 },
      { tag: 'h2', title: '第二章', top: 440 },
    ], 1000, 300);

    scrollSemanticToAnchor(createOutline(), semanticPane, anchor);

    expect(semanticPane.scrollTop).toBe(258);
  });

  it('maps a semantic section anchor back into the matching source lines', () => {
    useInstantScroll();
    const semanticPane = createSemanticPane([
      { tag: 'h1', title: '第一章', top: 40 },
      { tag: 'h2', title: '第二章', top: 440 },
    ], 1000, 300);
    semanticPane.scrollTop = 250;
    const anchor = getSemanticScrollAnchor(createOutline(), semanticPane);
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 1600,
      clientHeight: 400,
      scrollTop: 0,
    });
    const sourceTextarea = createTextarea(80);
    sourcePane.append(sourceTextarea);

    scrollSourceToAnchor(createOutline(), sourcePane, sourceTextarea, anchor);

    expect(sourcePane.scrollTop).toBe(420);
  });

  it('falls back to document progress when source mode has no headings', () => {
    useInstantScroll();
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 1000,
      clientHeight: 400,
      scrollTop: 300,
    });
    const sourceTextarea = createTextarea(50);
    sourcePane.append(sourceTextarea);
    const anchor = getSourceScrollAnchor([], sourcePane.scrollTop, 20, sourceTextarea);
    const semanticPane = createSemanticPane([], 1100, 300);

    scrollSemanticToAnchor([], semanticPane, anchor);

    expect(semanticPane.scrollTop).toBe(400);
  });

  it('falls back to document progress when semantic mode has no headings', () => {
    useInstantScroll();
    const semanticPane = createSemanticPane([], 1100, 300);
    semanticPane.scrollTop = 400;
    const anchor = getSemanticScrollAnchor([], semanticPane);
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 1000,
      clientHeight: 400,
      scrollTop: 0,
    });
    const sourceTextarea = createTextarea(50);
    sourcePane.append(sourceTextarea);

    scrollSourceToAnchor([], sourcePane, sourceTextarea, anchor);

    expect(sourcePane.scrollTop).toBe(300);
  });

  it('keeps rich-block height differences inside the same outline section', () => {
    useInstantScroll();
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 2000,
      clientHeight: 400,
      scrollTop: 920,
    });
    const sourceTextarea = createTextarea(100);
    sourcePane.append(sourceTextarea);
    const outline: OutlineItem[] = [
      { id: 'intro', level: 1, title: 'Intro', line: 1 },
      { id: 'details', level: 2, title: 'Details', line: 31 },
      { id: 'next', level: 2, title: 'Next', line: 70 },
    ];
    const anchor = getSourceScrollAnchor(outline, sourcePane.scrollTop, 20, sourceTextarea);
    const semanticPane = createSemanticPane([
      { tag: 'h1', title: 'Intro', top: 20 },
      { tag: 'h2', title: 'Details', top: 760 },
      { tag: 'h2', title: 'Next', top: 1560 },
    ], 2200, 500);

    scrollSemanticToAnchor(outline, semanticPane, anchor);

    expect(anchor?.kind).toBe('outline');
    expect(anchor && 'outlineId' in anchor ? anchor.outlineId : '').toBe('details');
    expect(semanticPane.scrollTop).toBeGreaterThan(760);
    expect(semanticPane.scrollTop).toBeLessThan(1560);
  });

  it('uses the source viewport top line as the mode switch anchor', () => {
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 2000,
      clientHeight: 400,
      scrollTop: 800,
    });
    const sourceTextarea = createTextarea(100);
    const anchor = getSourceScrollAnchorAtLine(
      createOutline(),
      50,
      sourcePane.scrollTop,
      20,
      sourceTextarea,
      sourcePane,
    );

    expect(anchor?.kind).toBe('outline');
    expect(anchor && 'outlineId' in anchor ? anchor.outlineId : '').toBe('第二章');
    expect(anchor?.sourceLine).toBe(50);
    expect(anchor?.sectionProgress).toBeCloseTo(9 / 60);
  });

  it('calculates source top line with textarea offset inside the scroll pane', () => {
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 2500,
      clientHeight: 400,
      scrollTop: 1024,
    });
    const sourceTextarea = createTextarea(100);
    setOffsetTop(sourceTextarea, 44);
    sourcePane.append(sourceTextarea);

    const anchor = getSourceScrollAnchor(
      createOutline(),
      sourcePane.scrollTop,
      20,
      sourceTextarea,
      sourcePane,
    );

    expect(anchor?.kind).toBe('outline');
    expect(anchor && 'outlineId' in anchor ? anchor.outlineId : '').toBe('第二章');
    expect(anchor?.sourceLine).toBe(50);
    expect(anchor?.offsetFromTop).toBe(0);
  });

  it('restores source same-mode pixel anchors with textarea offset', () => {
    useInstantScroll();
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 2500,
      clientHeight: 400,
      scrollTop: 324,
    });
    const sourceTextarea = createTextarea(100);
    setOffsetTop(sourceTextarea, 44);
    sourcePane.append(sourceTextarea);
    const anchor = getSourceScrollAnchor([], sourcePane.scrollTop, 20, sourceTextarea, sourcePane);
    sourcePane.scrollTop = 0;

    restoreSourceReadingPosition([], sourcePane, sourceTextarea, anchor, {
      anchorMode: 'source',
      behavior: 'instant',
    });

    expect(sourcePane.scrollTop).toBe(324);
  });

  it('ignores source pixel fields when restoring an anchor from semantic mode', () => {
    useInstantScroll();
    const sourcePane = createScrollableElement('section', {
      className: 'source-pane',
      scrollHeight: 2500,
      clientHeight: 400,
      scrollTop: 0,
    });
    const sourceTextarea = createTextarea(100);
    setOffsetTop(sourceTextarea, 44);
    sourcePane.append(sourceTextarea);

    restoreSourceReadingPosition(
      createOutline(),
      sourcePane,
      sourceTextarea,
      {
        kind: 'outline',
        outlineId: '第二章',
        anchorPos: 1,
        offsetFromTop: 999,
        scrollTop: 999,
        sectionProgress: 0,
        documentProgress: 0,
        sourceLine: 41,
      },
      { anchorMode: 'semantic', behavior: 'instant' },
    );

    expect(sourcePane.scrollTop).toBe(844);
  });

  it('ignores semantic pixel fields when restoring an anchor from source mode', () => {
    useInstantScroll();
    const semanticPane = createSemanticPane([
      { tag: 'h1', title: '第一章', top: 40 },
      { tag: 'h2', title: '第二章', top: 440 },
    ], 1200, 300);

    restoreSemanticReadingPosition(
      createOutline(),
      semanticPane,
      {
        kind: 'outline',
        outlineId: '第二章',
        anchorPos: 1,
        offsetFromTop: 999,
        scrollTop: 999,
        sectionProgress: 0,
        documentProgress: 0,
        sourceLine: 41,
      },
      { anchorMode: 'source', behavior: 'instant' },
    );

    expect(semanticPane.scrollTop).toBe(408);
  });

  it('uses the semantic cursor block instead of the viewport top when it is visible', () => {
    const semanticPane = createSemanticPane([
      { tag: 'h1', title: '第一章', top: 40 },
      { tag: 'h2', title: '第二章', top: 440 },
    ], 1200, 300);
    semanticPane.scrollTop = 100;
    const paragraph = document.createElement('p');
    paragraph.textContent = 'cursor block';
    paragraph.getBoundingClientRect = () => createRect(300 - semanticPane.scrollTop);
    semanticPane.querySelector('.ProseMirror')?.append(paragraph);

    const anchor = getSemanticScrollAnchorForBlock(createOutline(), semanticPane, paragraph);

    expect(anchor?.kind).toBe('outline');
    expect(anchor && 'outlineId' in anchor ? anchor.outlineId : '').toBe('第一章');
    expect(anchor?.sectionProgress).toBeCloseTo(260 / 400);
    expect(anchor?.documentProgress).toBeCloseTo(300 / 900);
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

function createOutline(): OutlineItem[] {
  return [
    { id: '第一章', level: 1, title: '第一章', line: 1 },
    { id: '第二章', level: 2, title: '第二章', line: 41 },
  ];
}

function useInstantScroll() {
  window.requestAnimationFrame = undefined as never;
  window.cancelAnimationFrame = vi.fn();
}

function createSemanticPane(
  headings: Array<{ tag: 'h1' | 'h2'; title: string; top: number }>,
  scrollHeight: number,
  clientHeight: number,
) {
  const semanticPane = createScrollableElement('section', {
    className: 'semantic-pane',
    scrollHeight,
    clientHeight,
    scrollTop: 0,
  });
  semanticPane.getBoundingClientRect = () => createRect(0);

  const editor = document.createElement('div');
  editor.className = 'ProseMirror';
  headings.forEach((heading) => {
    const element = document.createElement(heading.tag);
    element.textContent = heading.title;
    element.getBoundingClientRect = () => createRect(heading.top - semanticPane.scrollTop);
    editor.append(element);
  });
  semanticPane.append(editor);
  return semanticPane;
}

function createTextarea(lineCount: number): HTMLTextAreaElement & MarkdownSourceEditorHandle {
  const textarea = document.createElement('textarea') as HTMLTextAreaElement &
    MarkdownSourceEditorHandle;
  textarea.style.lineHeight = '20px';
  textarea.value = Array.from({ length: lineCount }, (_, index) => `line ${index + 1}`).join('\n');
  textarea.getMarkdown = () => textarea.value;
  textarea.setMarkdown = (value) => {
    textarea.value = value;
  };
  textarea.getSelection = () => ({ from: textarea.selectionStart, to: textarea.selectionEnd });
  textarea.setSelection = (from, to = from) => textarea.setSelectionRange(from, to);
  textarea.getSelectedMarkdown = () =>
    textarea.value.slice(textarea.selectionStart, textarea.selectionEnd);
  textarea.revealRange = textarea.setSelection;
  textarea.undo = () => false;
  textarea.redo = () => false;
  textarea.lineAtOffset = (offset) => textarea.value.slice(0, offset).split(/\r?\n/).length;
  textarea.offsetAtLine = (line) =>
    textarea.value.split(/\r?\n/).slice(0, line - 1).join('\n').length + (line > 1 ? 1 : 0);
  textarea.getLineCount = () => textarea.value.split(/\r?\n/).length;
  textarea.getLineTop = (line) => textarea.offsetTop + (line - 1) * 20;
  textarea.lineAtHeight = (height) =>
    Math.max(1, Math.floor((height - textarea.offsetTop) / 20) + 1);
  textarea.getLineHeight = () => 20;
  textarea.getScrollElement = () =>
    textarea.closest<HTMLElement>('.source-pane') ?? textarea.parentElement ?? textarea;
  textarea.getContentElement = () => textarea;
  textarea.getContentHeight = () => textarea.scrollHeight;
  textarea.getBlockGeometry = () => [];
  textarea.applyBlockGaps = () => undefined;
  textarea.clearBlockGaps = () => undefined;
  textarea.requestMeasure = () => undefined;
  return textarea;
}

function setOffsetTop(element: HTMLElement, offsetTop: number) {
  Object.defineProperty(element, 'offsetTop', {
    configurable: true,
    value: offsetTop,
  });
}

function createScrollableElement<K extends keyof HTMLElementTagNameMap>(
  tagName: K,
  options: {
    className?: string;
    scrollHeight: number;
    clientHeight: number;
    scrollTop: number;
  },
) {
  const element = document.createElement(tagName);
  if (options.className) {
    element.className = options.className;
  }
  Object.defineProperty(element, 'scrollHeight', {
    configurable: true,
    value: options.scrollHeight,
  });
  Object.defineProperty(element, 'clientHeight', {
    configurable: true,
    value: options.clientHeight,
  });
  element.scrollTop = options.scrollTop;
  return element;
}
