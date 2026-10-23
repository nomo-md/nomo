import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte/pure';
import { tick, type ComponentProps } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { extractOutline } from '../../lib/outline/outlineService';
import EditorWorkspace from './EditorWorkspace.svelte';
import { OutlineProjection } from '../services/outlineProjection';
import type { MarkdownSourceEditorHandle } from './markdownSourceEditor';
import type { EditorCore } from '../../lib/editor-core';
import { resetOutlinePanelPosition } from '../services/outlinePanelPosition';
import { executeDesktopCommand, type AppCommandHandlers } from '../services/appCommands';

type Props = ComponentProps<typeof EditorWorkspace>;

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  resetOutlinePanelPosition();
  vi.restoreAllMocks();
});

describe('EditorWorkspace outline drag', () => {
  it('moves from the top region including its blank space, saves on release and restores across mounts', async () => {
    const moveOutlineSection = vi.fn(() => true);
    const rendered = await renderWorkspace({
      props: createProps('# One\n# Two', { moveOutlineSection }),
    });
    const { panel, title } = mockPanelGeometry(rendered.container);
    const capture = mockPointerCapture(title);
    await dispatchPointer(title, 'pointerdown', { clientX: 920, clientY: 180 });
    await dispatchPointer(window, 'pointermove', { clientX: 620, clientY: 220 });
    expect(panel.style.left).toBe('500px');
    expect(panel.style.top).toBe('108px');
    expect(panel.hasAttribute('data-floating')).toBe(false);
    expect(window.localStorage.getItem('newmd.outline.position.v1')).toBeNull();
    expect(rendered.container.querySelector('.outline-drag-preview')).toBeNull();
    await dispatchPointer(window, 'pointerup', { clientX: 620, clientY: 220 });
    expect(panel.hasAttribute('data-floating')).toBe(true);
    expect(JSON.parse(window.localStorage.getItem('newmd.outline.position.v1')!)).toEqual({ left: 500, top: 108 });
    expect(moveOutlineSection).not.toHaveBeenCalled();
    expect(capture.release).toHaveBeenCalledOnce();
    const rows = [...panel.querySelectorAll<HTMLElement>('.content-outline-row')];
    rows[1].getBoundingClientRect = () => new DOMRect(500, 30, 220, 30);
    mockPointerCapture(panel);
    await dragTo(rows[0], rows[1], 45);
    expect(moveOutlineSection).toHaveBeenCalledWith({ sourceIndex: 0, targetIndex: 1, placement: 'inside' });
    expect(panel.style.left).toBe('500px');
    expect(JSON.parse(window.localStorage.getItem('newmd.outline.position.v1')!)).toEqual({ left: 500, top: 108 });
    rendered.unmount();

    const reopened = await renderWorkspace({ props: createProps('# Another document') });
    const geometry = mockPanelGeometry(reopened.container);
    await fireEvent(window, new Event('resize'));
    await new Promise((resolve) => window.requestAnimationFrame(resolve));
    expect(geometry.panel.style.left).toBe('500px');
    expect(geometry.panel.style.top).toBe('108px');
    expect(geometry.panel.hasAttribute('data-floating')).toBe(true);
  });

  it('excludes the bulk button and cancels title movement without saving', async () => {
    const rendered = await renderWorkspace({
      props: createProps('# Parent\n## Child', { isOutlineItemExpandable: (index) => index === 0 }),
    });
    const { panel, title } = mockPanelGeometry(rendered.container);
    const capture = mockPointerCapture(title);
    const button = panel.querySelector('.outline-bulk-toggle')!;
    await dispatchPointer(button, 'pointerdown', { clientX: 920, clientY: 180 });
    await dispatchPointer(window, 'pointermove', { clientX: 620, clientY: 220 });
    expect(capture.set).not.toHaveBeenCalled();
    expect(panel.hasAttribute('data-positioned')).toBe(false);

    for (const cancelType of ['escape', 'pointercancel', 'blur', 'lostpointercapture']) {
      await dispatchPointer(title, 'pointerdown', { clientX: 920, clientY: 180 });
      await dispatchPointer(window, 'pointermove', { clientX: 620, clientY: 220 });
      expect(panel.hasAttribute('data-positioned')).toBe(true);
      if (cancelType === 'escape') await fireEvent.keyDown(window, { key: 'Escape' });
      else if (cancelType === 'blur') await fireEvent(window, new Event('blur'));
      else await dispatchPointer(cancelType === 'lostpointercapture' ? title : window, cancelType, { clientX: 620, clientY: 220 });
      expect(panel.hasAttribute('data-positioned')).toBe(false);
      expect(panel.style.left).toBe('');
      expect(window.localStorage.getItem('newmd.outline.position.v1')).toBeNull();
    }
  });

  it('constrains restored positions without overwriting them and resets from either menu', async () => {
    window.localStorage.setItem('newmd.outline.position.v1', JSON.stringify({ left: 9000, top: 9000 }));
    const openContextMenu = vi.fn();
    const rendered = await renderWorkspace({ props: createProps('# One', { openContextMenu }) });
    const { panel } = mockPanelGeometry(rendered.container);
    await fireEvent(window, new Event('resize'));
    await new Promise((resolve) => window.requestAnimationFrame(resolve));
    expect(panel.style.left).toBe('772px');
    expect(panel.style.top).toBe('372px');
    expect(JSON.parse(window.localStorage.getItem('newmd.outline.position.v1')!)).toEqual({ left: 9000, top: 9000 });

    await fireEvent.contextMenu(panel);
    const reset = openContextMenu.mock.lastCall![0].items.find((item: { icon: string }) => item.icon === 'undo');
    reset.action();
    expect(panel.hasAttribute('data-positioned')).toBe(false);
    expect(panel.hasAttribute('data-floating')).toBe(false);
    expect(panel.style.top).toBe('');
    expect(window.localStorage.getItem('newmd.outline.position.v1')).toBeNull();

    // 原生“视图”菜单必须使用同一重置入口，即使当前面板已隐藏。
    window.localStorage.setItem('newmd.outline.position.v1', JSON.stringify({ left: 100, top: 80 }));
    rendered.unmount();
    executeDesktopCommand('reset-outline-position', {} as AppCommandHandlers);
    expect(window.localStorage.getItem('newmd.outline.position.v1')).toBeNull();
  });

  it('publishes a semantic workspace before creating source, then reuses source until unmount', async () => {
    const ready = vi.fn();
    const destroy = vi.fn();
    const rendered = await renderWorkspace({ props: createProps('hello', {
      mode: 'semantic', sourceDocumentId: 'lazy', onRuntimeReady: ready, onRuntimeDestroy: destroy,
    }) });
    await tick();
    expect(rendered.container.querySelector('.cm-editor')).toBeNull();
    expect(ready).toHaveBeenCalledTimes(1);
    expect(ready.mock.lastCall?.[0].sourceEditor).toBeUndefined();
    // rerender 会重放初始 props；保留组件已经回写的 DOM 绑定。
    const host = rendered.container.querySelector<HTMLDivElement>('.prosemirror-host')!;
    const semanticPane = rendered.container.querySelector<HTMLElement>('.semantic-pane')!;
    await rendered.rerender({ mode: 'split', editorHost: host, semanticPane });
    await tick();
    const source = rendered.container.querySelector('.cm-editor');
    expect(source).not.toBeNull();
    expect(ready.mock.lastCall?.[0].sourceEditor).toBeDefined();
    expect(destroy).not.toHaveBeenCalled();
    const binding = ready.mock.lastCall![0];
    await rendered.rerender({ mode: 'semantic', editorHost: host, semanticPane, sourceEditor: binding.sourceEditor, sourcePane: binding.sourcePane });
    expect(rendered.container.querySelector('.cm-editor')).toBe(source);
    rendered.unmount();
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(destroy.mock.lastCall?.[0].sourceRuntimeState.state.doc.toString()).toBe('hello');
  });

  it('keeps a short press as a click and starts whole-row dragging only after 5px', async () => {
    const jumpToOutlineItem = vi.fn();
    const moveOutlineSection = vi.fn(() => true);
    const markdown = '# One\none\n# Two\ntwo\n';
    const { container } = await renderWorkspace({
      props: createProps(markdown, { jumpToOutlineItem, moveOutlineSection }),
    });
    const rows = [...container.querySelectorAll<HTMLElement>('.content-outline-row')];
    const panel = container.querySelector<HTMLElement>('.content-outline')!;
    const firstLink = rows[0].querySelector<HTMLButtonElement>('.outline-link')!;
    const pointerCapture = mockPointerCapture(panel);
    expect(panel.classList.contains('outline-dragging')).toBe(false);

    await dispatchPointer(firstLink, 'pointerdown', { clientX: 0, clientY: 5 });
    await dispatchPointer(window, 'pointermove', { clientX: 4, clientY: 5 });
    expect(pointerCapture.set).not.toHaveBeenCalled();
    await dispatchPointer(window, 'pointerup', { clientX: 2, clientY: 5 });
    await fireEvent.click(firstLink);
    expect(jumpToOutlineItem).toHaveBeenCalledTimes(1);
    expect(moveOutlineSection).not.toHaveBeenCalled();

    rows[1].getBoundingClientRect = () =>
      ({ top: 30, bottom: 60, left: 0, right: 200, width: 200, height: 30 } as DOMRect);
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: () => rows[1],
    });
    await dispatchPointer(rows[0], 'pointerdown', { clientX: 0, clientY: 5 });
    await dispatchPointer(window, 'pointermove', { clientX: 4, clientY: 5 });
    expect(document.body.querySelector('.outline-drag-preview')).toBeNull();

    await dispatchPointer(window, 'pointermove', { clientX: 6, clientY: 45 });
    expect(document.body.querySelector('.outline-drag-preview')).not.toBeNull();
    expect(panel.classList.contains('outline-dragging')).toBe(true);
    expect(rows[1].classList.contains('outline-drop-inside')).toBe(true);
    expect(pointerCapture.set).toHaveBeenCalledTimes(1);
    expect(pointerCapture.set).toHaveBeenCalledWith(1);
    await dispatchPointer(window, 'pointerup', { clientX: 6, clientY: 45 });
    expect(panel.classList.contains('outline-dragging')).toBe(false);
    expect(pointerCapture.release).toHaveBeenCalledTimes(1);
    expect(pointerCapture.release).toHaveBeenCalledWith(1);

    expect(moveOutlineSection).toHaveBeenCalledWith({
      sourceIndex: 0,
      targetIndex: 1,
      placement: 'inside',
    });

    await dragTo(rows[0], rows[1], 35);
    await dragTo(rows[0], rows[1], 55);
    expect(moveOutlineSection).toHaveBeenNthCalledWith(2, {
      sourceIndex: 0,
      targetIndex: 1,
      placement: 'before',
    });
    expect(moveOutlineSection).toHaveBeenNthCalledWith(3, {
      sourceIndex: 0,
      targetIndex: 1,
      placement: 'after',
    });
    expect(pointerCapture.set).toHaveBeenCalledTimes(3);
    expect(pointerCapture.release).toHaveBeenCalledTimes(3);
    expect(jumpToOutlineItem).toHaveBeenCalledTimes(1);
    expect(container.querySelector('[data-outline-drag-handle]')).toBeNull();
  });

  it('auto-expands a collapsed target after 500ms and cancels without moving on Escape', async () => {
    vi.useFakeTimers();
    const markdown = '# Source\n# Target\n## Child\n';
    const outline = extractOutline(markdown);
    const toggleOutlineItemExpanded = vi.fn();
    const moveOutlineSection = vi.fn(() => true);
    const { container } = await renderWorkspace({
      props: createProps(markdown, {
        collapsedOutlineIds: new Set([outline[1].id]),
        isOutlineItemExpandable: (index) => index === 1,
        toggleOutlineItemExpanded,
        moveOutlineSection,
      }),
    });
    const panel = container.querySelector<HTMLElement>('.content-outline')!;
    panel.getBoundingClientRect = () =>
      ({ top: 0, bottom: 300, left: 0, right: 220, width: 220, height: 300 } as DOMRect);
    const rows = [...container.querySelectorAll<HTMLElement>('.content-outline-row')];
    const pointerCapture = mockPointerCapture(panel);
    rows[1].getBoundingClientRect = () =>
      ({ top: 30, bottom: 60, left: 0, right: 200, width: 200, height: 30 } as DOMRect);
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      value: () => rows[1],
    });

    await dispatchPointer(rows[0], 'pointerdown', { clientX: 0, clientY: 5 });
    await dispatchPointer(window, 'pointermove', { clientX: 6, clientY: 45 });
    expect(pointerCapture.set).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(499);
    expect(toggleOutlineItemExpanded).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(toggleOutlineItemExpanded).toHaveBeenCalledWith(outline[1]);

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(document.body.querySelector('.outline-drag-preview')).toBeNull();
    expect(pointerCapture.release).toHaveBeenCalledTimes(1);
    await dispatchPointer(window, 'pointerup', { clientX: 6, clientY: 45 });
    expect(moveOutlineSection).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('keeps the expand toggle outside the whole-row drag state machine', async () => {
    const markdown = '# Parent\n## Child\n';
    const outline = extractOutline(markdown);
    const toggleOutlineItemExpanded = vi.fn();
    const moveOutlineSection = vi.fn(() => true);
    const { container } = await renderWorkspace({
      props: createProps(markdown, {
        isOutlineItemExpandable: (index) => index === 0,
        toggleOutlineItemExpanded,
        moveOutlineSection,
      }),
    });
    const firstRow = container.querySelector<HTMLElement>('.content-outline-row')!;
    const toggle = firstRow.querySelector<HTMLButtonElement>('.outline-toggle')!;
    const pointerCapture = mockPointerCapture(container.querySelector<HTMLElement>('.content-outline')!);

    await dispatchPointer(toggle, 'pointerdown', { clientX: 0, clientY: 5 });
    await dispatchPointer(window, 'pointermove', { clientX: 8, clientY: 5 });
    await dispatchPointer(window, 'pointerup', { clientX: 8, clientY: 5 });
    await fireEvent.click(toggle);

    expect(toggleOutlineItemExpanded).toHaveBeenCalledWith(outline[0]);
    expect(pointerCapture.set).not.toHaveBeenCalled();
    expect(moveOutlineSection).not.toHaveBeenCalled();
    expect(document.body.querySelector('.outline-drag-preview')).toBeNull();
  });
});

