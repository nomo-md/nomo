// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { EditorState, Selection } from 'prosemirror-state';
import { parseMarkdown } from '../editor-core/markdown';
import { MarkdownSourceProjection } from './sourceProjection';
import { MarkdownDocumentSession } from './session';
import { ensureTrailingParagraph } from '../editor-core/plugins/trailingParagraph';

it('脚注后的编辑器空尾段定位到真实 EOF，不包含临时标记引入的空行', () => {
  const source = '[^note]: 跨窗口脚注内容。\n';
  const doc = ensureTrailingParagraph(parseMarkdown(source)).doc;
  const projection = new MarkdownSourceProjection(source, doc, 1000);
  expect(projection.positionToByte(Selection.atEnd(doc).head)).toBe(1000 + new TextEncoder().encode(source).length);
});

function replaceText(source: string, old: string, replacement: string) {
  const doc = parseMarkdown(source);
  let pos: number | undefined;
  doc.descendants((node, offset) => {
    if (pos === undefined && node.isText && node.text?.includes(old))
      pos = offset + node.text.indexOf(old);
  });
  expect(pos).toBeDefined();
  const next = EditorState.create({ doc }).tr.insertText(replacement, pos!, pos! + old.length).doc;
  const projection = new MarkdownSourceProjection(source, doc);
  const result = projection.project(next);
  const session = new MarkdownDocumentSession('projection', source);
  const status = session.applyTransaction({
    baseRevision: 0,
    operationId: 'edit',
    historyGroupId: 'edit',
    edits: result.edits,
  });
  expect(session.captureSnapshot().text.read()).toBe(result.source);
  session.undo(status.revision);
  expect(session.captureSnapshot().text.read()).toBe(source);
  return result;
}

