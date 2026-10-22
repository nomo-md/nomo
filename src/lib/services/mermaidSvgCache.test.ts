// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { MermaidSvgCache } from './mermaidSvgCache';

describe('Mermaid SVG cache', () => {
  it('isolates duplicate diagrams and rewrites references without changing labels or colors', () => {
    const cache = new MermaidSvgCache();
    cache.put('code', `<svg id="chart" aria-labelledby="title desc"><style>#chart .node {fill:#fff} #fff {stroke:url(#arrow)}</style><title id="title">chart #chart</title><desc id="desc">description</desc><defs><marker id="arrow"/></defs><g id="fff" marker-end="url('#arrow')"><a href="#fff">node</a></g></svg>`);
    const one = cache.get('code', 'one')!, two = cache.get('code', 'two')!;
    const root = document.createElement('div'); root.innerHTML = one + two;
    const ids = [...root.querySelectorAll('[id]')].map(node => node.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const svg of root.querySelectorAll('svg')) {
      expect(svg.querySelector('title')?.textContent).toBe('chart #chart');
      expect(svg.querySelector('style')?.textContent).toContain('fill:#fff');
      expect(svg.querySelector('style')?.textContent).toContain(`#${svg.id} .node`);
      for (const id of svg.getAttribute('aria-labelledby')!.split(' ')) expect(svg.querySelector(`[id="${id}"]`)).not.toBeNull();
      expect(svg.querySelector('a')?.getAttribute('href')).toBe('#' + svg.querySelector('g')?.id);
      expect(svg.querySelector('g')?.getAttribute('marker-end')).toBe(`url('#${svg.querySelector('marker')?.id}')`);
    }
  });
  it('bounds both bytes and entry count, renews hits and releases all data', () => {
    const cache = new MermaidSvgCache(2, 600);
    cache.put('a', '<svg/>'); cache.put('b', '<svg/>'); cache.get('a', 'first'); cache.put('c', '<svg/>');
    expect(cache.size).toBe(2); expect(cache.get('b', 'missing')).toBeUndefined();
    cache.put('large', '<svg>' + 'x'.repeat(600) + '</svg>');
    expect(cache.get('large', 'missing')).toBeUndefined();
    expect(cache.estimatedBytes).toBeLessThanOrEqual(600);
    cache.put('c', '<svg>changed</svg>');
    expect(cache.get('c', 'changed')).toContain('changed');
    cache.clear(); expect(cache.size).toBe(0); expect(cache.estimatedBytes).toBe(0);
  });
  it('rewrites absolute marker URLs enabled by Mermaid arrowMarkerAbsolute', () => {
    const cache = new MermaidSvgCache();
    cache.put('absolute', '<svg><marker id="arrow"/><path marker-end="url(http://tauri.localhost/#arrow)"/></svg>');
    expect(cache.get('absolute', 'new')).toContain('url(http://tauri.localhost/#new-0)');
  });
});