describe('EditorWorkspace paper clicks', () => {
  async function workspace(markdown = '第一段\n\n第二段', overrides: Partial<Props> = {}) {
    const place = vi.fn(() => true);
    const ready = vi.fn();
    const rendered = await renderWorkspace({ props: createProps(markdown, {
      mode: 'split', sourceDocumentId: 'paper', onRuntimeReady: ready,
      editorCore: { subscribe: () => () => undefined, placeCaretAtCoordinates: place,
        getScrollSyncSnapshot: () => ({ ready: false, revision: 0, renderRevision: 0, markdown, anchors: [] }) } as unknown as EditorCore,
      ...overrides,
    }) });
    await tick();
    const pane = rendered.container.querySelector<HTMLElement>('.semantic-pane')!;
    const host = pane.querySelector<HTMLElement>('.prosemirror-host')!;
    const root = document.createElement('div');
    root.className = 'ProseMirror';
    root.innerHTML = '<p>第一段</p><p>第二段</p>';
    host.append(root);
    return { ...rendered, pane, host, root, place, source: ready.mock.lastCall![0].sourceEditor as MarkdownSourceEditorHandle };
  }

  it.each(['', '第一段\n\n第二段'])('forwards all paper backgrounds for empty and populated documents: %s', async (markdown) => {
    const { pane, host, root, place } = await workspace(markdown);
    for (const target of [pane, host.parentElement!, host, root]) {
      await dispatchPointer(target, 'pointerdown', { clientX: 800, clientY: 500 });
      expect(place).toHaveBeenLastCalledWith({ left: 800, top: 500 });
    }
    expect(place).toHaveBeenCalledTimes(4);
  });

  it('lets paragraph gutter selection finish before paper handling and leaves body or control clicks alone', async () => {
    const { pane, host, root, place } = await workspace();
    pane.addEventListener('pointerdown', (event) => event.preventDefault(), { once: true });
    await dispatchPointer(host.parentElement!, 'pointerdown', { clientX: 80, clientY: 100 });
    const input = document.createElement('textarea');
    const button = document.createElement('button');
    host.append(input, button);
    for (const target of [root.firstElementChild!, input, button])
      await dispatchPointer(target, 'pointerdown', { clientX: 120, clientY: 100 });
    expect(place).not.toHaveBeenCalled();
  });

  it('routes source paper clicks only to its registered source editor', async () => {
    const { container, source, place } = await workspace();
    const sourcePlace = vi.spyOn(source, 'placeCaretAtCoordinates').mockReturnValue(true);
    await dispatchPointer(container.querySelector('.source-pane .document-layout')!, 'pointerdown', { clientX: 80, clientY: 500 });
    expect(sourcePlace).toHaveBeenCalledWith({ left: 80, top: 500 });
    expect(place).not.toHaveBeenCalled();
  });

  it('ignores modified clicks, secondary buttons, touch and actual scrollbar areas', async () => {
    const { pane, place } = await workspace();
    for (const init of [{ shiftKey: true }, { ctrlKey: true }, { metaKey: true }, { altKey: true }, { button: 2 }]) {
      const event = new MouseEvent('pointerdown', { bubbles: true, cancelable: true, ...init });
      Object.defineProperties(event, { isPrimary: { value: true }, pointerType: { value: 'mouse' } });
      pane.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    const touch = new MouseEvent('pointerdown', { bubbles: true, cancelable: true });
    Object.defineProperties(touch, { isPrimary: { value: true }, pointerType: { value: 'touch' } });
    pane.dispatchEvent(touch);
    vi.spyOn(pane, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 500, 400));
    Object.defineProperties(pane, { offsetWidth: { value: 500 }, offsetHeight: { value: 400 }, clientWidth: { value: 480 }, clientHeight: { value: 400 } });
    await dispatchPointer(pane, 'pointerdown', { clientX: 490, clientY: 100 });
    expect(place).not.toHaveBeenCalled();
  });

  it('keeps readonly paper non-editable', async () => {
    const { pane, place } = await workspace('只读内容', { readonlyDocumentMode: true });
    await dispatchPointer(pane, 'pointerdown', { clientX: 80, clientY: 500 });
    expect(place).not.toHaveBeenCalled();
  });
});

