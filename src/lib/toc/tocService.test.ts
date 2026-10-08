import { describe, expect, it } from 'vitest';
import {
  createTocBlock,
  createTocList,
  extractTocItems,
  hasTocBlock,
  removeTocBlocks,
  updateTocBlocks,
} from './tocService';

describe('tocService', () => {
  it('creates nested Markdown links from headings', () => {
    expect(createTocList('# 标题1\n\n## 标题2\n\n### 标题3')).toBe(
      '- [标题1](#标题1)\n  - [标题2](#标题2)\n    - [标题3](#标题3)',
    );
  });

  it('uses stable ids for duplicate headings', () => {
    expect(createTocList('# Intro\n\n## Intro')).toBe('- [Intro](#intro)\n  - [Intro](#intro-2)');
  });

  it('uses plain text titles for headings with inline Markdown syntax', () => {
    expect(createTocList('# **项目概述**\n\n## `代码模块` 详解')).toBe(
      '- [项目概述](#项目概述)\n  - [代码模块 详解](#代码模块-详解)',
    );
  });

  it('keeps six-format title IDs consistent and escapes literal code contents in generated links', () => {
    const markdown = '# <u>重复</u>\n## <mark>重复</mark>\n# `**原样**`\n# \\*转义\\*';
    expect(extractTocItems(markdown).map(({ id, title }) => ({ id, title }))).toEqual([
      { id: '重复', title: '重复' }, { id: '重复-2', title: '重复' },
      { id: '原样', title: '**原样**' }, { id: '转义', title: '*转义*' },
    ]);
    expect(createTocList(markdown)).toBe(
      '- [重复](#重复)\n  - [重复](#重复-2)\n- [\\*\\*原样\\*\\*](#原样)\n- [\\*转义\\*](#转义)',
    );
  });

  it('skips headings inside fenced code blocks and existing toc blocks', () => {
    const markdown = [
      '<!-- toc -->',
      '- [旧标题](#旧标题)',
      '<!-- /toc -->',
      '',
      '# 正文标题',
      '',
      '```md',
      '# 代码标题',
      '```',
    ].join('\n');

    expect(extractTocItems(markdown).map((item) => item.title)).toEqual(['正文标题']);
  });

  it('updates existing toc block content', () => {
    const markdown = ['<!-- toc -->', '- [旧标题](#旧标题)', '<!-- /toc -->', '', '# 新标题'].join(
      '\n',
    );

    expect(updateTocBlocks(markdown)).toContain('- [新标题](#新标题)');
    expect(updateTocBlocks(markdown)).not.toContain('旧标题');
  });

  it('returns Markdown without toc markers unchanged', () => {
    const markdown = '# 标题\n\n正文\n\n```md\n# 代码标题\n```';

    expect(updateTocBlocks(markdown)).toBe(markdown);
  });

  it('only treats standalone marker lines outside fenced code as toc blocks', () => {
    const inlineCode = '`<!-- toc --><!-- /toc -->`';
    const inlineText = '正文 `<!-- toc --><!-- /toc -->` 后续';
    const paragraphText = '正文 <!-- toc --><!-- /toc --> 后续';
    const adjacentMarkers = '<!-- toc --><!-- /toc -->';
    const fencedSplitMarkers = ['```md', '<!-- toc -->', '<!-- /toc -->', '```', '', '# 标题'].join(
      '\n',
    );
    const fencedAdjacentMarkers = ['```', '<!-- toc --><!-- /toc -->', '```', '', '# 标题'].join(
      '\n',
    );

    expect(hasTocBlock(inlineCode)).toBe(false);
    expect(updateTocBlocks(inlineCode)).toBe(inlineCode);
    expect(updateTocBlocks(inlineText)).toBe(inlineText);
    expect(updateTocBlocks(paragraphText)).toBe(paragraphText);
    expect(updateTocBlocks(adjacentMarkers)).toBe(adjacentMarkers);
    expect(updateTocBlocks(fencedSplitMarkers)).toBe(fencedSplitMarkers);
    expect(updateTocBlocks(fencedAdjacentMarkers)).toBe(fencedAdjacentMarkers);
  });

  it('removes toc blocks without touching body content', () => {
    const markdown = `${createTocBlock('# 标题')}\n\n# 标题\n\n正文`;

    expect(removeTocBlocks(markdown).trim()).toBe('# 标题\n\n正文');
  });
});
