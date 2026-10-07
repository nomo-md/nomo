import { afterEach, describe, expect, it, vi } from 'vitest';
import { parseMarkdown } from '../editor-core/markdown';
import * as stats from './writingStats';
import * as serialization from '../editor-core/markdownSerialization';
import { createWritingStatsEngine } from './writingStatsEngine';

vi.mock('./writingStats', async (load) => {
  const actual = await load<typeof import('./writingStats')>();
  return {
    ...actual,
    buildSourceWritingStats: vi.fn(actual.buildSourceWritingStats),
    buildSemanticWritingStats: vi.fn(actual.buildSemanticWritingStats),
  };
});
vi.mock('../editor-core/markdownSerialization', async (load) => {
  const actual = await load<typeof import('../editor-core/markdownSerialization')>();
  return {
    ...actual,
    serializeMarkdownSelection: vi.fn(actual.serializeMarkdownSelection),
    measureMarkdownSelection: vi.fn(actual.measureMarkdownSelection),
  };
});
afterEach(() => vi.clearAllMocks());
const context = {
  documentId: 'a',
  contentRevision: 1,
  selectionRevision: 1,
  mode: 'source' as const,
};

describe('Worker writing statistics cache', () => {
  it('maps CodeMirror LF ranges back to CRLF source, including select-all and newline lengths', () => {
    const engine = createWritingStatsEngine();
    const markdown = '# 标题\r\n\r\nhello 中文';
    const normalized = markdown.replace(/\r\n/g, '\n');
    const full = engine({ requestId: 1, context, snapshot: { markdown }, selection: null }).full;
    const from = normalized.indexOf('hello');
    const query = (from: number, to: number) =>
      engine({ requestId: 2, context, selection: { from, to, sourceCoordinates: 'normalized' } })
        .selected;
    expect(query(from, from + 5)).toMatchObject({ words: 1, visibleChars: 5, chars: 5 });
    expect(query(from + 5, from)).toEqual(query(from, from + 5));
    expect(query(0, normalized.length)).toEqual(full);
    const newline = normalized.indexOf('\n');
    expect(query(newline, newline + 1)).toMatchObject({
      words: 0,
      visibleChars: 0,
      chars: 2,
      lines: 2,
    });
  });
  it('adds semantic positions after a mode switch without reparsing the source', () => {
    const engine = createWritingStatsEngine();
    const markdown = '中文 hello';
    engine({ requestId: 1, context, snapshot: { markdown }, selection: null });
    const response = engine({
      requestId: 2,
      context: { ...context, mode: 'semantic' },
      snapshot: { semanticDoc: JSON.stringify(parseMarkdown(markdown).toJSON()) },
      selection: { from: 1, to: 2 },
    });
    expect(response.selected?.words).toBe(1);
    expect(stats.buildSourceWritingStats).toHaveBeenCalledOnce();
    expect(stats.buildSemanticWritingStats).toHaveBeenCalledOnce();
  });
  it('reuses both indexes for range queries and never serializes source selections', () => {
    const engine = createWritingStatsEngine();
    const markdown = '# 中文\n\nhel**lo**\n\n```\n排除代码\n```';
    const doc = parseMarkdown(markdown);
    engine({
      requestId: 1,
      context,
      snapshot: { markdown, semanticDoc: doc.toJSON() },
      selection: null,
    });
    for (let i = 2; i < 30; i++) {
      const response = engine({ requestId: i, context, selection: { from: 7, to: 10 } });
      expect(response.error).toBeUndefined();
    }
    expect(stats.buildSourceWritingStats).toHaveBeenCalledOnce();
    expect(stats.buildSemanticWritingStats).toHaveBeenCalledOnce();
    expect(serialization.serializeMarkdownSelection).not.toHaveBeenCalled();
    expect(serialization.measureMarkdownSelection).not.toHaveBeenCalled();
  });
  it.each(['README.md', 'sample.md'])(
    'matches legacy Markdown selection serialization in %s',
    async (file) => {
      const { readFileSync } = await import('node:fs');
      const doc = parseMarkdown(readFileSync(file, 'utf8'));
      for (let i = 0; i < 20; i++) {
        const from = Math.max(1, Math.floor((doc.content.size * i) / 40));
        const to = Math.min(doc.content.size - 1, from + Math.floor(doc.content.size / 2));
        const markdown = serialization.serializeMarkdownSelection(doc, from, to);
        expect(serialization.measureMarkdownSelection(doc, from, to)).toEqual(
          markdown === null
            ? null
            : {
                chars: markdown.length,
                lines: markdown.split(/\r\n|\r|\n/).length,
              },
        );
      }
    },
  );
  it('counts partial semantic words across marks and retains Markdown selection character semantics', () => {
    const engine = createWritingStatsEngine();
    const markdown = 'hel**lo**\n\n```\n不计入\n```';
    const doc = parseMarkdown(markdown);
    const semantic = { ...context, mode: 'semantic' as const };
    engine({
      requestId: 1,
      context: semantic,
      snapshot: { markdown, semanticDoc: doc.toJSON() },
      selection: null,
    });
    const response = engine({ requestId: 2, context: semantic, selection: { from: 2, to: 5 } });
    expect(response.selected).toMatchObject({ words: 1, visibleChars: 3, chars: 7 });
    let codePosition = 0;
    doc.descendants((node, pos) => {
      if (node.type.name === 'code_block') codePosition = pos + 1;
    });
    const code = engine({
      requestId: 3,
      context: semantic,
      selection: { from: codePosition, to: codePosition + 2 },
    });
    expect(code.selected).toMatchObject({ words: 0, visibleChars: 0 });
  });
  it('counts a partial table cell with plain-text character fallback in both directions', () => {
    const engine = createWritingStatsEngine();
    const markdown = '| 表头 |\n| --- |\n| 甲乙 |';
    const doc = parseMarkdown(markdown);
    const semantic = { ...context, mode: 'semantic' as const };
    engine({
      requestId: 1,
      context: semantic,
      snapshot: { markdown, semanticDoc: doc.toJSON() },
      selection: null,
    });
    let from = 0;
    doc.descendants((node, pos) => {
      if (node.text === '甲乙') from = pos;
    });
    const forward = engine({ requestId: 2, context: semantic, selection: { from, to: from + 1 } });
    const backward = engine({
      requestId: 3,
      context: semantic,
      selection: { from: from + 1, to: from },
    });
    expect(forward.selected).toMatchObject({ words: 1, visibleChars: 1, chars: 1 });
    expect(backward.selected).toEqual(forward.selected);
  });
});
