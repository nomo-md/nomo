import { describe, expect, it } from 'vitest';
import { EditorState, TextSelection } from 'prosemirror-state';
import { EditorView } from 'prosemirror-view';
import { parseMarkdown, serializeMarkdown } from './markdown';
import { schema } from './schema';
import { inlineSourceEditingPlugin } from './plugins/inlineSourceEditing';
import { projectInlineSource } from './InlineSourceCodec';
import { codeHighlightDecorationPlugin } from './plugins/codeHighlightDecorationPlugin';
import { inlineCodeSelectionBridgePlugin } from './plugins/inlineCodeSelectionBridge';

function hasCodeMark(node: ReturnType<typeof schema.node>): boolean {
  let found = false;
  projectInlineSource(node).descendants((n) => {
    if (n.isText && n.marks.some((m) => m.type.name === 'code')) {
      found = true;
      return false;
    }
    return true;
  });
  return found;
}

function getCodeTexts(node: ReturnType<typeof schema.node>): string[] {
  const texts: string[] = [];
  projectInlineSource(node).descendants((n) => {
    if (n.isText && n.marks.some((m) => m.type.name === 'code')) {
      texts.push(n.text ?? '');
    }
    return true;
  });
  return texts;
}

describe('code mark markdown-it parser', () => {
  it('parses `code` as code mark', () => {
    const doc = parseMarkdown('`const x = 1`');
    expect(hasCodeMark(doc)).toBe(true);
    expect(getCodeTexts(doc)).toEqual(['const x = 1']);
  });

  it('preserves code mark inside paragraph text', () => {
    const doc = parseMarkdown('这是 `const x = 1` 代码片段');
    expect(getCodeTexts(doc)).toEqual(['const x = 1']);
  });

  it('handles multiple code marks in one paragraph', () => {
    const doc = parseMarkdown('`foo` and `bar` and `baz`');
    expect(getCodeTexts(doc)).toEqual(['foo', 'bar', 'baz']);
  });

  it('handles code mark with spaces', () => {
    const doc = parseMarkdown('`const ok = true`');
    expect(getCodeTexts(doc)).toEqual(['const ok = true']);
  });

  it('handles code mark with punctuation', () => {
    const doc = parseMarkdown('`foo.bar()`');
    expect(getCodeTexts(doc)).toEqual(['foo.bar()']);
  });

  it('handles code mark with Chinese text', () => {
    const doc = parseMarkdown('`变量名`');
    expect(getCodeTexts(doc)).toEqual(['变量名']);
  });

  it('handles code mark with mixed symbols', () => {
    const doc = parseMarkdown('`a + b = c`');
    expect(getCodeTexts(doc)).toEqual(['a + b = c']);
  });
});

describe('code mark round-trip', () => {
  it('serializes code mark back to `code`', () => {
    const markdown = '`const x = 1`';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toBe(markdown);
  });

  it('round-trips code mark within paragraph', () => {
    const markdown = 'text `const x = 1` more';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toContain('`const x = 1`');
  });

  it('round-trips code mark with spaces', () => {
    const markdown = '`const ok = true`';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toBe(markdown);
  });

  it('round-trips code mark with Chinese text', () => {
    const markdown = '`变量名`';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toBe(markdown);
  });

  it('round-trips code mark with punctuation', () => {
    const markdown = '`foo.bar()`';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toBe(markdown);
  });

  it('handles code mark containing backticks with double backtick syntax', () => {
    const markdown = '`` code with ` backtick ``';
    expect(serializeMarkdown(parseMarkdown(markdown)).trim()).toBe(markdown);
  });
});

describe('code mark boundaries', () => {
  it('does NOT parse empty backticks as code mark', () => {
    const doc = parseMarkdown('``');
    expect(hasCodeMark(doc)).toBe(false);
  });

  it('does NOT parse unclosed backtick as code mark', () => {
    const doc = parseMarkdown('open ` but not closed');
    expect(hasCodeMark(doc)).toBe(false);
  });

  it('handles escaped backtick as literal', () => {
    const doc = parseMarkdown('costs \\`100');
    expect(hasCodeMark(doc)).toBe(false);
  });

  it('does NOT parse code mark inside code block', () => {
    const markdown = '```\n`code`\n```\n';
    const doc = parseMarkdown(markdown);
    expect(hasCodeMark(doc)).toBe(false);
  });
});

describe('code mark schema', () => {
  it('code mark has inline-code class in toDOM', () => {
    const mark = schema.marks.code;
    const dom = mark.spec.toDOM!(mark.create(), true);
    expect(dom).toEqual(['code', { class: 'inline-code' }, 0]);
  });
});

describe('code mark with math_inline coexistence', () => {
  it('parses both code mark and inline math in same paragraph', () => {
    const doc = parseMarkdown('code `x` and math $y$');
    const codeTexts = getCodeTexts(doc);
    const mathValues: string[] = [];
    doc.descendants((node) => {
      if (node.type.name === 'math_inline') {
        mathValues.push(node.attrs.tex as string);
      }
      return true;
    });
    expect(codeTexts).toEqual(['x']);
    expect(mathValues).toEqual(['y']);
  });
});