describe('语义窗口的源文写回', () => {
  it('GFM 裸链接保持源码范围，普通正文编辑不改写网址和邮箱', () => {
    const source='中文 www.example.com/a_(b) 和 foo+bar@example.com 尾部';
    const doc=parseMarkdown(source);
    const projection=new MarkdownSourceProjection(source,doc);
    expect(projection.project(doc).source).toBe(source);
    expect(replaceText(source,'尾部','结尾').source).toBe(source.replace('尾部','结尾'));
    const links:string[]=[];doc.descendants(node=>{if(node.isText && node.marks.some(mark=>mark.type.name==='link'))links.push(node.text!);});
    expect(links).toEqual(['www.example.com/a_(b)','foo+bar@example.com']);
  });
  it('Mermaid 后含下划线的普通文本可映射并恢复正反向选区端点', () => {
    const source = '# 图表\n\n```mermaid\nflowchart TD\n' + Array.from({ length: 399 }, (_, i) => `N${i}[步骤 ${i}] --> N${i + 1}[步骤 ${i + 1}]`).join('\n') + '\n```\n\n\n\nNOMO_END — 文件终点。\n';
    const doc = parseMarkdown(source), projection = new MarkdownSourceProjection(source, doc);
    let start = 0;
    doc.descendants((node, pos) => { if (node.isText && node.text?.startsWith('NOMO_END')) start = pos; });
    expect(start).toBeGreaterThan(0);
    for (const offset of [1, 6]) {
      const byte = new TextEncoder().encode(source.slice(0, source.indexOf('NOMO_END') + offset)).length;
      expect(projection.positionToByte(start + offset)).toBe(byte);
      expect(projection.byteToPosition(byte)).toBe(start + offset);
    }
  });
  it.each(['', '正文', '# 标题', '正文\r\n\r\n'])('空格输入可写回并继续中文输入，保留原有换行：%j', (source) => {
    const doc = parseMarkdown(source);
    const pos = 1 + doc.firstChild!.content.size;
    const next = EditorState.create({ doc }).tr.insertText(' ', pos).doc;
    const result = new MarkdownSourceProjection(source, doc).project(next);
    expect(result.source).toBe(source.replace(/(?=\r?\n|$)/, ' '));
    const continued = EditorState.create({ doc: next }).tr.insertText('你', pos + 1).doc;
    const following = new MarkdownSourceProjection(result.source, next).project(continued);
    expect(following.source).toBe(source ? source.replace(/(?=\r?\n|$)/, ' 你') : '&#32;你');
    const session = new MarkdownDocumentSession('whitespace', source);
    session.applyTransaction({ baseRevision: 0, operationId: 'space', historyGroupId: 'space', edits: result.edits });
    expect(session.captureSnapshot().text.read()).toBe(result.source);
    session.undo(1);
    expect(session.captureSnapshot().text.read()).toBe(source);
  });

  it.each([
    '# 标题\r\n\r\n正文  \r\n\r\n尾部\t\n\n\n',
    '+ 正文\r\n+ 第二项   \r\n    + 嵌套项\r\n\r\n尾部\n',
    '| 左 | 右 |\r\n| :--- | ---: |\r\n| 正文 | 不改  |\r\n\r\n尾部  \n',
    '> 正文\r\n> 引用第二行\r\n\r\n尾部\n',
    '[链接][id]\n\n正文\n\n[id]: <https://example.com> "原始标题"\n',
    '~~~js\r\n正文\r\n~~~\r\n\r\n尾部  \n',
  ])('输入只修改目标文字，保留其它原文：%s', (source) => {
    const result = replaceText(source, '正文', '新正文😀');
    expect(result.source).toBe(source.replace('正文', '新正文😀'));
    expect(result.edits.every((edit) => edit.fromByte === edit.toByte)).toBe(true);
  });
  it('无编辑不改写引用写法、混合换行或尾部空白', () => {
    const source = '[文字][x]\r\n\r\n[x]: <https://example.com>\n\n  \n';
    const doc = parseMarkdown(source);
    expect(new MarkdownSourceProjection(source, doc).project(doc)).toEqual({ source, edits: [] });
  });
  it('使用窗口外的引用定义解析与写回，不把引用展开为内联链接', () => {
    const source = '[原文字][末尾定义]\r\n\r\n';
    const context = {
      references: { 末尾定义: { href: 'https://example.com', title: '尾部标题' } },
    };
    const doc = parseMarkdown(source, context);
    expect(doc.firstChild?.firstChild?.marks.some((mark) => mark.type.name === 'link')).toBe(true);
    const next = EditorState.create({ doc }).tr.insertText('新', 1, 2).doc;
    const result = new MarkdownSourceProjection(source, doc, 200, context).project(next);
    expect(result.source).toBe(source.replace('原', '新'));
    expect(result.edits).toEqual([{ fromByte: 201, toByte: 204, insertedText: '新' }]);
  });
  it('全局 UTF-8 与语义光标在中文及 emoji 周围可以双向恢复', () => {
    const source = '前😀后\r\n\r\n第二段';
    const doc = parseMarkdown(source);
    const projection = new MarkdownSourceProjection(source, doc, 100);
    for (const [position, byte] of [
      [1, 100],
      [2, 103],
      [4, 107],
      [5, 110],
      [7, 114],
      [8, 117],
    ]) {
      expect(projection.positionToByte(position)).toBe(byte);
      const restored = projection.byteToPosition(byte);
      expect(restored).not.toBeNull();
      expect(projection.positionToByte(restored!)).toBe(byte);
    }
  });

  it.each([
    '前😀 **粗体** *斜体* ~~删除~~ `行内代码` [链接][x]\r\n\r\n[x]: https://example.com\n',
    '> 引用中文\r\n> 第二行\r\n\r\n- 列表\n  - 嵌套😀\n',
    '| 列一 | 列二 |\n| --- | --- |\n| 重复文字 | **重复文字** |\n| 中文😀 | a\\|b |\n',
    '~~~js\r\nconst 中文 = "😀";\r\n~~~\r\n\r\n尾部',
    '转义 \\*星号\\* &amp; &#32; 空格  \n下一行\n\n![图片](image.png)\n\n尾段',
    '> [!NOTE]\n> 提示正文\n\n<details>\n<summary>摘要</summary>\n\n里面正文\n\n</details>\n',
  ])('重新挂载后逐个恢复文本光标，保留格式、重复内容及 Unicode：%s', source => {
    const doc = parseMarkdown(source);
    const before = new MarkdownSourceProjection(source, doc, 123);
    const after = new MarkdownSourceProjection(source, parseMarkdown(source), 123);
    doc.descendants((node, start) => {
      if (!node.isText) return;
      for (let offset = 0; offset <= node.nodeSize; offset++) {
        if (offset && /[\uD800-\uDBFF]/.test(node.text![offset - 1]) && /[\uDC00-\uDFFF]/.test(node.text![offset] ?? '')) continue;
        const byte = before.positionToByte(start + offset);
        if (byte === null) continue;
        const restored = after.byteToPosition(byte);
        expect(restored, `source byte ${byte}, PM ${start + offset}`).not.toBeNull();
        expect(after.positionToByte(restored!)).toBe(byte);
      }
    });
  });

  it('深表格定位不再为每个候选反复投影，缓存只保留有界标量并支持窗口重定位', () => {
    const source = '| 列 | 内容 |\n| --- | --- |\n' + Array.from({ length: 400 }, (_, i) => `| ${i} | 正文😀${i} |\n`).join('');
    const projection = new MarkdownSourceProjection(source, parseMarkdown(source), 100);
    const project = vi.spyOn(projection, 'project');
    const byte = new TextEncoder().encode(source.slice(0, source.indexOf('正文😀398'))).length;
    const pos = projection.byteToPosition(100 + byte);
    expect(pos).not.toBeNull();
    expect(project).not.toHaveBeenCalled();
    expect(projection.positionToByte(pos!)).toBe(100 + byte);
    projection.fromByte = 500;
    expect(projection.byteToPosition(500 + byte)).toBe(pos);
    expect(projection.positionToByte(pos!)).toBe(500 + byte);
    for (let i = 0; i < 100; i++) projection.byteToPosition(500 + i);
    expect((projection as any).bytePositions.size).toBeLessThanOrEqual(32);
  });

  it('拒绝 Unicode、CRLF 和格式语法内部的无效落点，不修改原文', () => {
    const source = '前😀后\r\n\r\n**粗体** [文字](https://example.com)';
    const projection = new MarkdownSourceProjection(source, parseMarkdown(source));
    expect(projection.byteToPosition(4)).toBeNull();
    expect(projection.byteToPosition(11)).toBeNull();
    const offset = source.indexOf('example');
    expect(projection.byteToPosition(new TextEncoder().encode(source.slice(0, offset)).length)).toBeNull();
    expect(projection.project(projection.doc)).toEqual({ source, edits: [] });
  });
});
