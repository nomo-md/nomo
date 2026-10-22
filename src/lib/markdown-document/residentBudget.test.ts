import { afterEach, describe, expect, it, vi } from 'vitest';
import { MarkdownResidentBudget, emptyResidentCost, type MarkdownResidentCost } from './residentBudget';
import { emptyEnhancements } from './enhancements';

afterEach(() => vi.useRealTimers());
const progress = { revision: 1, state: 'ready' as const, totalBytes: 10000, indexedBytes: 10000,
  layoutUnits: 100, enhancementObjects: 0, enhancements: emptyEnhancements(), metricsComplete: true };
function setup() {
  const budget = new MarkdownResidentBudget({ views: 4, sourceBytes: 10000, domNodes: 100, imageBytes: 1000 }, () => Date.now());
  const a = {}, b = {}, changed = vi.fn();
  budget.register(a, 'a', changed); budget.register(b, 'b', changed);
  budget.updateDocument(a, progress); budget.updateDocument(b, progress);
  return { budget, a, b, changed };
}
function reserve(budget: MarkdownResidentBudget, host: object, cost: Partial<MarkdownResidentCost>, required = false) {
  const result = budget.reserve(host, { ...emptyResidentCost(), ...cost }, required);
  if (result.kind !== 'admitted') throw Error(result.kind);
  return result.token;
}
describe('共享驻留预算', () => {
  it('连续交互只推迟共享可选扩展，基础内容照常准入且隐藏取消等待', async () => {
    vi.useFakeTimers();
    const { budget, a, b, changed } = setup();
    budget.interacting(a);
    expect(budget.reserve(b, emptyResidentCost(), false).kind).toBe('waiting');
    const required = reserve(budget, b, { views: 1 }, true);
    await vi.advanceTimersByTimeAsync(100);
    budget.interacting(a);
    await vi.advanceTimersByTimeAsync(299);
    expect(changed).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(changed).toHaveBeenCalledOnce();
    const optional = reserve(budget, b, { views: 1 });
    budget.release(optional); budget.release(required);
    budget.holdInteraction(a, 'composition', true);
    expect(budget.reserve(b, emptyResidentCost(), false).kind).toBe('waiting');
    await vi.advanceTimersByTimeAsync(1000);
    expect(vi.getTimerCount()).toBe(0);
    budget.holdInteraction(a, 'composition', false);
    expect(vi.getTimerCount()).toBe(1);
    budget.unregister(b); budget.unregister(a);
    expect(vi.getTimerCount()).toBe(0);
    expect(budget.diagnostics).toMatchObject({ hosts: 0, entries: 0 });
  });
  it('未知可选块串行预占，取消唤醒其它宿主且不触发压力', async () => {
    const { budget, a, b, changed } = setup();
    const token = reserve(budget, a, { views: 1 });
    expect(budget.reserve(b, emptyResidentCost(), false).kind).toBe('waiting');
    budget.release(token); await Promise.resolve();
    expect(changed).toHaveBeenCalled();
    expect(budget.pressured(a)).toBe(false);
    expect(budget.diagnostics.reserved.views).toBe(0);
    const next = reserve(budget, b, { views: 1 });
    budget.commit(next, { ...emptyResidentCost(), views: 1, domNodes: 10 });
    expect(budget.diagnostics.actual).toMatchObject({ views: 1, domNodes: 10 });
    budget.unregister(a); budget.unregister(b);
    expect(budget.diagnostics).toMatchObject({ hosts: 0, documents: 0, entries: 0 });
    expect(budget.commit(next, { ...emptyResidentCost(), views: 99 })).toBe(false);
  });
  it('跨宿主累计，必需内容越线仍准入，释放自身资源不会恢复全文', async () => {
    vi.useFakeTimers();
    const { budget, a, b } = setup();
    const one = reserve(budget, a, { views: 2 }, true);
    budget.commit(one, { ...emptyResidentCost(), views: 2, domNodes: 70 });
    const two = reserve(budget, b, { views: 2 }, true);
    budget.commit(two, { ...emptyResidentCost(), views: 2, domNodes: 40 });
    expect(budget.pressured(a)).toBe(true); expect(budget.pressured(b)).toBe(true);
    expect(budget.diagnostics.protectedExcess.domNodes).toBe(10);
    budget.release(two); budget.release(one);
    await vi.advanceTimersByTimeAsync(10000);
    expect(budget.pressured(a)).toBe(true);
    budget.unregister(b);
    budget.holdInteraction(a, 'composition', true);
    await vi.advanceTimersByTimeAsync(10000);
    expect(budget.pressured(a)).toBe(true);
    budget.holdInteraction(a, 'composition', false);
    await vi.advanceTimersByTimeAsync(1500);
    budget.interacting(a);
    await vi.advanceTimersByTimeAsync(1999);
    expect(budget.pressured(a)).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    expect(budget.pressured(a)).toBe(false);
    budget.unregister(a);
  });
  it('预占拒绝锁存，文档减少 20% 后仍须低水位和空闲同时成立', async () => {
    vi.useFakeTimers();
    const { budget, a, b } = setup();
    const token = reserve(budget, a, { views: 3 }, true);
    budget.commit(token, { ...emptyResidentCost(), views: 3 });
    expect(budget.reserve(b, { ...emptyResidentCost(), views: 2 }, false).kind).toBe('denied');
    budget.updateDocument(a, { ...progress, totalBytes: 8000 });
    await vi.advanceTimersByTimeAsync(4000);
    expect(budget.pressured(a)).toBe(true);
    budget.release(token);
    await vi.advanceTimersByTimeAsync(2000);
    expect(budget.pressured(a)).toBe(false);
    expect(budget.pressured(b)).toBe(true);
    budget.unregister(a); budget.unregister(b);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('低水位计时期间的临时预占也会重置连续两秒，不要求再次超过上限', async () => {
    vi.useFakeTimers();
    const { budget, a, b } = setup();
    const pressure = reserve(budget, a, { views: 5 }, true);
    budget.release(pressure); budget.unregister(b);
    await vi.advanceTimersByTimeAsync(1000);
    const pending = reserve(budget, a, { views: 3 }, true);
    await vi.advanceTimersByTimeAsync(100);
    budget.release(pending);
    await vi.advanceTimersByTimeAsync(1999);
    expect(budget.pressured(a)).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    expect(budget.pressured(a)).toBe(false);
    budget.unregister(a);
  });
});
