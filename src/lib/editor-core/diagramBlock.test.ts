import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { EditorState, NodeSelection, TextSelection } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { createEditorCore, setCodeBlockDiagramRenderer } from './createEditorCore';
import { DIAGRAM_TEMPLATES } from './diagramTemplates';
import { executeEditorCommand } from './editorCommands';
import { parseMarkdown, serializeMarkdown } from './markdown';
import { MermaidBlockNodeView } from './nodeViews/MermaidBlockNodeView';
import { schema } from './schema';

afterEach(() => {
  vi.useRealTimers();
});

describe('mermaid_block markdown', () => {
  it('parses mermaid fenced code block as mermaid_block node', () => {
    const doc = parseMarkdown('```mermaid\nflowchart TD\n  A --> B\n```');
    const mermaidBlock = findFirstNode(doc, 'mermaid_block');

    expect(mermaidBlock?.attrs.code).toBe('flowchart TD\n  A --> B');
  });

  it('serializes mermaid_block back to standard mermaid fence', () => {
    const node = schema.nodes.mermaid_block.create({ code: 'sequenceDiagram\n  A->>B: Hi' });
    const doc = schema.nodes.doc.create(null, [node]);

    expect(serializeMarkdown(doc)).toContain('```mermaid\nsequenceDiagram\n  A->>B: Hi\n```');
  });

  it('keeps non-mermaid fenced code blocks as code_block', () => {
    const doc = parseMarkdown('```ts\nconst ok = true;\n```');

    expect(findFirstNode(doc, 'code_block')?.textContent).toBe('const ok = true;');
    expect(findFirstNode(doc, 'mermaid_block')).toBeNull();
  });
});

