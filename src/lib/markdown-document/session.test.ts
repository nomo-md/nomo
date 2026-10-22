import { describe, expect, it } from 'vitest';
import { MarkdownDocumentSession } from './session';
import { DocumentConflictError } from './protocol';
import { Utf8Text, utf8Length } from './utf8Text';
import { MarkdownMountPolicy } from './mountPolicy';
import { emptyEnhancements } from './enhancements';

describe('Markdown document integrity', () => {
  it('keeps an immutable complete snapshot while later edits continue', () => {
    const original = '# 开头\r\n\r\n' + '尾部 😀  \r\n'.repeat(10_000);
    const session = new MarkdownDocumentSession('doc', original);
    const saved = session.captureSnapshot();
    session.applyTransaction({
      baseRevision: 0,
      operationId: '1',
      historyGroupId: 'typing',
      edits: [{ fromByte: 2, toByte: 2, insertedText: '新' }],
    });
    expect(saved.text.read()).toBe(original);
    expect(session.captureSnapshot().text.read()).toBe('# 新' + original.slice(2));
    expect(session.markSaved(saved).dirty).toBe(true);
    expect(session.undo().dirty).toBe(false);
    expect(session.captureSnapshot().text.read()).toBe(original);
    expect(session.redo().dirty).toBe(true);
  });
  it('groups IME and multi-range edits without dropping history on snapshots', () => {
    const session = new MarkdownDocumentSession('doc', '甲\n乙\n尾');
    session.applyTransaction({
      baseRevision: 0,
      operationId: '1',
      historyGroupId: 'composition',
      edits: [{ fromByte: 0, toByte: 3, insertedText: '中' }],
    });
    session.applyTransaction({
      baseRevision: 1,
      operationId: '2',
      historyGroupId: 'composition',
      edits: [
        { fromByte: 0, toByte: 3, insertedText: '中文' },
        { fromByte: 4, toByte: 7, insertedText: '段' },
      ],
    });
    expect(session.captureSnapshot().text.read()).toBe('中文\n段\n尾');
    expect(session.undo().canUndo).toBe(false);
    expect(session.captureSnapshot().text.read()).toBe('甲\n乙\n尾');
    session.redo();
    expect(session.captureSnapshot().text.read()).toBe('中文\n段\n尾');
  });
  it('deduplicates retries and rejects stale, overlapping and split-character transactions atomically', () => {
    const session = new MarkdownDocumentSession('doc', 'a😀\r\nb');
    const transaction = {
      baseRevision: 0,
      operationId: '1',
      historyGroupId: 'paste',
      edits: [{ fromByte: 1, toByte: 5, insertedText: '中' }],
    };
    expect(session.applyTransaction(transaction).revision).toBe(1);
    expect(session.applyTransaction(transaction).revision).toBe(1);
    expect(() => session.applyTransaction({ ...transaction, operationId: '2' })).toThrow(
      DocumentConflictError,
    );
    expect(() =>
      session.applyTransaction({
        ...transaction,
        baseRevision: 1,
        operationId: '3',
        edits: [{ fromByte: 2, toByte: 3, insertedText: '' }],
      }),
    ).toThrow(RangeError);
    expect(() =>
      session.applyTransaction({
        ...transaction,
        baseRevision: 1,
        operationId: '4',
        edits: [
          { fromByte: 0, toByte: 4, insertedText: '' },
          { fromByte: 1, toByte: 4, insertedText: '' },
        ],
      }),
    ).toThrow(RangeError);
    expect(session.captureSnapshot().text.read()).toBe('a中\r\nb');
    expect(session.status().revision).toBe(1);
  });
  it('treats returning to saved content as clean independently of revision and tree shape', () => {
    const session = new MarkdownDocumentSession('doc', 'abc');
    session.applyTransaction({
      baseRevision: 0,
      operationId: '1',
      historyGroupId: '1',
      edits: [{ fromByte: 1, toByte: 2, insertedText: 'X' }],
    });
    session.applyTransaction({
      baseRevision: 1,
      operationId: '2',
      historyGroupId: '2',
      edits: [{ fromByte: 1, toByte: 2, insertedText: 'b' }],
    });
    expect(session.status()).toMatchObject({ revision: 2, dirty: false, canUndo: true });
  });
});