async function renderWorkspace(options: { props: Props }) {
  const rendered = render(EditorWorkspace, options);
  const panel = rendered.container.querySelector<HTMLElement>('.content-outline');
  if (panel) {
    Object.defineProperty(panel, 'clientHeight', { configurable: true, value: 320 });
    if (vi.isFakeTimers()) {
      for (let i = 0; i < 100 && panel.dataset.outlineReady !== 'true'; i++) await vi.advanceTimersByTimeAsync(16);
      expect(panel.dataset.outlineReady).toBe('true');
    }
    else await waitFor(() => expect(panel.dataset.outlineReady).toBe('true'));
    await tick();
  }
  return rendered;
}

function mockPanelGeometry(container: HTMLElement) {
  const grid = container.querySelector<HTMLElement>('.editor-grid')!;
  const panel = container.querySelector<HTMLElement>('.content-outline')!;
  const title = panel.querySelector<HTMLElement>('.content-outline-header')!;
  Object.defineProperties(grid, { clientWidth: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 600 } });
  Object.defineProperties(panel, { offsetWidth: { configurable: true, value: 220 }, offsetHeight: { configurable: true, value: 220 } });
  grid.getBoundingClientRect = () => new DOMRect(100, 100, 1000, 600);
  panel.getBoundingClientRect = () => new DOMRect(900, 168, 220, 220);
  return { grid, panel, title };
}

