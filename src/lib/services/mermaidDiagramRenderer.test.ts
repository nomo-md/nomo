// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { createMermaidDiagramRenderer } from './mermaidDiagramRenderer';

const mermaidMock = vi.hoisted(() => ({
  registerIconPacks: vi.fn(),
  initialize: vi.fn(),
  render: vi.fn(async (_id: string, code: string) => ({ svg: `<svg>${code}</svg>` })),
}));

vi.mock('mermaid', () => ({
  default: mermaidMock,
}));

describe('createMermaidDiagramRenderer', () => {
  it('reuses Mermaid initialization for the same theme and reinitializes on theme changes', async () => {
    const renderer = createMermaidDiagramRenderer();

    await renderer.renderMermaid('flowchart TD\n  A --> B', { theme: { theme: 'default' } });
    await renderer.renderMermaid('flowchart TD\n  B --> C', { theme: { theme: 'default' } });
    await renderer.renderMermaid('flowchart TD\n  C --> D', { theme: { theme: 'dark' } });

    expect(mermaidMock.initialize).toHaveBeenCalledTimes(2);
    expect(mermaidMock.initialize).toHaveBeenNthCalledWith(1, {
      startOnLoad: false,
      securityLevel: 'strict',
      theme: 'default',
      themeVariables: undefined,
    });
    expect(mermaidMock.initialize).toHaveBeenNthCalledWith(2, {
      startOnLoad: false,
      securityLevel: 'strict',
      theme: 'dark',
      themeVariables: undefined,
    });
    expect(mermaidMock.render).toHaveBeenCalledTimes(3);
    const renderIds = mermaidMock.render.mock.calls.map(([id]) => id);
    expect(new Set(renderIds).size).toBe(3);
  });
  it('reuses SVG across renderer instances while retaining theme and source boundaries', async () => {
    const code = 'flowchart LR\n cacheA --> cacheB';
    const first = await createMermaidDiagramRenderer().renderMermaid(code, { theme: { theme: 'default' } });
    const count = mermaidMock.render.mock.calls.length;
    const again = await createMermaidDiagramRenderer().renderMermaid(code, { theme: { theme: 'default' } });
    const text = (svg: string) => new DOMParser().parseFromString(svg, 'text/html').body.textContent;
    expect(text(again.svg)).toBe(text(first.svg));
    expect(mermaidMock.render).toHaveBeenCalledTimes(count);
    await createMermaidDiagramRenderer().renderMermaid(code, { theme: { theme: 'dark' } });
    await createMermaidDiagramRenderer().renderMermaid(code + '\n cacheB --> cacheC', { theme: { theme: 'dark' } });
    expect(mermaidMock.render).toHaveBeenCalledTimes(count + 2);
  });
  it('does not publish or cache a cancelled in-flight render and removes the temporary surface', async () => {
    const abort = new AbortController();
    mermaidMock.render.mockImplementationOnce(async () => {
      abort.abort();
      return { svg: '<svg>cancelled</svg>' };
    });
    const renderer = createMermaidDiagramRenderer();
    const code = 'flowchart LR\n cancelledA --> cancelledB';
    const before = document.body.childElementCount;
    expect((await renderer.renderMermaid(code, { theme: { theme: 'default' }, signal: abort.signal })).error).toBeTruthy();
    const count = mermaidMock.render.mock.calls.length;
    await renderer.renderMermaid(code, { theme: { theme: 'default' } });
    expect(mermaidMock.render).toHaveBeenCalledTimes(count + 1);
    expect(document.body.childElementCount).toBe(before);
    expect((await renderer.renderMermaid(code, { theme: { theme: 'default' }, signal: abort.signal })).error).toBeTruthy();
  });
});