describe('real inline code input', () => {
  it('derives formatted content from newly typed `code` text without removing delimiters', () => {
    let state = EditorState.create({
      doc: schema.node('doc', null, [schema.node('paragraph')]),
      plugins: [inlineSourceEditingPlugin()],
    });

    state = state.apply(state.tr.insertText('语义输入 `const x = 1`'));

    expect(getCodeTexts(state.doc)).toEqual(['const x = 1']);
  });

  it('derives formatted content from newly typed `code` with spaces without removing delimiters', () => {
    let state = EditorState.create({
      doc: schema.node('doc', null, [schema.node('paragraph')]),
      plugins: [inlineSourceEditingPlugin()],
    });

    state = state.apply(state.tr.insertText('语义输入 `const ok = true`'));

    expect(getCodeTexts(state.doc)).toEqual(['const ok = true']);
  });

  it('converts `a` without removing delimiters', () => {
    let state = EditorState.create({
      doc: schema.node('doc', null, [schema.node('paragraph')]),
      plugins: [inlineSourceEditingPlugin()],
    });

    state = state.apply(state.tr.insertText('`a`'));

    expect(getCodeTexts(state.doc)).toEqual(['a']);
  });

  it('converts double backtick code without removing delimiters', () => {
    let state = EditorState.create({
      doc: schema.node('doc', null, [schema.node('paragraph')]),
      plugins: [inlineSourceEditingPlugin()],
    });

    state = state.apply(state.tr.insertText('语义输入 `` code with ` backtick ``'));

    expect(getCodeTexts(state.doc)).toEqual(['code with ` backtick']);
  });
});

describe('code mark syntax highlight decoration', () => {
  it('creates decoration for code mark tokens', () => {
    const doc = schema.node('doc', null, [
      schema.node('paragraph', null, [
        schema.text('const x = 1', [schema.marks.code.create()]),
      ]),
    ]);

    const plugin = codeHighlightDecorationPlugin();
    const state = EditorState.create({ doc, plugins: [plugin] });
    const decorations = plugin.getState(state) as import('prosemirror-view').DecorationSet;

    expect(decorations).toBeDefined();
  });
});

describe('code mark selection bridge decoration', () => {
  it('bridges selection from ordinary text into inline code', () => {
    const { target, view } = createInlineCodeSelectionView(3, 8);

    expect(hasInlineSelectionBridge(target)).toBe(true);
    expect(hasInlineCodeSelectionBridge(target)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('bridges selection from inline code into ordinary text', () => {
    const { target, view } = createInlineCodeSelectionView(8, 17);

    expect(hasInlineSelectionBridge(target)).toBe(true);
    expect(hasInlineCodeSelectionBridge(target)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('bridges selection that fully covers inline code', () => {
    const { target, view } = createInlineCodeSelectionView(5, 15);

    expect(hasInlineSelectionBridge(target)).toBe(true);
    expect(hasInlineCodeSelectionBridge(target)).toBe(true);

    view.destroy();
    target.remove();
  });

  it('does not bridge a collapsed cursor inside inline code', () => {
    const { target, view } = createInlineCodeSelectionView(8, 8);

    expect(hasInlineSelectionBridge(target)).toBe(false);
    expect(hasInlineCodeSelectionBridge(target)).toBe(false);

    view.destroy();
    target.remove();
  });

  it('keeps inline-code bridge limited to the actually selected characters', () => {
    const { target, view } = createInlineCodeSelectionView(7, 10);

    expect(hasInlineSelectionBridge(target)).toBe(true);
    expect(hasInlineCodeSelectionBridge(target)).toBe(true);
    expect(getInlineCodeSelectionBridgeText(target)).toBe('_lv');

    view.destroy();
    target.remove();
  });
});

function createInlineCodeSelectionView(
  selectionFrom: number,
  selectionTo: number,
): { target: HTMLElement; view: EditorView } {
  const doc = schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, [
      schema.text('old '),
      schema.text('vm_lvm_log', [schema.marks.code.create()]),
      schema.text(' new'),
    ]),
  ]);
  const target = document.createElement('div');
  document.body.appendChild(target);

  const view = new EditorView(target, {
    state: EditorState.create({
      doc,
      selection: TextSelection.create(doc, selectionFrom, selectionTo),
      plugins: [inlineCodeSelectionBridgePlugin()],
    }),
  });

  return { target, view };
}

function hasInlineCodeSelectionBridge(target: HTMLElement): boolean {
  return Boolean(target.querySelector('.pm-inline-code-selection-bridge'));
}

function hasInlineSelectionBridge(target: HTMLElement): boolean {
  return Boolean(target.querySelector('.pm-inline-selection-bridge'));
}

function getInlineCodeSelectionBridgeText(target: HTMLElement): string {
  return Array.from(target.querySelectorAll('.pm-inline-code-selection-bridge'))
    .map((node) => node.textContent ?? '')
    .join('');
}
