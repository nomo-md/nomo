import assert from 'node:assert/strict';
export const inlineResults = {
  toggleBold: '**Alpha**', toggleItalic: '*Alpha*', toggleCode: '`Alpha`',
  // 原生公开格式采用 markdown_templates.rs 的 HTML 标记；旧 WebView 使用 ++ / ==。
  toggleStrikethrough: '~~Alpha~~', toggleUnderline: '<u>Alpha</u>', toggleHighlight: '<mark>Alpha</mark>',
};
export const blockResults = {
  setHeading: '## Alpha beta', toggleBlockquote: '> Alpha beta', toggleBulletList: '- Alpha beta',
  toggleOrderedList: '1. Alpha beta', toggleTaskList: '- [ ] Alpha beta',
};
export function assertMarkdown(actual, expected, name) {
  assert.equal(actual.replaceAll('\r\n', '\n'), expected, name);
}
