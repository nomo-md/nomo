import { describe, expect, it } from 'vitest';
import { MarkdownStableGroups, MarkdownScrollDemand } from './mountWindow';
import { MarkdownStructureScanner } from './structure';
import { MarkdownLayoutCache } from './layoutCache';

describe('稳定块组与有界准备', () => {
  it('保留已确认编辑窗口的新结构区间，前后余段不丢失或重叠', () => {
    const blocks = new MarkdownStructureScanner(0).push('段落\n\n'.repeat(140), true).blocks;
    const grouping = new MarkdownStableGroups();
    const initial = grouping.update(blocks);
    const changed = blocks.map((block, i) => i === 10 ? { ...block, id: 'edited' } : block);
    const groups = grouping.update(changed, [{ id: initial[0].id, from: 5, to: 65 }]);
    expect(groups.find(group => group.id === initial[0].id)).toMatchObject({ from: 5, to: 65 });
    expect(groups.flatMap(group => group.blockIds)).toEqual(changed.map(block => block.id));
    expect(groups.every((group, i) => group.from === (groups[i - 1]?.to ?? 0))).toBe(true);
    expect(groups.every(group => group.to - group.from <= 64)).toBe(true);
    expect(grouping.update(changed)).toBe(groups);
  });
  it('同一索引追加保留已封口组对象，仅更新尾部', () => {
    const blocks = new MarkdownStructureScanner(0).push('段落\n\n'.repeat(200), true).blocks;
    const incremental = blocks.slice(0, 70), grouping = new MarkdownStableGroups();
    const before = [...grouping.update(incremental)];
    incremental.push(...blocks.slice(70));
    const after = grouping.update(incremental);
    expect(after[0]).toBe(before[0]); expect(after[1]).toBe(before[1]);
    expect(after[2].from).toBe(70);
    incremental[incremental.length - 1] = { ...incremental.at(-1)!, toByte: blocks.at(-1)!.toByte + 10 };
    expect(grouping.update(incremental)[0]).toBe(before[0]);
  });
  it('单项失效只扣除所属缓存字节，重复删除不改变计数', () => {
    const cache = new MarkdownLayoutCache(10, 4096);
    cache.put('a', { height: 20 }); cache.put('b', { height: 30 });
    const bytes = cache.estimatedBytes;
    expect(cache.delete('a')).toBe(true);
    expect(cache.estimatedBytes).toBe(bytes / 2);
    expect(cache.delete('a')).toBe(false);
    expect(cache.get('b')?.height).toBe(30);
    cache.delete('b'); expect(cache.estimatedBytes).toBe(0);
  });
  it('追加索引封口旧组，前方新增块不重新划分后方组', () => {
    const blocks = new MarkdownStructureScanner(0).push('段落\n\n'.repeat(200), true).blocks;
    const grouping = new MarkdownStableGroups();
    const first = grouping.update(blocks.slice(0, 70));
    const appended = grouping.update(blocks);
    expect(appended.slice(0, first.length).map(group => group.id)).toEqual(first.map(group => group.id));
    const inserted = { ...blocks[0], id: 'new' };
    const shifted = grouping.update([inserted, ...blocks]);
    expect(shifted.slice(1).map(group => group.id)).toEqual(appended.map(group => group.id));
    expect(shifted[1].from).toBe(1);
    expect(shifted.every(group => group.to - group.from <= 64)).toBe(true);
  });
  it('重复内容具有不同身份，单组变化不导致后续组变化', () => {
    const blocks = new MarkdownStructureScanner(0).push('重复\n\n'.repeat(200), true).blocks;
    expect(new Set(blocks.map(block => block.id)).size).toBe(blocks.length);
    const grouping = new MarkdownStableGroups();
    const before = grouping.update(blocks);
    const after = grouping.update(blocks.map((block, i) => i === 2 ? { ...block, id: 'changed' } : block));
    expect(after.slice(1).map(group => group.id)).toEqual(before.slice(1).map(group => group.id));
  });
  it('速度扩展有上限，反向保留旧余量，静默后收缩，远跳不污染速度', () => {
    const demand = new MarkdownScrollDemand();
    expect(demand.range(800, 0).after).toBe(2400);
    demand.sample(0, 800, 0);
    demand.sample(4000, 800, 20);
    expect(demand.range(800, 20).after).toBe(4800);
    demand.sample(0, 800, 40);
    expect(demand.range(800, 40).after).toBe(4800);
    expect(demand.range(800, 341).after).toBe(2400);
    demand.sample(100000, 800, 342);
    expect(demand.range(800, 342).after).toBe(2400);
    for (let i = 0; i < 32; i++) demand.prepared(10);
    demand.prepared(20);
    expect(demand.preparationMs).toBe(160);
  });
  it('布局缓存同时限制条数和估算字节，命中续期，释放归零', () => {
    const cache = new MarkdownLayoutCache(2, 200);
    cache.put('a', { height: 20 }); cache.put('b', { height: 30 });
    expect(cache.get('a')?.height).toBe(20);
    cache.put('c', { height: 40 });
    expect(cache.get('b')).toBeUndefined();
    expect(cache.estimatedBytes).toBeLessThanOrEqual(200);
    cache.put('x'.repeat(200), { height: 50 });
    expect(cache.size).toBe(2);
    cache.clear();
    expect(cache.size).toBe(0); expect(cache.estimatedBytes).toBe(0);
  });
  it('多个文档和模式共用总预算，清理一个分区不删除其他文档', () => {
    const pool = new MarkdownLayoutCache(3, 600);
    const first = pool.scope('1:semantic:'), second = pool.scope('2:semantic:'), source = pool.scope('1:source:');
    first.put('a', { height: 20 }); second.put('a', { height: 30 }); source.put('a', { height: 40 });
    expect(first.get('a')?.height).toBe(20);
    second.put('b', { height: 50 });
    expect(second.get('a')).toBeUndefined();
    expect(pool.size).toBe(3); expect(pool.estimatedBytes).toBeLessThanOrEqual(600);
    first.clear();
    expect(first.size).toBe(0); expect(first.estimatedBytes).toBe(0);
    expect(second.get('b')?.height).toBe(50); expect(source.get('a')?.height).toBe(40);
  });
});
