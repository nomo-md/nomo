// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { EditorState } from 'prosemirror-state';
import { parseMarkdown } from '../editor-core/markdown';
import { MarkdownSourceProjection } from './sourceProjection';
import { MarkdownDocumentSession } from './session';

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
});
