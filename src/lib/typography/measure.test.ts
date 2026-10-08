import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { measureParagraph, measureParagraphSync } from './measure';
import { DEFAULT_TYPOGRAPHY } from './types';

let clock = 0;

function advance(text: string): number {
  return text.length * 8 - (text.match(/AV/g)?.length ?? 0) * 2;
}

function paragraph(text = 'sample'): HTMLElement {
  const element = document.createElement('p');
  element.style.cssText =
    'writing-mode:horizontal-tb;text-align:left;font-family:Arial;font-size:16px;' +
    'font-weight:400;font-style:normal;font-stretch:normal;line-height:28px;text-indent:6px;';
  Object.defineProperties(element, {
    clientWidth: { value: 320 },
    offsetWidth: { value: 320 },
  });
  element.textContent = text;
  document.body.append(element);
  return element;
}

beforeEach(() => {
  clock = 0;
  vi.spyOn(performance, 'now').mockImplementation(() => clock);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
    font: '',
    measureText: (text: string) => ({
      width: advance(text),
      actualBoundingBoxAscent: 10,
      actualBoundingBoxDescent: 3,
      actualBoundingBoxLeft: 0,
      actualBoundingBoxRight: advance(text),
    }),
  } as unknown as CanvasRenderingContext2D);
  vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(function (this: Range) {
    return new DOMRect(0, 0, advance(this.toString()), 16);
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.tagName === 'P') return new DOMRect(0, 0, 320, 28);
    if (this.classList.contains('task-checkbox-widget')) return new DOMRect(0, 0, 20, 16);
    if (this.style.verticalAlign === 'baseline') return new DOMRect(0, 12, 0, 0);
    return new DOMRect(0, 0, advance(this.textContent ?? ''), 16);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe('paragraph measurement scheduling', () => {
  it('shares shaping, styled spans, source-code boxes, breaks and checkbox bindings', async () => {
    const element = paragraph('A');
    const continuation = document.createElement('span');
    continuation.style.cssText = element.style.cssText;
    continuation.textContent = 'V';
    const styled = document.createElement('span');
    styled.style.cssText = 'font-family:Arial;font-size:20px;font-weight:700;';
    styled.textContent = '中';
    const softBreak = document.createElement('br');
    softBreak.dataset.nomoBreak = 'soft';
    const code = document.createElement('span');
    code.className = 'pm-inline-source-code';
    code.style.cssText = 'font-family:monospace;font-size:14px;margin-left:2px;margin-right:3px;';
    code.innerHTML =
      '<span class="pm-inline-source-marker is-hidden">`</span> code ' +
      '<span class="pm-inline-source-marker is-hidden">`</span>';
    const checkbox = document.createElement('span');
    checkbox.className = 'ProseMirror-widget task-checkbox-widget';
    checkbox.style.cssText = 'margin-left:3px;margin-right:4px;';
    element.append(
      continuation,
      styled,
      ' \u00adZ',
      softBreak,
      'word',
      code,
      document.createElement('br'),
      '尾',
      checkbox,
    );
    const originalHtml = element.innerHTML;

    const sync = measureParagraphSync(element, DEFAULT_TYPOGRAPHY);
    const asyncResult = await measureParagraph(element, DEFAULT_TYPOGRAPHY);

    expect(sync).not.toBeNull();
    expect(sync).toEqual(asyncResult);
    expect(sync!.bindings.slice(0, 2).map((binding) => binding.width)).toEqual([8, 6]);
    expect(sync!.input.items.find((item) => item.text === '中')?.fontSize).toBe(20);
    expect(sync!.bindings.find((binding) => binding.node === code)).toMatchObject({
      atomic: true,
      sourceSize: 8,
      width: 53,
    });
    expect(sync!.input.items.find((item) => item.kind === 'penalty')?.breakWidth).toBe(8);
    expect(sync!.input.firstLineIndent).toBe(33);
    expect(sync!.input.breaks.filter((point) => point.required)).toHaveLength(2);
    expect(element.innerHTML).toBe(originalHtml);
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
  });

  it('drains measurement batches synchronously without scheduling timers', () => {
    const element = paragraph('abcd');
    const measure = vi.spyOn(Range.prototype, 'getBoundingClientRect');
    measure.mockImplementation(function (this: Range) {
      clock += 9;
      return new DOMRect(0, 0, advance(this.toString()), 16);
    });
    const schedule = vi.spyOn(globalThis, 'setTimeout');

    const result = measureParagraphSync(element, DEFAULT_TYPOGRAPHY, 50);

    expect(result?.bindings).toHaveLength(4);
    expect(schedule).not.toHaveBeenCalled();
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
  });

  it('measures existing per-character KP wrappers with one sandbox write and original bindings', () => {
    const text = 'AV 中文 '.repeat(32).slice(0, 190);
    const plain = paragraph(text);
    const expected = measureParagraphSync(plain, DEFAULT_TYPOGRAPHY);
    const fragmented = paragraph('');
    const wrappers = Array.from(text, (character, index) => {
      const span = document.createElement('span');
      span.className = 'kp-unit';
      span.style.cssText = `${fragmented.style.cssText}margin-right:${index % 3}px;line-height:32px;`;
      span.textContent = character;
      fragmented.append(span);
      return span;
    });
    const writes = vi.spyOn(Node.prototype, 'textContent', 'set');
    const styles = vi.spyOn(globalThis, 'getComputedStyle');

    const measured = measureParagraphSync(fragmented, DEFAULT_TYPOGRAPHY);

    expect(measured).not.toBeNull();
    expect(measured!.input).toEqual(expected!.input);
    expect(measured!.bindings.map((binding) => binding.node)).toEqual(
      wrappers.map((wrapper) => wrapper.firstChild),
    );
    expect(
      measured!.bindings.map(({ offset, endOffset, from, to }) => ({
        offset,
        endOffset,
        from,
        to,
      })),
    ).toEqual(
      Array.from(text, (_, index) => ({ offset: 0, endOffset: 1, from: index, to: index + 1 })),
    );
    expect(
      writes.mock.calls.filter((_, index) => {
        const node = writes.mock.contexts[index];
        return node instanceof HTMLElement && node.dataset.kpOwned === 'measurement';
      }),
    ).toEqual([[text]]);
    expect(
      styles.mock.calls.filter(([node]) => wrappers.includes(node as HTMLSpanElement)),
    ).toHaveLength(wrappers.length);
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
  });

  it('uses the short sync budget while retaining the async 1500 ms budget', async () => {
    const element = paragraph('sample');
    vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(function (this: Range) {
      clock = 25;
      return new DOMRect(0, 0, advance(this.toString()), 16);
    });

    expect(measureParagraphSync(element, DEFAULT_TYPOGRAPHY)).toBeNull();
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
    clock = 0;
    expect((await measureParagraph(element, DEFAULT_TYPOGRAPHY))?.text).toBe('sample');
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();

    clock = 0;
    vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(() => {
      clock = 1_501;
      return new DOMRect();
    });
    expect(await measureParagraph(element, DEFAULT_TYPOGRAPHY)).toBeNull();
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
  });

  it('cleans the sandbox if initialization throws and does not alter the source DOM', () => {
    const element = paragraph();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => {
      expect(document.querySelector('[data-kp-owned="measurement"]')).not.toBeNull();
      throw new Error('canvas unavailable');
    });

    expect(() => measureParagraphSync(element, DEFAULT_TYPOGRAPHY)).toThrow('canvas unavailable');
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
    expect(element.textContent).toBe('sample');
  });

  it('returns null for an exhausted budget or unsupported layout before creating a sandbox', async () => {
    const element = paragraph();
    expect(measureParagraphSync(element, DEFAULT_TYPOGRAPHY, 0)).toBeNull();
    element.style.textAlign = 'center';
    expect(measureParagraphSync(element, DEFAULT_TYPOGRAPHY)).toBeNull();
    expect(await measureParagraph(element, DEFAULT_TYPOGRAPHY)).toBeNull();
    expect(HTMLCanvasElement.prototype.getContext).not.toHaveBeenCalled();
    expect(document.querySelector('[data-kp-owned="measurement"]')).toBeNull();
  });
});
