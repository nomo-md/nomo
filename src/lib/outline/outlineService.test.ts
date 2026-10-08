import { describe, expect, it } from 'vitest';
import { analyzeMarkdown, calculateDocumentStats, extractOutline, normalizeHeadingTitle } from './outlineService';

describe('outlineService', () => {
  it('extracts heading outline with stable ids and line numbers', () => {
    expect(extractOutline('# 标题\n\n## Child\n### Child')).toEqual([
      { id: '标题', level: 1, title: '标题', line: 1 },
      { id: 'child', level: 2, title: 'Child', line: 3 },
      { id: 'child-2', level: 3, title: 'Child', line: 4 },
    ]);
  });

  it('normalizes inline Markdown syntax in heading titles', () => {
    expect(
      extractOutline(
        [
          '## **项目概述**',
          '### __整体架构__',
          '#### `代码模块` 详解',
          '##### [总结](#summary)',
          '###### foo_bar_baz',
        ].join('\n'),
      ),
    ).toEqual([
      { id: '项目概述', level: 2, title: '项目概述', line: 1 },
      { id: '整体架构', level: 3, title: '整体架构', line: 2 },
      { id: '代码模块-详解', level: 4, title: '代码模块 详解', line: 3 },
      { id: '总结', level: 5, title: '总结', line: 4 },
      { id: 'foobarbaz', level: 6, title: 'foo_bar_baz', line: 5 },
    ]);
  });

  it('calculates basic writing stats', () => {
    const stats = calculateDocumentStats('# Title\n\nhello world\n\n```ts\nconst x = 1\n```');

    expect(stats).toMatchObject({
      chars: 43,
      words: 3,
      lines: 7,
      headings: 1,
      readingMinutes: 1,
    });
  });

  it('uses the shared six-format source semantics without reparsing code contents', () => {
    expect(normalizeHeadingTitle('**粗体** *斜体* ~~删除~~ <u>下划线</u> <mark>高亮</mark> `**原样**`'))
      .toBe('粗体 斜体 删除 下划线 高亮 **原样**');
    expect(normalizeHeadingTitle('`` `literal` [link](url) **原样** ``'))
      .toBe('`literal` [link](url) **原样**');
    expect(normalizeHeadingTitle('<code>**原样** [link](url)</code>')).toBe('**原样** [link](url)');
  });

  it('keeps incomplete syntax and escaped literals in heading labels', () => {
    expect(normalizeHeadingTitle('**未闭合 <u>未闭合')).toBe('**未闭合 <u>未闭合');
    expect(normalizeHeadingTitle('\\*字面星号\\* &amp; foo_bar_baz')).toBe('*字面星号* & foo_bar_baz');
  });

  it('keeps link labels and image alt text while using shared semantics for their contents', () => {
    expect(normalizeHeadingTitle('[**粗体** 与 `*字面*`](https://example.com/a_(b))'))
      .toBe('粗体 与 *字面*');
    expect(normalizeHeadingTitle('![**图像**](image.png "说明") 后续')).toBe('图像 后续');
    expect(normalizeHeadingTitle('`![图片](url)`')).toBe('![图片](url)');
  });

  it('analyzes outline and stats in one pass without changing results', () => {
    const markdown = '# Title\r\n\r\nhello world\r\n\r\n```ts\r\nconst x = 1\r\n```';
    const analysis = analyzeMarkdown(markdown);

    expect(analysis.outline).toEqual(extractOutline(markdown));
    expect(analysis.stats).toEqual(calculateDocumentStats(markdown));
    expect(analysis.stats).toMatchObject({
      lines: 7,
      headings: 1,
    });
  });
});
