// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { parseMarkdown, serializeMarkdown } from '../editor-core/markdown';
import { MarkdownStructureScanner, type MarkdownStructureBlock } from './structure';
import { Utf8Text } from './utf8Text';
import { addEnhancements, emptyEnhancements } from './enhancements';

function scan(source: string, chunkSize: number, limit?: number) {
  const scanner = new MarkdownStructureScanner(1, { maxSemanticBytes: limit });
  const blocks = new Map<string, MarkdownStructureBlock>();
  const chars = Array.from(source);
  let result;
  for (let index = 0; index < chars.length; index += chunkSize) {
    result = scanner.push(
      chars.slice(index, index + chunkSize).join(''),
      index + chunkSize >= chars.length,
    );
    for (const block of result.blocks) blocks.set(block.id, block);
  }
  if (!chars.length) result = scanner.push('', true);
  expect(result!.complete).toBe(true);
  const sorted = [...blocks.values()].sort((a, b) => a.fromByte - b.fromByte);
  const text = Utf8Text.from(source);
  let offset = 0;
  for (const block of sorted) {
    expect(block.fromByte).toBe(offset);
    offset = block.toByte;
  }
  expect(offset).toBe(text.byteLength);
  return { blocks: sorted, text, result: result! };
}

describe('结构边界渐进扫描', () => {
  it('增强按类型计数，Mermaid 不重复记为普通代码块', () => {
    const source = '`path` $x$ ![img](a.png)\n\n```ts\na()\n```\n\n```MeRmAiD\ngraph TD; A-->B\n```\n\n$$\ny\n$$\n';
    const { blocks } = scan(source, 17);
    const counts = emptyEnhancements();
    for (const block of blocks) addEnhancements(counts, block.enhancements);
    expect(counts).toEqual({ inlineCode: 1, inlineMath: 1, images: 1, codeBlocks: 1, diagrams: 1, blockMath: 1, htmlDetails: 0 });
    expect(blocks.reduce((sum, block) => sum + block.enhancementObjects, 0)).toBe(6);
  });
  it('Details 与 HTML 使用现有解析分类，未知巨型结构保持指标不完整', () => {
    const source = '<details>\n<summary>标题</summary>\n\n正文 `a`\n\n</details>\n\n<div>HTML</div>\n';
    const { blocks } = scan(source, 13);
    const counts = emptyEnhancements();
    for (const block of blocks) addEnhancements(counts, block.enhancements);
    expect(counts.htmlDetails).toBe(2);
    expect(counts.inlineCode).toBe(1);
    expect(blocks.some(block => !block.metricsComplete)).toBe(true);
    expect(scan('```\n' + 'x'.repeat(1000) + '\n```', 32, 128).blocks[0].metricsComplete).toBe(false);
  });
  const fixtures = [
    '开头\n#并非标题\n\n后文\n',
    '- 第一项\n\n  缩进续行\n  - 嵌套\n\n- 第二项\n\n# 结尾\n',
    '> 引用\n> - 嵌套\n>\n> 续行\n\n正文\n',
    'Setext 标题\n=====\n\n| a | b |\n|---|---|\n| c | d |\n\n尾部\n',
    '```md\n# 围栏内容\n\n- 列表样例\n```\n\n真实内容\n',
    '<details>\n<summary>标题</summary>\n\n# 内部标题\n\n- 项目\n\n</details>\n\n尾部\n',
    '<!-- toc -->\n- [原目录](#原目录)\n<!-- /toc -->\n\n# 新标题\n',
    '<div>\n<strong>HTML</strong>\n</div>\n\n正文\n',
    '# 中文\r\n\r\n正文😀\n\r\n末尾  \n',
  ];
  it.each(fixtures)('任意传输边界不截断语义结构：%s', (source) => {
    const expected = serializeMarkdown(parseMarkdown(source));
    for (const size of [1, 3, 17, 83]) {
      const { blocks, text } = scan(source, size);
      const docs = blocks.map((block) => parseMarkdown(text.read(block.fromByte, block.toByte)));
      const full = docs[0].type.create(
        docs[0].attrs,
        docs.flatMap((doc) =>
          Array.from({ length: doc.childCount }, (_, index) => doc.child(index)),
        ),
      );
      expect(serializeMarkdown(full)).toBe(expected);
    }
  });
  it('巨型围栏保持原文，闭合后继续语义扫描；内容覆盖不依赖解析成功', () => {
    const source = '```text\n' + '一行😀\n'.repeat(100) + '```\n\n# 后文\n\n可编辑\n';
    const { blocks } = scan(source, 13, 128);
    expect(blocks.some((block) => block.mode === 'raw' && !block.metricsComplete)).toBe(true);
    expect(blocks.at(-1)?.mode).toBe('semantic');
    expect(blocks.flatMap((block) => block.headings).map((heading) => heading.text)).toContain(
      '后文',
    );
  });
  it('只包含巨大单行时以原文区域覆盖全文，不把中间字符发布为新段落', () => {
    const { blocks } = scan('长😀'.repeat(1_000), 37, 128);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].mode).toBe('raw');
  });
  it('文末引用被记录为全局依赖，并且不篡改首屏原文', () => {
    const source = '[显示][id]\n\n正文\n\n[id]: /target "标题"\n';
    const { blocks, result } = scan(source, 7);
    expect(blocks.flatMap((block) => block.references)).toContain('ID');
    expect(result.references.ID).toMatchObject({ href: '/target', title: '标题' });
  });
});
