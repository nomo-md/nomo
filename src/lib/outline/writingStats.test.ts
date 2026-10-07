import { describe, expect, it } from 'vitest';
import { parseMarkdown } from '../editor-core/markdown';
import {
  buildSourceWritingStats,
  buildSemanticWritingStats,
  queryWritingStats,
  calculateWritingStats,
} from './writingStats';
import { createWritingStatsEngine } from './writingStatsEngine';
import { readFileSync } from 'node:fs';

describe('writing statistics', () => {
  it.each([
    ['中文 hello', 3],
    ["don't well-known foo_bar 3.14 2026", 5],
    ['hel**lo**', 1],
    ['中文Hello2026', 3],
    ['！？ 😀👨‍👩‍👧‍👦', 0],
    ['`中文 foo_bar`', 3],
    ['e\u0301', 1],
  ])('counts words in %s', (markdown, words) => {
    expect(calculateWritingStats(markdown).words).toBe(words);
  });
  it('counts graphemes and punctuation, excluding whitespace', () => {
    expect(calculateWritingStats('中 e\u0301，👨‍👩‍👧‍👦')).toMatchObject({ words: 2, visibleChars: 4 });
  });
  it.each(['- `[x]` 中文', '- **[x]** 中文', '- > [x] 中文', '- <div><code>[x]</code> 中文</div>'])(
    'keeps task markers distinct from inline code in %s',
    (markdown) => {
      const source = calculateWritingStats(markdown);
      const semantic = queryWritingStats(buildSemanticWritingStats(parseMarkdown(markdown)));
      expect(source.words).toBe(semantic.words);
      expect(source.visibleChars).toBe(semantic.visibleChars);
      if (markdown.includes('`[x]`') || markdown.includes('<code>')) expect(source.words).toBe(3);
    },
  );
  it('uses the same body in source and semantic modes', () => {
    const markdown =
      '# 中文 hello\n\n- [x] 项目 `code`\n\n> 引用\n\n| 表头 | 第二列 |\n| --- | --- |\n| 正文 | [链接](https://example.com) |\n\n[^id]: 脚注内容\n\n公式 $a$ 和图片 ![替代](img.png)\n\n```ts\n代码内容\n```\n\n~~~\n更多代码\n~~~\n\n    缩进代码\n\n$$\n公式\n$$\n\n```mermaid\n图表内容\n```\n\n<!-- 注释内容 -->';
    const source = calculateWritingStats(markdown);
    const semantic = queryWritingStats(buildSemanticWritingStats(parseMarkdown(markdown)));
    expect(semantic.words).toBe(source.words);
    expect(semantic.visibleChars).toBe(source.visibleChars);
    expect(calculateWritingStats('```\n中文\n```')).toMatchObject({ words: 0, visibleChars: 0 });
  });
  it('excludes front matter, automatic TOC and comments', () => {
    const source =
      '---\ntitle: 隐藏\n---\n\n正文\n\n<!-- toc -->\n- [隐藏](#a)\n<!-- /toc -->\n\n<!-- 隐藏 -->';
    expect(calculateWritingStats(source)).toMatchObject({
      words: 2,
      visibleChars: 2,
      chars: source.length,
    });
  });
  it('preserves block and link context for partial source selections', () => {
    const source = '[链接文字](https://example.com)\n\n```\n代码中文\n```\n\n`行内中文`';
    const index = buildSourceWritingStats(source);
    for (const text of ['example.com', '代码中文']) {
      const from = source.indexOf(text);
      expect(queryWritingStats(index, from, from + text.length).words).toBe(0);
    }
    const from = source.indexOf('行内中文');
    expect(queryWritingStats(index, from + 1, from + 3)).toMatchObject({
      words: 2,
      visibleChars: 2,
      chars: 2,
    });
    expect(queryWritingStats(index, from + 3, from + 1).words).toBe(2);
  });
  it('handles mark boundaries, partial English words and nested lists', () => {
    const source = '- 第一项\n  - hel**lo**\n';
    const index = buildSourceWritingStats(source);
    const from = source.indexOf('lo');
    expect(queryWritingStats(index, from, from + 1).words).toBe(1);
    const syntax = source.indexOf('**');
    expect(queryWritingStats(index, syntax, syntax + 2).visibleChars).toBe(0);
  });
  it('decodes HTML entities without a DOM and maps their source ranges', () => {
    const source = '<div><strong>中文</strong> hello &amp; e&#769;</div>';
    expect(calculateWritingStats(source)).toMatchObject({ words: 4, visibleChars: 9 });
    const index = buildSourceWritingStats(source);
    const from = source.indexOf('&amp;');
    expect(queryWritingStats(index, from, from + 5).visibleChars).toBe(1);
  });
  it('handles CRLF and repeated table cells', () => {
    const source = '| 中文 | 中文 |\r\n| --- | --- |\r\n| 中文 | 中文 |';
    const index = buildSourceWritingStats(source);
    expect(queryWritingStats(index)).toMatchObject({ words: 8, visibleChars: 8, lines: 3 });
    expect(queryWritingStats(index, source.lastIndexOf('中文'), source.length).words).toBe(2);
  });
  it.each([
    '<div>\r\n<strong>中文</strong> hello &amp;\r\n</div>',
    '<p>\r\n<a href="https://example.com"><img src="badge.png" /></a>\r\n</p>',
    '<p align="center">\r\n<img src="image.png" alt="图片替代" />\r\n</p>',
    '<img src="image.png" alt="图片替代" />',
    '> <div>\r\n> 中文 hello\r\n> </div>',
    '- <div>\r\n  中文 hello\r\n  </div>',
  ])('uses semantic HTML interpretation in CRLF source: %s', (markdown) => {
    const source = calculateWritingStats(markdown);
    const semantic = queryWritingStats(buildSemanticWritingStats(parseMarkdown(markdown)));
    expect(source.words).toBe(semantic.words);
    expect(source.visibleChars).toBe(semantic.visibleChars);
  });
  it.each([
    'foo**bar',
    '[x] 正文',
    '> 引用\n> > 内层',
    '> [!NOTE]\n> 中文 **正文**',
    '1. 1\n\n2. 2',
    '<p>中文</p>',
    '<span>中文</span>',
    '\\*中文\\*',
    'hello<br/>world',
    'hello <strong/>world',
    'hello </strong>world',
    'hello <a href="javascript:alert(1)">world</a>',
  ])('agrees with semantic body for %s', (markdown) => {
    const source = calculateWritingStats(markdown);
    const semantic = queryWritingStats(buildSemanticWritingStats(parseMarkdown(markdown)));
    expect(source.words).toBe(semantic.words);
    expect(source.visibleChars).toBe(semantic.visibleChars);
  });
  it('does not count list numbers or footnote identifiers as selected body', () => {
    const source = '1. 1\n\n[^id]: id';
    const index = buildSourceWritingStats(source);
    expect(queryWritingStats(index, 0, 1).visibleChars).toBe(0);
    expect(
      queryWritingStats(index, source.indexOf('id'), source.indexOf('id') + 2).visibleChars,
    ).toBe(0);
    expect(queryWritingStats(index, source.lastIndexOf('id'), source.length).words).toBe(1);
  });
  it('maps footnote content after CRLF lines to original source offsets', () => {
    const source = '# 标题\r\n\r\n[^id]: id';
    const index = buildSourceWritingStats(source);
    expect(queryWritingStats(index, source.indexOf('id'), source.indexOf('id') + 2).words).toBe(0);
    expect(queryWritingStats(index, source.lastIndexOf('id'), source.length).words).toBe(1);
  });
  it.each(['README.md', 'sample.md'])('indexes the complete %s fixture consistently', (path) => {
    const source = readFileSync(path, 'utf8');
    const indexed = calculateWritingStats(source);
    const semantic = queryWritingStats(buildSemanticWritingStats(parseMarkdown(source)));
    expect(indexed.chars).toBe(source.length);
    expect(indexed.words).toBe(semantic.words);
    expect(indexed.visibleChars).toBe(semantic.visibleChars);
  });
  it('reuses worker indexes and makes select-all equal the full source statistics', () => {
    const engine = createWritingStatsEngine();
    const markdown = '# 中文\n\n`hello`\n\n```\n不计入\n```';
    const doc = parseMarkdown(markdown);
    const context = {
      documentId: 'a',
      contentRevision: 1,
      selectionRevision: 1,
      mode: 'semantic' as const,
    };
    const first = engine({
      requestId: 1,
      context,
      snapshot: { markdown, semanticDoc: doc.toJSON() },
      selection: null,
    });
    const next = engine({ requestId: 2, context, selection: { from: 0, to: doc.content.size } });
    expect(first.error).toBeUndefined();
    expect(next.selected).toEqual(first.full);
    expect(
      engine({ requestId: 3, context: { ...context, contentRevision: 2 }, selection: null }).error,
    ).toBeTruthy();
  });
});
