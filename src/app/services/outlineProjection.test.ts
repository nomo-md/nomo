import { describe, expect, it } from 'vitest';
import type { OutlineItem } from '../../lib/outline/outlineService';
import { getOutlineWindow, OutlineProjection } from './outlineProjection';

const heading = (id: string, level: OutlineItem['level'] = 1): OutlineItem => ({ id, level, title: id, line: 1 });
function finish(model: OutlineProjection) {
  let slices = 0;
  while (!model.complete && slices++ < 1000) model.advance(model.generation, () => 0);
  expect(model.complete).toBe(true);
}

describe('progressive outline lifetime', () => {
  it('follows structural identity when duplicate heading slugs are renumbered by an edit', () => {
    const model = new OutlineProjection('duplicates');
    const original = [{ ...heading('same'), blockId: 'a' }, { ...heading('child', 2), blockId: 'b' },
      { ...heading('same-2'), blockId: 'c' }, { ...heading('child-2', 2), blockId: 'd' }];
    model.update(original, 1, 'ready'); finish(model);
    model.toggle(original[2]); finish(model);
    const anchor = { id: 'same-2', key: 'c:0', index: 2, offset: 7 };
    // 删除前面的同名章节后，原来的 same-2 变成 same，仍然应该折叠。
    const edited = [{ ...original[2], id: 'same' }, { ...original[3], id: 'child' }];
    model.update(edited, 2, 'ready'); finish(model);
    expect(model.isCollapsed(edited[0])).toBe(true);
    expect(model.visible).toEqual([0]);
    expect(model.positionFor(anchor)).toBe(0);
    // 新插入的同名标题不继承其他结构块的用户状态。
    const added = [{ ...heading('same'), blockId: 'e' }, ...original.map(item => ({ ...item, id: `${item.id}-next` }))];
    model.update(added, 3, 'ready'); finish(model);
    expect(model.isCollapsed(added[0])).toBe(false);
    const nested = [
      { ...heading('quote-a'), blockId: 'quote', blockHeadingIndex: 0 },
      { ...heading('quote-b'), blockId: 'quote', blockHeadingIndex: 1 },
    ];
    model.update(nested, 4, 'ready'); finish(model);
    model.toggle(nested[0]); finish(model);
    expect(model.isCollapsed(nested[1])).toBe(false);
    expect(model.identity(nested[0])).not.toBe(model.identity(nested[1]));
    const temporary = [
      { ...heading('local-heading:1'), blockId: '', blockHeadingIndex: 0 },
      { ...heading('local-heading:2'), blockId: '', blockHeadingIndex: 0 },
    ];
    expect(temporary.map(item => model.identity(item))).toEqual(['local-heading:1', 'local-heading:2']);
  });

  it('appends at a parent boundary without reprocessing or losing a user collapse', () => {
    const model = new OutlineProjection('doc');
    const items = [heading('root'), heading('child', 2)];
    model.update(items, 1, 'partial'); finish(model);
    model.toggle(items[1]); finish(model);
    items.push(heading('grandchild', 3), heading('sibling', 2), heading('next'));
    model.update(items, 1, 'ready');
    expect(model.processed).toBe(2);
    finish(model);
    expect(model.visible.map(i => items[i].id)).toEqual(['root', 'child', 'sibling', 'next']);
    expect(model.expandableCount).toBe(2);
    expect(model.collapsedExpandableCount).toBe(1);
  });

  it('bounds each slice and keeps a 4225-heading document within a three-viewport DOM window', () => {
    const model = new OutlineProjection('large');
    model.update(Array.from({ length: 4225 }, (_, i) => heading(String(i))), 1, 'ready');
    model.advance(model.generation, () => 0);
    expect(model.processed).toBe(128);
    expect(model.ready).toBe(false);
    finish(model);
    const first = getOutlineWindow(model, 32000, 640, 32, false);
    expect(first.length).toBe(20);
    const buffered = getOutlineWindow(model, 32000, 640, 32, true, '1');
    expect(buffered.length).toBe(61);
    expect(buffered.find(row => row.item.id === '1')).toBeDefined();
    expect(model.headings[1000]).toBe(first[0].item);
  });

  it('yields on time budget and rejects obsolete generations and revisions', () => {
    const model = new OutlineProjection('doc');
    const items = Array.from({ length: 400 }, (_, i) => heading(String(i)));
    model.update(items, 2, 'ready');
    let now = 0;
    model.advance(model.generation, () => now++);
    expect(model.processed).toBe(3);
    const oldGeneration = model.generation;
    model.update([heading('new')], 3, 'ready');
    model.advance(oldGeneration, () => 0);
    expect(model.processed).toBe(0);
    model.update(items, 2, 'ready');
    expect(model.headings[0].id).toBe('new');
    model.dispose(); model.advance();
    expect(model.headings).toHaveLength(0);
    expect(model.visible).toHaveLength(0);
  });

  it('preserves explicit expansion across revisions and restores a removed anchor to its visible parent', () => {
    const model = new OutlineProjection('doc', 2);
    const items = [heading('root'), heading('a', 2), heading('a-child', 3), heading('b', 2), heading('b-child', 3)];
    model.update(items, 1, 'ready'); finish(model);
    model.toggle(items[1]); finish(model);
    model.anchor = { id: 'a-child', index: 2, offset: 5 };
    model.update([...items], 2, 'partial'); finish(model);
    expect(model.isCollapsed(items[1])).toBe(false);
    const updated = [items[0], items[1], items[3], items[4]];
    model.update(updated, 3, 'ready'); finish(model);
    expect(model.positionFor(model.anchor)).toBe(2);
    expect(model.positionFor({ id: 'b-child', index: 3, offset: 0 })).toBe(2);
  });

  it('distinguishes empty building, empty ready and failed states, and bulk intent applies to future headings', () => {
    const model = new OutlineProjection('empty', 2);
    model.update([], 0, 'building'); finish(model);
    expect(model.ready).toBe(false);
    model.update([], 0, 'ready'); finish(model);
    expect(model.ready).toBe(true);
    model.setDefaultLevel(7);
    const items = [heading('same'), heading('same-2', 2), heading('child', 3)];
    model.update(items, 1, 'ready'); finish(model);
    expect(model.visible).toEqual([0, 1, 2]);
    model.update(items, 1, 'ready', new Error('index failed'));
    expect(model.ready).toBe(false);
    expect(model.failure).toBeInstanceOf(Error);
  });
});
