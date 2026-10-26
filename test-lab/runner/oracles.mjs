import assert from 'node:assert/strict';
export const inlineResults = {
  toggleBold: '**Alpha**', toggleItalic: '*Alpha*', toggleCode: '`Alpha`',
  toggleStrikethrough: '~~Alpha~~', toggleUnderline: '++Alpha++', toggleHighlight: '==Alpha==',
};
export const blockResults = {
  setHeading: '## Alpha beta', toggleBlockquote: '> Alpha beta', toggleBulletList: '- Alpha beta',
  toggleOrderedList: '1. Alpha beta', toggleTaskList: '- [ ] Alpha beta',
};
export function assertMarkdown(actual, expected, name) {
  assert.equal(actual.replaceAll('\r\n', '\n'), expected, name);
}
