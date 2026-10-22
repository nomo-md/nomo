// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { MarkdownResidentBudget, emptyResidentCost, UNKNOWN_IMAGE_BYTES } from './residentBudget';
import { MarkdownResidentTracker } from './residentTracker';

describe('驻留 DOM 与图片增量统计', () => {
  it('子树重挂、移动和增强结果替换准确计数，销毁后无残留', () => {
    const budget = new MarkdownResidentBudget(), host = {};
    budget.register(host, 'doc', () => {});
    const tracker = new MarkdownResidentTracker(budget);
    const a = document.createElement('div'), b = document.createElement('div');
    a.innerHTML = '<p>正文<span>公式</span></p>';
    function track(root: HTMLElement) {
      const reservation = budget.reserve(host, { ...emptyResidentCost(), views: 1 }, true);
      if (reservation.kind !== 'admitted') throw Error('reservation');
      tracker.track(root, reservation.token, 20, 1);
    }
    track(a); track(b);
    expect(budget.diagnostics.actual.domNodes).toBe(6);
    b.append(a.firstChild!); tracker.flush();
    expect(budget.diagnostics.actual.domNodes).toBe(6);
    b.querySelector('span')!.innerHTML = '<i>新</i><b>结果</b>'; tracker.flush();
    expect(budget.diagnostics.actual.domNodes).toBe(9);
    tracker.release(a); tracker.release(b); tracker.dispose(); budget.unregister(host);
    expect(budget.diagnostics.actual).toEqual(emptyResidentCost());
  });
  it('图片 resolve、解码、失败及 src 更换期间保持估算，副本独立累计', () => {
    const budget = new MarkdownResidentBudget(), host = {};
    budget.register(host, 'doc', () => {});
    const tracker = new MarkdownResidentTracker(budget), root = document.createElement('div');
    root.innerHTML = '<span class="image-node is-loading"></span>';
    const result = budget.reserve(host, { ...emptyResidentCost(), imageBytes: UNKNOWN_IMAGE_BYTES }, true);
    if (result.kind !== 'admitted') throw Error('reservation');
    tracker.track(root, result.token, 100, 1);
    expect(budget.diagnostics.actual.imageBytes).toBe(UNKNOWN_IMAGE_BYTES);
    const wrapper = root.firstElementChild!, image = document.createElement('img');
    image.src = '/large.png'; wrapper.append(image); wrapper.classList.remove('is-loading'); tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(UNKNOWN_IMAGE_BYTES);
    Object.defineProperties(image, { complete: { value: true, configurable: true }, naturalWidth: { value: 4000 }, naturalHeight: { value: 3000 } });
    image.dispatchEvent(new Event('load')); tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(48_000_000);
    const copy = document.createElement('img'); copy.src = image.src; root.append(copy); tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(48_000_000 + UNKNOWN_IMAGE_BYTES);
    copy.dispatchEvent(new Event('error')); tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(48_000_000);
    copy.src = '/other.png'; tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(48_000_000 + UNKNOWN_IMAGE_BYTES);
    const failed = document.createElement('img'); failed.src = '/already-failed.png';
    Object.defineProperty(failed, 'complete', { value: true });
    root.append(failed); tracker.flush();
    expect(budget.diagnostics.actual.imageBytes).toBe(48_000_000 + UNKNOWN_IMAGE_BYTES);
    tracker.dispose(); budget.unregister(host);
    expect(budget.diagnostics.actual).toEqual(emptyResidentCost());
  });
});