describe('persistent UTF-8 text', () => {
  it('preserves literal U+FEFF at document, piece and window boundaries', () => {
    const source = '\uFEFF正文' + 'a'.repeat(16_384 - 9) + '\uFEFF尾部';
    const text = Utf8Text.from(source);
    expect(text.read()).toBe(source);
    expect([...text.chunks()].join('')).toBe(source);
    const boundary = utf8Length(source.slice(0, source.lastIndexOf('\uFEFF')));
    expect(text.readWindow(boundary, text.byteLength).text).toBe('\uFEFF尾部');
    expect(() => Utf8Text.from('非法\uD800')).toThrow('Unicode');
  });
  it('matches independent string edits across leaf boundaries and retains all old roots', () => {
    let expected = 'abc中😀\r\n'.repeat(8_000);
    let text = Utf8Text.from(expected);
    const original = text;
    const initial = expected;
    let seed = 2137;
    for (let i = 0; i < 250; i++) {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      const chars = Array.from(expected);
      let index = seed % chars.length;
      if (chars[index] === '\n' && chars[index - 1] === '\r') index--;
      const prefix = chars.slice(0, index).join('');
      const remove = chars[index] === '\r' ? 2 : 1;
      const old = chars.slice(index, index + remove).join('');
      const replacement = i % 3 === 0 ? '新😀\r\n' : 'Q';
      text = text.replace(utf8Length(prefix), utf8Length(prefix + old), replacement);
      expected = prefix + replacement + chars.slice(index + remove).join('');
      expect(text.read()).toBe(expected);
      expect(text.equals(Utf8Text.from(expected))).toBe(true);
    }
    expect(original.read()).toBe(initial);
    expect([...text.chunks()].join('')).toBe(expected);
  });
});

describe('mount policy', () => {
  const complete = {
    revision: 1,
    state: 'ready' as const,
    indexedBytes: 100,
    totalBytes: 100,
    layoutUnits: 10,
    enhancementObjects: 0,
    enhancements: emptyEnhancements(),
  };
  it('keeps unknown structure windowed and classifies small complex documents', () => {
    const policy = new MarkdownMountPolicy();
    expect(policy.update({ ...complete, state: 'partial' }, 0, false)).toBe('windowed');
    expect(policy.windowLimited).toBe(false);
    expect(policy.update(complete, 0, false)).toBe('progressive-full');
    expect(policy.update({ ...complete, layoutUnits: 6_017 }, 1, false)).toBe('windowed');
    expect(policy.windowLimited).toBe(true);
    expect(policy.update(complete, 100, false)).toBe('windowed');
    expect(policy.update(complete, 2_099, false)).toBe('windowed');
    expect(policy.windowLimited).toBe(true);
    expect(policy.update(complete, 2_100, false)).toBe('progressive-full');
    expect(policy.windowLimited).toBe(false);
  });
  it('requires all lower thresholds and an uninterrupted idle interval', () => {
    const policy = new MarkdownMountPolicy();
    policy.update({ ...complete, layoutUnits: 6_001 }, 0, false);
    expect(policy.update({ ...complete, layoutUnits: 5_000 }, 10_000, false)).toBe('windowed');
    policy.update(complete, 11_000, false);
    policy.update(complete, 12_000, true);
    expect(policy.update(complete, 13_000, false)).toBe('windowed');
    expect(policy.update(complete, 15_000, false)).toBe('progressive-full');
  });
  it.each([
    ['totalBytes', 1_048_576, 838_860],
    ['layoutUnits', 6_000, 4_800],
  ] as const)('uses strict upper and inclusive lower %s boundaries', (metric, upper, lower) => {
    const policy = new MarkdownMountPolicy();
    expect(policy.update({ ...complete, [metric]: upper }, 0, false)).toBe('progressive-full');
    expect(policy.update({ ...complete, [metric]: upper + 1 }, 1, false)).toBe('windowed');
    expect(policy.update({ ...complete, [metric]: lower + 1 }, 3_000, false)).toBe('windowed');
    expect(policy.update({ ...complete, [metric]: lower }, 4_000, false)).toBe('windowed');
    expect(policy.update({ ...complete, [metric]: lower }, 5_999, false)).toBe('windowed');
    expect(policy.update({ ...complete, [metric]: lower }, 6_000, false)).toBe('progressive-full');
  });
  it('does not infer complete complexity from a ready index with unknown giant-block metrics', () => {
    const policy = new MarkdownMountPolicy();
    expect(policy.update({ ...complete, metricsComplete: false }, 0, false)).toBe('windowed');
  });
  it('轻量增强数量不否决准入，资源压力独立限制且不污染规模滞回', () => {
    const policy = new MarkdownMountPolicy();
    const rich = { ...complete, enhancementObjects: 5000, enhancements: { ...emptyEnhancements(), inlineCode: 5000 } };
    expect(policy.update(rich, 0, false)).toBe('progressive-full');
    expect(policy.update(rich, 1, false, true)).toBe('windowed');
    expect(policy.restriction).toBe('resource-pressure');
    expect(policy.update(rich, 3000, false, false)).toBe('progressive-full');
  });
});