function createProps(markdown: string, overrides: Partial<Props> = {}): Props {
  const outline = extractOutline(markdown);
  const outlineProjection = new OutlineProjection('test', 7);
  outlineProjection.update(outline, 0, 'ready');
  for (const item of outline) if (overrides.collapsedOutlineIds?.has(item.id)) outlineProjection.toggle(item);
  while (!outlineProjection.complete) outlineProjection.advance(outlineProjection.generation, () => 0);
  const noop = () => undefined;
  return {
    interfaceLocale: 'en-US',
    mode: 'source',
    markdown,
    largeDocumentMode: false,
    frontMatter: null,
    frontMatterEditing: false,
    frontMatterFocusRequest: 0,
    frontMatterFocusTarget: 'default',
    readonlyDocumentMode: false,
    outlineVisible: true,
    outline,
    activeOutlineId: '',
    collapsedOutlineIds: new Set(),
    outlineProjection,
    sourceEditor: undefined as unknown as MarkdownSourceEditorHandle,
    sourcePane: undefined as unknown as HTMLElement,
    semanticPane: undefined as unknown as HTMLElement,
    editorHost: undefined as unknown as HTMLDivElement,
    editorCore: { subscribe: () => () => undefined } as unknown as EditorCore,
    updateMarkdown: noop,
    enterFrontMatterEdit: noop,
    leaveFrontMatterEdit: noop,
    updateFrontMatterContent: noop,
    deleteFrontMatter: noop,
    updateActiveOutlineFromSourceScroll: noop,
    updateActiveOutlineFromSemanticScroll: noop,
    handleEditorPaste: noop,
    handleEditorDrop: noop,
    handleWorkspaceContextMenu: noop,
    openContextMenu: noop,
    copyContextText: noop,
    isOutlineItemExpandable: () => false,
    toggleOutlineItemExpanded: noop,
    jumpToOutlineItem: noop,
    moveOutlineSection: () => false,
    ...overrides,
  };
}