describe('diagram templates', () => {
  it('inserts all first-phase diagram templates as mermaid markdown', () => {
    for (const template of DIAGRAM_TEMPLATES) {
      const target = document.createElement('div');
      const editor = createEditorCore({ markdown: '', target });

      expect(editor.execute({ type: 'insertDiagramBlock', diagramType: template.type })).toBe(true);
      expect(editor.getMarkdown()).toContain('```mermaid');
      expect(editor.getMarkdown()).toContain(template.code.split('\n')[0]);

      editor.destroy();
      target.remove();
    }
  });

  it('keeps legacy insertMermaidBlock command working', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });

    expect(editor.execute({ type: 'insertMermaidBlock', code: 'flowchart TD\n  A --> B' })).toBe(
      true,
    );
    expect(editor.getMarkdown()).toContain('```mermaid\nflowchart TD\n  A --> B\n```');

    editor.destroy();
    target.remove();
  });

  it('inserts blank Mermaid blocks without default chart content', () => {
    const target = document.createElement('div');
    const editor = createEditorCore({ markdown: '', target });

    expect(editor.execute({ type: 'insertMermaidBlock' })).toBe(true);

    expect(editor.getMarkdown()).toContain('```mermaid\n\n```');
    expect(editor.getMarkdown()).not.toContain('flowchart TD');
    expect(target.querySelector('.mermaid-block.ProseMirror-selectednode')).toBeNull();
    expect(target.querySelector('.mermaid-block')).not.toBeNull();

    if (!target.querySelector('.mermaid-block-textarea')) {
      target.querySelector<HTMLElement>('.mermaid-block')?.click();
    }
    expect(target.querySelector('.mermaid-block-textarea')).not.toBeNull();

    editor.destroy();
    target.remove();
  });

  it('keeps inserted diagram blocks in display mode without selecting the chart node', () => {
    setCodeBlockDiagramRenderer({
      async renderMermaid() {
        return { svg: '<svg viewBox="0 0 100 50"><g></g></svg>' };
      },
    });

    const target = document.createElement('div');
    document.body.appendChild(target);

    const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
    const view = new EditorView(target, {
      state: EditorState.create({ doc }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    expect(
      executeEditorCommand(
        { type: 'insertDiagramBlock', diagramType: 'flowchart' },
        view,
        '',
        () => {},
      ),
    ).toBe(true);

    expect(view.state.selection).toBeInstanceOf(TextSelection);
    expect(view.state.selection.from).toBe(2);
    expect(view.state.selection.to).toBe(2);
    expect(target.querySelector('.mermaid-block-textarea')).toBeNull();
    expect(target.querySelector('.mermaid-block.ProseMirror-selectednode')).toBeNull();

    view.destroy();
    target.remove();
  });
});

describe('MermaidBlockNodeView', () => {
  it('renders display mode and enters edit mode with source above preview', async () => {
    const renderCalls: string[] = [];
    setCodeBlockDiagramRenderer({
      async renderMermaid(code) {
        renderCalls.push(code);
        return { svg: `<svg data-code="${code.split('\n')[0]}"></svg>` };
      },
    });

    const node = schema.nodes.mermaid_block.create({ code: 'flowchart TD\n  A --> B' });
    const doc = schema.nodes.doc.create(null, [node]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: NodeSelection.create(doc, 0),
      }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    await Promise.resolve();
    expect(target.querySelector('.mermaid-block svg')).not.toBeNull();

    target.querySelector<HTMLElement>('.mermaid-block')?.click();

    const textarea = target.querySelector('.mermaid-block-textarea');
    const preview = target.querySelector('.mermaid-block-preview');
    const editSurface = target.querySelector('.mermaid-block-edit-surface');
    const previewSnapshot = target.querySelector('.mermaid-block-preview-snapshot');
    expect(editSurface).not.toBeNull();
    expect(previewSnapshot).not.toBeNull();
    expect(textarea).not.toBeNull();
    expect(preview).not.toBeNull();
    expect(textarea!.compareDocumentPosition(preview!) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
    expect(renderCalls).toHaveLength(1);

    view.destroy();
    target.remove();
  });

  it('normalizes Mermaid SVG intrinsic size before inserting it into the editor', async () => {
    setCodeBlockDiagramRenderer({
      async renderMermaid() {
        return {
          svg: '<svg width="100%" style="max-width: 165px;" viewBox="0.5 0 165 470"><g data-diagram="flowchart"></g></svg>',
        };
      },
    });

    const node = schema.nodes.mermaid_block.create({ code: 'flowchart TD\n  A --> B' });
    const doc = schema.nodes.doc.create(null, [node]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({ doc }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    await Promise.resolve();

    const svg = target.querySelector<SVGElement>('.mermaid-block-rendered > svg');
    expect(svg?.getAttribute('width')).toBe('165');
    expect(svg?.getAttribute('height')).toBe('470');
    expect(svg?.getAttribute('style')).toBeNull();

    view.destroy();
    target.remove();
  });

  it('keeps a stale display render from overwriting edit mode', async () => {
    const pendingRenders: Array<(value: { svg: string }) => void> = [];
    setCodeBlockDiagramRenderer({
      renderMermaid() {
        return new Promise<{ svg: string }>((resolve) => {
          pendingRenders.push(resolve);
        });
      },
    });

    const node = schema.nodes.mermaid_block.create({ code: 'flowchart TD\n  A --> B' });
    const doc = schema.nodes.doc.create(null, [node]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({
        doc,
        selection: NodeSelection.create(doc, 0),
      }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    const block = target.querySelector<HTMLElement>('.mermaid-block');
    expect(block).not.toBeNull();
    block?.click();
    expect(target.querySelector('.mermaid-block-textarea')).not.toBeNull();

    pendingRenders[0]?.({ svg: '<svg data-stale-display-render="true"></svg>' });
    await Promise.resolve();

    expect(target.querySelector('.mermaid-block-textarea')).not.toBeNull();
    expect(target.querySelector('[data-stale-display-render="true"]')).toBeNull();

    view.destroy();
    target.remove();
  });

  it('debounces edit-mode Mermaid preview rendering and uses the latest source', async () => {
    vi.useFakeTimers();
    const renderCalls: string[] = [];
    setCodeBlockDiagramRenderer({
      async renderMermaid(code) {
        renderCalls.push(code);
        return { svg: `<svg data-code="${code}"></svg>` };
      },
    });

    const node = schema.nodes.mermaid_block.create({ code: 'flowchart TD\n  A --> B' });
    const doc = schema.nodes.doc.create(null, [node]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({ doc }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    await Promise.resolve();
    expect(renderCalls).toHaveLength(1);

    target.querySelector<HTMLElement>('.mermaid-block')?.click();
    const textarea = target.querySelector<HTMLTextAreaElement>('.mermaid-block-textarea');
    expect(textarea).not.toBeNull();

    textarea!.value = 'flowchart TD\n  A --> C';
    textarea!.dispatchEvent(new Event('input', { bubbles: true }));
    textarea!.value = 'flowchart TD\n  A --> D';
    textarea!.dispatchEvent(new Event('input', { bubbles: true }));

    vi.advanceTimersByTime(249);
    expect(renderCalls).toHaveLength(1);

    vi.advanceTimersByTime(1);
    await Promise.resolve();

    expect(renderCalls).toEqual(['flowchart TD\n  A --> B', 'flowchart TD\n  A --> D']);

    view.destroy();
    target.remove();
  });

  it('opens rendered Mermaid diagrams in a fullscreen preview', async () => {
    setCodeBlockDiagramRenderer({
      async renderMermaid() {
        return {
          svg: '<svg data-fullscreen-source="diagram" width="100%" viewBox="0 0 100 50"><g></g></svg>',
        };
      },
    });

    const node = schema.nodes.mermaid_block.create({ code: 'flowchart TD\n  A --> B' });
    const doc = schema.nodes.doc.create(null, [node]);
    const target = document.createElement('div');
    document.body.appendChild(target);

    const view = new EditorView(target, {
      state: EditorState.create({ doc }),
      nodeViews: {
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
      },
    });

    await Promise.resolve();

    const button = target.querySelector<HTMLButtonElement>('.mermaid-block-fullscreen-button');
    expect(button).not.toBeNull();
    button?.click();

    const overlay = document.body.querySelector('.mermaid-fullscreen-overlay');
    expect(overlay).not.toBeNull();
    expect(overlay?.querySelector('[data-fullscreen-source="diagram"]')).not.toBeNull();
    expect(overlay?.querySelector('.mermaid-fullscreen-header')).toBeNull();
    expect(overlay?.querySelector('.mermaid-fullscreen-zoom-badge')?.textContent).toBe('125%');
    expect(
      overlay
        ?.querySelector('.mermaid-fullscreen-viewport .mermaid-block-rendered > svg')
        ?.getAttribute('width'),
    ).toBe('125');
    expect(
      overlay
        ?.querySelector('.mermaid-fullscreen-viewport .mermaid-block-rendered > svg')
        ?.getAttribute('height'),
    ).toBe('63');
    expect(overlay?.textContent).not.toContain('图表预览');
    expect(target.querySelector('.mermaid-block-textarea')).toBeNull();

    overlay?.querySelector('.mermaid-fullscreen-viewport')?.dispatchEvent(
      new WheelEvent('wheel', {
        ctrlKey: true,
        deltaY: -100,
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(overlay?.querySelector('.mermaid-fullscreen-zoom-badge')?.textContent).toBe('135%');
    expect(
      overlay
        ?.querySelector('.mermaid-fullscreen-viewport .mermaid-block-rendered > svg')
        ?.getAttribute('width'),
    ).toBe('135');
    expect(
      overlay
        ?.querySelector('.mermaid-fullscreen-viewport .mermaid-block-rendered > svg')
        ?.getAttribute('height'),
    ).toBe('68');

    overlay
      ?.querySelector('.mermaid-fullscreen-viewport')
      ?.dispatchEvent(createPointerLikeEvent('pointerdown', 1));
    expect(overlay?.querySelector('.mermaid-fullscreen-viewport')?.className).toContain(
      'is-dragging',
    );
    overlay
      ?.querySelector('.mermaid-fullscreen-viewport')
      ?.dispatchEvent(createPointerLikeEvent('pointerup', 1));
    expect(overlay?.querySelector('.mermaid-fullscreen-viewport')?.className).not.toContain(
      'is-dragging',
    );

    document.body.querySelector<HTMLButtonElement>('.mermaid-fullscreen-close-button')?.click();
    expect(document.body.querySelector('.mermaid-fullscreen-overlay')).toBeNull();

    view.destroy();
    target.remove();
  });
});

function findFirstNode(doc: ProseMirrorNode, name: string): ProseMirrorNode | null {
  let found: ProseMirrorNode | null = null;
  doc.descendants((node) => {
    if (node.type.name === name) {
      found = node;
      return false;
    }
    return true;
  });
  return found;
}

function createPointerLikeEvent(type: string, pointerId: number): MouseEvent {
  const event = new MouseEvent(type, {
    button: 0,
    clientX: 120,
    clientY: 120,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, 'pointerId', { value: pointerId });
  return event;
}

describe('Mermaid theme refresh', () => {
  let view: EditorView | undefined;
  afterEach(() => {
    view?.destroy();
    view = undefined;
    document.body.replaceChildren();
    vi.unstubAllGlobals();
  });

  // 模拟真实滚动可见性，检验屏幕外主题合并和导出补齐，而非依赖 jsdom 的布局。
  function setupDiagram() {
    let notify: IntersectionObserverCallback;
    const disconnect = vi.fn();
    vi.stubGlobal(
      'IntersectionObserver',
      class {
        constructor(callback: IntersectionObserverCallback) {
          notify = callback;
        }
        observe() {}
        disconnect = disconnect;
      },
    );
    const render = vi.fn(async (_code, { theme }) => ({
      svg: `<svg viewBox="0 0 100 50" data-test-theme="${theme.theme}"></svg>`,
    }));
    setCodeBlockDiagramRenderer({ renderMermaid: render });
    const target = document.createElement('div');
    document.body.append(target);
    view = new EditorView(target, {
      state: EditorState.create({
        doc: schema.nodes.doc.create(null, [
          schema.nodes.mermaid_block.create({ code: 'flowchart TD\nA --> B' }),
        ]),
      }),
      nodeViews: {
        mermaid_block: (node, editor, getPos) =>
          new MermaidBlockNodeView(node, editor, getPos as () => number),
      },
    });
    return {
      target,
      render,
      disconnect,
      visible(value: boolean) {
        notify!(
          [{ isIntersecting: value } as IntersectionObserverEntry],
          {} as IntersectionObserver,
        );
      },
    };
  }

  it('coalesces offscreen changes and renders the latest theme before entering the viewport', async () => {
    const diagram = setupDiagram();
    await Promise.resolve();
    diagram.render.mockClear();
    diagram.visible(false);
    MermaidBlockNodeView.updateTheme({ theme: 'dark' });
    MermaidBlockNodeView.updateTheme({ theme: 'default' });
    expect(diagram.render).not.toHaveBeenCalled();

    diagram.visible(true);
    await MermaidBlockNodeView.flushThemeUpdates(diagram.target);
    expect(diagram.render).toHaveBeenCalledTimes(1);
    expect(diagram.target.querySelector('svg')?.getAttribute('data-test-theme')).toBe('default');
    view!.destroy();
    view = undefined;
    expect(diagram.disconnect).toHaveBeenCalledOnce();
  });

  it('flushes offscreen themes before an export snapshot', async () => {
    const diagram = setupDiagram();
    await Promise.resolve();
    diagram.visible(false);
    MermaidBlockNodeView.updateTheme({ theme: 'dark' });
    await MermaidBlockNodeView.flushThemeUpdates(diagram.target);
    expect(diagram.target.querySelector('svg')?.getAttribute('data-test-theme')).toBe('dark');
  });
});
