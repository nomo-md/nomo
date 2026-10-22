// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
// The exact patched version is locked in pnpm-workspace.yaml/pnpm-lock.yaml.
// @ts-expect-error Mermaid's internal config module has no declaration file.
import { getConfig, setConfig, setSiteConfig, updateSiteConfig, addDirective, reset, assignWithDepth_default as assign } from 'mermaid/dist/chunks/mermaid.core/chunk-CSCIHK7Q.mjs';
// @ts-expect-error Local dependency patch, exercised through its installed entry.
import { createRenderTasks } from 'mermaid/dist/chunks/mermaid.core/nomo-render-tasks.mjs';

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren(); setSiteConfig({ theme: 'default' }); reset(); });

describe('Mermaid configuration copy compatibility', () => {
  it('retains the original depth, null, function and array semantics while isolating copies', () => {
    const callback = () => 'test';
    const values = { custom: { nullable: null, callback, array: [1, 1, { id: 2 }], nested: { nullable: null, array: [1, 1], deep: { id: 3 } } } };
    setConfig(values);
    const expected = assign({}, values).custom;
    const first = getConfig(), second = getConfig();
    expect(first.custom).toEqual(expected);
    expect(first.custom.callback).toBe(callback);
    expect(first.custom).not.toBe(second.custom);
    expect(first.custom.array).not.toBe(second.custom.array);
    expect(first.custom.nested).not.toBe(second.custom.nested);
    // Deeper values have the same sharing boundary as upstream assignWithDepth.
    expect(first.custom.nested.deep).toBe(second.custom.nested.deep);
    first.custom.array.push(7); first.custom.nested.nullable = 'changed'; first.flowchart.nodeSpacing = 999;
    expect(getConfig().custom).toEqual(expected);
    expect(getConfig().flowchart.nodeSpacing).not.toBe(999);
  });
  it('invalidates the plan for site updates, directives, setConfig and reset', () => {
    setSiteConfig({ theme: 'default', flowchart: { nodeSpacing: 61 } });
    expect(getConfig().flowchart.nodeSpacing).toBe(61);
    updateSiteConfig({ flowchart: { nodeSpacing: 62 } });
    expect(getConfig().flowchart.nodeSpacing).toBe(62);
    addDirective({ flowchart: { nodeSpacing: 63 }, securityLevel: 'loose' });
    expect(getConfig().flowchart.nodeSpacing).toBe(63);
    expect(getConfig().securityLevel).toBe('strict');
    setConfig({ flowchart: { nodeSpacing: 64 } });
    expect(getConfig().flowchart.nodeSpacing).toBe(64);
    reset();
    expect(getConfig().flowchart.nodeSpacing).toBe(62);
    setSiteConfig({ theme: 'dark' });
    expect(getConfig().theme).toBe('dark');
  });
  it('retains copy isolation when a configuration exceeds the plan budget', () => {
    setConfig({ custom: { text: 'x'.repeat(256 * 1024), items: [1, 2] } });
    const first = getConfig(); first.custom.items.push(3);
    expect(getConfig().custom.items).toEqual([1, 2]);
    expect(getConfig().custom.text).toHaveLength(256 * 1024);
  });
});

describe('Mermaid cooperative rendering', () => {
  it('preserves node order and yields between bounded batches', async () => {
    const root = document.createElement('div'); document.body.append(root);
    let time = 0; vi.spyOn(performance, 'now').mockImplementation(() => time);
    const pause = vi.fn(async () => {}); vi.stubGlobal('scheduler', { yield: pause });
    const visited: number[] = [];
    await createRenderTasks(root)(Array.from({ length: 400 }, (_, i) => i), async (i: number) => { visited.push(i); time++; });
    expect(visited).toEqual(Array.from({ length: 400 }, (_, i) => i));
    expect(pause.mock.calls.length).toBeGreaterThan(1);
    vi.restoreAllMocks();
  });
  it('stops pending batches when the render surface is removed and releases errors', async () => {
    const root = document.createElement('div'); document.body.append(root);
    const visited: number[] = [];
    await expect(createRenderTasks(root)(Array.from({ length: 400 }, (_, i) => i), async (i: number) => {
      visited.push(i); if (i === 5) root.remove();
    })).rejects.toMatchObject({ name: 'AbortError' });
    expect(visited.length).toBe(32);
    await expect(createRenderTasks(document.createElement('div'))([1], () => { throw new Error('node failed'); })).rejects.toThrow('node failed');
  });
});