async function dispatchPointer(
  target: EventTarget,
  type: string,
  init: { clientX: number; clientY: number },
) {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    button: 0,
    clientX: init.clientX,
    clientY: init.clientY,
  });
  Object.defineProperties(event, {
    pointerId: { value: 1 },
    pointerType: { value: 'mouse' },
    isPrimary: { value: true },
  });
  target.dispatchEvent(event);
  await tick();
}

async function dragTo(source: HTMLElement, target: HTMLElement, clientY: number) {
  Object.defineProperty(document, 'elementFromPoint', {
    configurable: true,
    value: () => target,
  });
  await dispatchPointer(source, 'pointerdown', { clientX: 0, clientY: 5 });
  await dispatchPointer(window, 'pointermove', { clientX: 6, clientY });
  await dispatchPointer(window, 'pointerup', { clientX: 6, clientY });
}

function mockPointerCapture(row: HTMLElement) {
  let captured = false;
  const set = vi.fn(() => {
    captured = true;
  });
  const release = vi.fn(() => {
    captured = false;
  });
  const has = vi.fn(() => captured);
  Object.defineProperties(row, {
    setPointerCapture: { configurable: true, value: set },
    releasePointerCapture: { configurable: true, value: release },
    hasPointerCapture: { configurable: true, value: has },
  });
  return { set, release, has };
}
