import { describe, expect, it } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { inputRules } from 'prosemirror-inputrules';
import { createMarkdownInputRules, parseMarkdown, serializeMarkdown } from './markdown';
import { schema } from './schema';
import { isInlineSourceText, projectInlineSource } from './InlineSourceCodec';

describe('markdown serialization', () => {
  it('preserves blank paragraphs between non-paragraph blocks', () => {
    const input = '# 标题\n\n\n\n---\n\n\n\n## 下一节';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('preserves blank paragraphs around code and math blocks', () => {
    const input = [
      '```ts',
      'const value = 1;',
      '```',
      '',
      '',
      '$$',
      'E = mc^2',
      '$$',
      '',
      '',
      '<div>HTML</div>',
    ].join('\n');

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('preserves blank paragraphs between list items', () => {
    const input = '- 第一项\n\n\n\n- 第二项';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('preserves blank paragraphs between different list blocks', () => {
    const input = '- 第一项\n\n\n\n1. 第二项';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('renders source soft line breaks as in-paragraph breaks and preserves them on save', () => {
    const input = '1\n2\n3';
    const doc = parseMarkdown(input);
    const paragraph = doc.child(0);

    expect(doc.childCount).toBe(1);
    expect(paragraph.type.name).toBe('paragraph');
    expect(paragraph.child(1).type.name).toBe('hard_break');
    expect(paragraph.child(1).attrs.soft).toBe(true);
    expect(paragraph.child(3).type.name).toBe('hard_break');
    expect(paragraph.child(3).attrs.soft).toBe(true);
    expect(serializeMarkdown(doc)).toBe(input);
  });

  it('keeps Markdown hard breaks distinct from source soft line breaks', () => {
    const input = ['1\\', '2'].join('\n');
    const doc = parseMarkdown(input);
    const breakNode = doc.child(0).child(1);

    expect(breakNode.type.name).toBe('hard_break');
    expect(breakNode.attrs.soft).toBe(false);
    expect(serializeMarkdown(doc)).toBe(input);
  });

  it('does not escape manual inline mark trigger characters in plain text', () => {
    const serialized = serializeMarkdown(parseMarkdown('use * and ~ manually')).trim();

    expect(serialized).toBe('use * and ~ manually');
    expect(serialized).not.toContain('\\*');
    expect(serialized).not.toContain('\\~');
  });

  it('keeps incomplete inline source characters editable and unchanged', () => {
    const serialized = serializeMarkdown(parseMarkdown('use [x] and `code')).trim();

    expect(serialized).toBe('use [x] and `code');
  });

  it('serializes underline marks as u tags', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [schema.text('123', [schema.marks.underline.create()])]),
    ]);

    expect(serializeMarkdown(doc).trim()).toBe('<u>123</u>');
  });

  it('preserves u tags as source and projects underline semantics', () => {
    const doc = parseMarkdown('<u>123</u>');
    expect(doc.textContent).toBe('<u>123</u>');
    expect(isInlineSourceText(doc.firstChild!.firstChild!)).toBe(true);
    let hasUnderline = false;
    projectInlineSource(doc).descendants((node) => {
      if (!node.isText) return true;
      hasUnderline = node.marks.some((mark) => mark.type === schema.marks.underline);
      return !hasUnderline;
    });

    expect(hasUnderline).toBe(true);
  });

  it('serializes highlight marks as mark tags', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [schema.text('重点', [schema.marks.highlight.create()])]),
    ]);

    expect(serializeMarkdown(doc).trim()).toBe('<mark>重点</mark>');
  });

  it('preserves mark tags as source and projects highlight semantics', () => {
    const doc = parseMarkdown('<mark>重点</mark>');
    expect(doc.textContent).toBe('<mark>重点</mark>');
    expect(isInlineSourceText(doc.firstChild!.firstChild!)).toBe(true);
    let hasHighlight = false;
    projectInlineSource(doc).descendants((node) => {
      if (!node.isText) return true;
      hasHighlight = node.marks.some((mark) => mark.type === schema.marks.highlight);
      return !hasHighlight;
    });

    expect(hasHighlight).toBe(true);
    expect(serializeMarkdown(doc).trim()).toBe('<mark>重点</mark>');
  });

  it('preserves highlight marks inside table cells', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.table.create(null, [
        schema.nodes.table_row.create(null, [
          schema.nodes.table_header.create(null, [
            schema.nodes.paragraph.create(null, [
              schema.text('重点', [schema.marks.highlight.create()]),
            ]),
          ]),
        ]),
        schema.nodes.table_row.create(null, [
          schema.nodes.table_cell.create(null, [
            schema.nodes.paragraph.create(null, [
              schema.text('内容', [schema.marks.highlight.create()]),
            ]),
          ]),
        ]),
      ]),
    ]);

    expect(serializeMarkdown(doc).trim()).toBe(
      '| <mark>重点</mark> |\n| :--- |\n| <mark>内容</mark> |',
    );
  });

  it('round trips single-line footnotes', () => {
    const input = '正文内容[^1]\n\n[^1]: 脚注内容';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('preserves inline marks inside footnote definitions', () => {
    const input = '正文[^1]\n\n[^1]: **重点** [链接](https://example.com) `code`';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('round trips Markdown links with optional titles', () => {
    const plain = '[链接](https://example.com)';
    const titled = '[链接](https://example.com "说明")';

    expect(serializeMarkdown(parseMarkdown(plain)).trim()).toBe(plain);
    expect(serializeMarkdown(parseMarkdown(titled)).trim()).toBe(titled);
  });

  it('round trips Markdown images with alt and optional title', () => {
    const plain = '![截图](./assets/a.png)';
    const titled = '![截图](./assets/a.png "说明")';

    expect(serializeMarkdown(parseMarkdown(plain)).trim()).toBe(plain);
    expect(serializeMarkdown(parseMarkdown(titled)).trim()).toBe(titled);
  });

  it('allows common safe link targets', () => {
    const input = [
      '[相对](../docs/readme.md)',
      '[锚点](#标题)',
      '[邮件](mailto:user@example.com)',
    ].join(' ');

    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(
      '[相对](../docs/readme.md) [锚点](#%E6%A0%87%E9%A2%98) [邮件](mailto:user@example.com)',
    );
  });

  it('does not create link marks for dangerous link protocols', () => {
    const doc = parseMarkdown(
      '[危险](javascript:alert(1)) [data](data:text/html,<script></script>)',
    );
    let hasLink = false;

    doc.descendants((node) => {
      if (!node.isText) return true;
      hasLink = node.marks.some((mark) => mark.type === schema.marks.link);
      return !hasLink;
    });

    expect(hasLink).toBe(false);
  });

  it('preserves non-numeric footnote ids', () => {
    const input = '正文[^note]\n\n[^note]: 说明';

    expect(serializeMarkdown(parseMarkdown(input))).toBe(input);
  });

  it('does not parse normal brackets as footnotes', () => {
    const doc = parseMarkdown('正文[不是脚注] [^缺少结束');

    expect(doc.child(0).type.name).toBe('paragraph');
    expect(doc.child(0).textContent).toBe('正文[不是脚注] [^缺少结束');
  });

  it('parses block comments as dedicated comment_block nodes', () => {
    const input = '<!--\n这里是一段注释\n可以多行\n-->';
    const doc = parseMarkdown(input);

    expect(doc.child(0).type.name).toBe('comment_block');
    expect(doc.child(0).attrs.content).toBe('这里是一段注释\n可以多行');
    expect(serializeMarkdown(doc).trim()).toBe(input);
  });

  it('parses inline comments as dedicated comment_inline nodes', () => {
    const input = '这是正文 <!-- 这里是行内注释 --> 后面继续正文。';
    const doc = parseMarkdown(input);
    const paragraph = doc.child(0);

    expect(paragraph.type.name).toBe('paragraph');
    expect(paragraph.child(1).type.name).toBe('comment_inline');
    expect(paragraph.child(1).attrs.content).toBe('这里是行内注释');
    expect(serializeMarkdown(doc).trim()).toBe(input);
  });

  it('keeps an inline-only comment as comment_inline after reopening', () => {
    const doc = schema.node('doc', null, [
      schema.nodes.paragraph.create(null, [
        schema.nodes.comment_inline.create({ content: '这里是行内注释' }),
      ]),
    ]);
    const serialized = serializeMarkdown(doc).trim();
    const reopened = parseMarkdown(serialized);
    const paragraph = reopened.child(0);

    expect(serialized).toBe('<!-- 这里是行内注释 -->');
    expect(paragraph.type.name).toBe('paragraph');
    expect(paragraph.child(0).type.name).toBe('comment_inline');
    expect(paragraph.child(0).attrs.content).toBe('这里是行内注释');
  });

  it('serializes block comments in multiline form so they reopen as comment_block nodes', () => {
    const doc = schema.node('doc', null, [
      schema.nodes.comment_block.create({ content: '这里是一段注释' }),
    ]);
    const serialized = serializeMarkdown(doc).trim();
    const reopened = parseMarkdown(serialized);

    expect(serialized).toBe('<!--\n这里是一段注释\n-->');
    expect(reopened.child(0).type.name).toBe('comment_block');
    expect(reopened.child(0).attrs.content).toBe('这里是一段注释');
  });

  it('keeps HTML blocks separate from Markdown comments', () => {
    const doc = parseMarkdown('<section>HTML</section>\n\n<!--\n注释\n-->');

    expect(doc.child(0).type.name).toBe('html_block');
    expect(doc.child(1).type.name).toBe('comment_block');
    expect(serializeMarkdown(doc).trim()).toBe('<section>HTML</section>\n\n<!--\n注释\n-->');
  });

  it('still reserves toc comments for toc_block parsing', () => {
    const input = '<!-- toc -->\n- [标题](#标题)\n<!-- /toc -->\n\n# 标题';
    const doc = parseMarkdown(input);

    expect(doc.child(0).type.name).toBe('toc_block');
    expect(doc.child(0).attrs.content).toBe('- [标题](#标题)');
  });

  it('round trips toc marker examples as inline code instead of toc blocks', () => {
    const input = '`<!-- toc --><!-- /toc -->`';
    const doc = parseMarkdown(input);
    const paragraph = doc.child(0);

    expect(paragraph.type.name).toBe('paragraph');
    const firstChild = paragraph.child(0);
    expect(firstChild.isText).toBe(true);
    expect(isInlineSourceText(firstChild)).toBe(true);
    expect(projectInlineSource(doc).firstChild!.firstChild!.marks.some((m) => m.type === schema.marks.code)).toBe(true);
    expect(serializeMarkdown(doc).trim()).toBe(input);
  });

  it('keeps adjacent toc marker comments as ordinary text without corrupting comment endings', () => {
    const input = '正文 <!-- toc --><!-- /toc --> 后续';

    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(input);
    expect(serializeMarkdown(parseMarkdown('<!-- toc --><!-- /toc -->')).trim()).toBe(
      '<!-- toc --><!-- /toc -->',
    );
  });

  it('parses image with align and width attributes (legacy format)', () => {
    const input = '![demo](./assets/demo.png){align=center width=60%}';
    const doc = parseMarkdown(input);
    const img = doc.child(0).child(0);

    expect(img.type.name).toBe('image');
    expect(img.attrs.src).toBe('./assets/demo.png');
    expect(img.attrs.align).toBe('center');
    expect(img.attrs.width).toBe('60%');
  });

  it('serializes aligned image as <p> block HTML', () => {
    const input = '![demo](./assets/demo.png){align=center width=60%}';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(
      '<p align="center">\n  <img src="./assets/demo.png" alt="demo" width="60%">\n</p>',
    );
  });

  it('serializes width-only image as inline <img> tag', () => {
    const input = '![demo](./assets/demo.png){width=128}';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(
      '<img src="./assets/demo.png" alt="demo" width="128">',
    );
  });

  it('serializes plain image as standard Markdown', () => {
    const input = '![demo](./assets/demo.png)';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(input);
  });

  it('parses image with px width (legacy format)', () => {
    const input = '![pic](./a.png){align=left width=600}';
    const doc = parseMarkdown(input);
    const img = doc.child(0).child(0);

    expect(img.attrs.align).toBe('left');
    expect(img.attrs.width).toBe('600');
  });

  it('parses inline <img> HTML tag', () => {
    const input = '<img src="./a.png" alt="pic" width="200">';
    const doc = parseMarkdown(input);
    const img = doc.child(0).child(0);

    expect(img.type.name).toBe('image');
    expect(img.attrs.src).toBe('./a.png');
    expect(img.attrs.alt).toBe('pic');
    expect(img.attrs.width).toBe('200');
    expect(img.attrs.align).toBeNull();
  });

  it('parses <p align="center"><img ...></p> block HTML', () => {
    const input = '<p align="center">\n  <img src="./a.png" alt="pic" width="200">\n</p>';
    const doc = parseMarkdown(input);
    const img = doc.child(0).child(0);

    expect(img.type.name).toBe('image');
    expect(img.attrs.src).toBe('./a.png');
    expect(img.attrs.alt).toBe('pic');
    expect(img.attrs.width).toBe('200');
    expect(img.attrs.align).toBe('center');
  });

  it('round-trips <p align> block HTML', () => {
    const input = '<p align="right">\n  <img src="./a.png" alt="pic">\n</p>';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(input);
  });

  it('round-trips inline <img> with width', () => {
    const input = '<img src="./a.png" alt="pic" width="300">';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(input);
  });

  it('serializes image with title as standard Markdown', () => {
    const input = '![截图](./assets/a.png "说明")';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(input);
  });

  it('serializes image with title and width as inline <img>', () => {
    const input = '![截图](./assets/a.png "说明"){width=50%}';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(
      '<img src="./assets/a.png" alt="截图" title="说明" width="50%">',
    );
  });

  it('serializes image with title and align as <p> block', () => {
    const input = '![截图](./assets/a.png "说明"){align=left}';
    expect(serializeMarkdown(parseMarkdown(input)).trim()).toBe(
      '<p align="left">\n  <img src="./assets/a.png" alt="截图" title="说明">\n</p>',
    );
  });

  it('does not parse non-image HTML blocks as image', () => {
    const input = '<div>not an image</div>';
    const doc = parseMarkdown(input);
    expect(doc.child(0).type.name).toBe('html_block');
  });

  it('does not parse <p> with multiple children as image', () => {
    const input = '<p align="center"><img src="./a.png" alt="a"><span>extra</span></p>';
    const doc = parseMarkdown(input);
    // 不应被解析为 image 节点 — 内有额外元素时不转换
    let hasImage = false;
    doc.descendants((node) => {
      if (node.type.name === 'image') hasImage = true;
      return !hasImage;
    });
    expect(hasImage).toBe(false);
  });

  it('parses image without attributes', () => {
    const input = '![plain](./plain.png)';
    const doc = parseMarkdown(input);
    const img = doc.child(0).child(0);

    expect(img.attrs.src).toBe('./plain.png');
    expect(img.attrs.align).toBeNull();
    expect(img.attrs.width).toBeNull();
  });
});

describe('markdown input rules', () => {
  it('converts --- into a horizontal rule while typing', () => {
    const { target, view } = createInputRuleView();

    typeText(view, '---');

    expect(getTopLevelNodeNames(view.state.doc)).toContain('horizontal_rule');

    view.destroy();
    target.remove();
  });

  it('converts ___ into a horizontal rule while typing', () => {
    const { target, view } = createInputRuleView();

    typeText(view, '___');

    expect(getTopLevelNodeNames(view.state.doc)).toContain('horizontal_rule');

    view.destroy();
    target.remove();
  });

  it('does not convert *** into a horizontal rule', () => {
    const { target, view } = createInputRuleView();

    typeText(view, '***');

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['paragraph']);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('***');

    view.destroy();
    target.remove();
  });

  it('keeps **** as text so it can wrap a bold span', () => {
    const { target, view } = createInputRuleView();

    typeText(view, '****');

    expect(getTopLevelNodeNames(view.state.doc)).toEqual(['paragraph']);
    expect(view.state.doc.textBetween(0, view.state.doc.content.size)).toBe('****');

    view.destroy();
    target.remove();
  });
});

function createInputRuleView(): { target: HTMLElement; view: EditorView } {
  const doc = schema.nodes.doc.create(null, [schema.nodes.paragraph.create()]);
  const target = document.createElement('div');
  document.body.appendChild(target);
  const view = new EditorView(target, {
    state: EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1),
      plugins: [inputRules({ rules: createMarkdownInputRules() })],
    }),
  });
  return { target, view };
}

function typeText(view: EditorView, text: string): void {
  for (const char of text) {
    const { from, to } = view.state.selection;
    let handled = false;
    view.someProp('handleTextInput', (handler) => {
      handled = Boolean(
        handler(view, from, to, char, () => view.state.tr.insertText(char, from, to)),
      );
      return handled;
    });
    if (!handled) {
      view.dispatch(view.state.tr.insertText(char, from, to));
    }
  }
}

function getTopLevelNodeNames(doc: EditorState['doc']): string[] {
  const names: string[] = [];
  doc.forEach((node) => {
    names.push(node.type.name);
  });
  return names;
}
