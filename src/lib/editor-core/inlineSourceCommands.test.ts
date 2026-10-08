import { describe, expect, it } from 'vitest';
import { EditorState, TextSelection, type Command, type Transaction } from 'prosemirror-state';
import { history, redo, undo } from 'prosemirror-history';
import { schema } from './schema';
import { parseMarkdown, serializeMarkdown } from './markdown';
import { analyzeInlineSource, projectInlineSource } from './InlineSourceCodec';
import {
  inlineSourceEditingPlugin,
  getEmptyInlineSourceTemplates,
} from './plugins/inlineSourceEditing';
import {
  appendClearInlineSourceStyles,
  getActiveInlineSourceFormats,
  toggleInlineSourceFormat,
  type InlineSourceFormat,
} from './inlineSourceCommands';

function rawState(source: string, from: number, to = from): EditorState {
  const marks = schema.marks.inline_source ? [schema.marks.inline_source.create()] : [];
  const paragraph = schema.nodes.paragraph.create(
    null,
    source ? schema.text(source, marks) : undefined,
  );
  const doc = schema.nodes.doc.create(null, paragraph);
  return EditorState.create({
    doc,
    selection: TextSelection.create(doc, from + 1, to + 1),
    plugins: [history()],
  });
}

function run(state: EditorState, command: Command): EditorState {
  let next = state;
  expect(
    command(state, (tr) => {
      next = state.apply(tr);
    }),
  ).toBe(true);
  return next;
}

describe('real inline Markdown commands', () => {
  it.each<[InlineSourceFormat, string]>([
    ['strong', '**文字**'],
    ['em', '*文字*'],
    ['code', '`文字`'],
    ['strikethrough', '~~文字~~'],
    ['underline', '<u>文字</u>'],
    ['highlight', '<mark>文字</mark>'],
  ])('%s wraps real text rather than creating a semantic mark', (type, expected) => {
    const next = run(rawState('文字', 0, 2), toggleInlineSourceFormat(type));
    expect(next.doc.textContent).toBe(expected);
    expect(next.doc.firstChild?.firstChild?.marks.some((mark) => mark.type.name === type)).toBe(
      false,
    );
    expect(getActiveInlineSourceFormats(next).has(type)).toBe(true);
  });

  it.each<InlineSourceFormat>(['strong', 'em', 'code', 'strikethrough', 'underline', 'highlight'])(
    '%s inserts an explicitly tracked, editable empty template',
    (type) => {
      const state = rawState('前后', 1);
      let transaction: Transaction | undefined;
      expect(
        toggleInlineSourceFormat(type)(state, (tr) => {
          transaction = tr;
        }),
      ).toBe(true);
      const meta = transaction!.getMeta('inlineSourceTemplate');
      expect(meta.action).toBe('add');
      expect(meta.template.type).toBe(type);
      expect(meta.template.from).toBe(2);
      expect(transaction!.selection.from).toBe(meta.template.cursor);
      // Every raw character boundary remains a valid document position.
      for (let pos = meta.template.from; pos <= meta.template.to; pos++) {
        expect(transaction!.doc.resolve(pos).parent.type.name).toBe('paragraph');
      }
    },
  );

  it('removes the enclosing format without changing a nested format', () => {
    const next = run(rawState('**a *b* c**', 5), toggleInlineSourceFormat('strong'));
    expect(next.doc.textContent).toBe('a *b* c');
    expect(analyzeInlineSource(next.doc.firstChild!).spans.map((span) => span.type)).toEqual([
      'em',
    ]);
  });

  it('partially cancels only the selected content and leaves both remaining sides formatted', () => {
    const next = run(rawState('**abcd**', 3, 5), toggleInlineSourceFormat('strong'));
    expect(next.doc.textContent).toBe('<strong>a</strong>bc<strong>d</strong>');
    const spans = analyzeInlineSource(next.doc.firstChild!).spans;
    expect(spans.map((span) => next.doc.textContent.slice(span.from, span.to))).toEqual(['a', 'd']);
    expect(next.doc.textBetween(next.selection.from, next.selection.to)).toBe('bc');
  });

  it('uses HTML fallback when Markdown emphasis cannot surround the selected whitespace', () => {
    const next = run(rawState('a b', 1, 2), toggleInlineSourceFormat('strong'));
    expect(next.doc.textContent).toBe('a<strong> </strong>b');
  });

  it('chooses a longer code delimiter and padding for literal backticks', () => {
    const next = run(rawState('`x`', 0, 3), toggleInlineSourceFormat('code'));
    // An existing code span is already active and therefore toggles off.
    expect(next.doc.textContent).toBe('x');
    const added = run(rawState('x``y', 0, 4), toggleInlineSourceFormat('code'));
    expect(added.doc.textContent).toBe('```x``y```');
  });

  it('formats separately across text blocks and leaves code blocks untouched', () => {
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, schema.text('one')),
      schema.nodes.code_block.create(null, schema.text('raw')),
      schema.nodes.heading.create({ level: 2 }, schema.text('two')),
    ]);
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1, doc.content.size - 1),
    });
    const next = run(state, toggleInlineSourceFormat('strong'));
    expect(next.doc.child(0).textContent).toBe('**one**');
    expect(next.doc.child(1).textContent).toBe('raw');
    expect(next.doc.child(2).textContent).toBe('**two**');
  });

  it('keeps malformed and escaped source literal when clearing recognised styles', () => {
    const source = '**yes** \\*literal* **unfinished';
    const state = rawState(source, 0, source.length);
    const tr = state.tr;
    expect(appendClearInlineSourceStyles(state, tr)).toBe(true);
    expect(tr.doc.textContent).toBe('yes \\*literal* **unfinished');
  });

  it('clears all nested recognised formats in one transaction', () => {
    const state = rawState('**a *b* ~~c~~**', 0, 14);
    const tr = state.tr;
    expect(appendClearInlineSourceStyles(state, tr)).toBe(true);
    expect(tr.doc.textContent).toBe('a b c');
  });

  it('one command is one undo and redo step', () => {
    const initial = rawState('文字', 0, 2);
    const formatted = run(initial, toggleInlineSourceFormat('highlight'));
    const reverted = run(formatted, undo);
    expect(reverted.doc.eq(initial.doc)).toBe(true);
    expect(run(reverted, redo).doc.eq(formatted.doc)).toBe(true);
  });

  it('preserves links and inline atoms when wrapping a mixed selection', () => {
    const source = schema.marks.inline_source.create();
    const link = schema.marks.link.create({ href: 'https://example.com' });
    const math = schema.nodes.math_inline.create({ tex: 'x+1' });
    const doc = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('前', [source]),
        schema.text('链接', [source, link]),
        math,
        schema.text('后', [source]),
      ]),
    );
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1, doc.content.size - 1),
    });
    const next = run(state, toggleInlineSourceFormat('strong'));
    let preservedMath = false;
    let linkText = '';
    next.doc.descendants((node) => {
      if (node === math) preservedMath = true;
      if (node.isText && link.isInSet(node.marks)) linkText += node.text;
    });
    expect(preservedMath).toBe(true);
    expect(linkText).toBe('链接');
    expect(projectInlineSource(next.doc).textContent).toBe('前链接后');
  });

  it('retains nested italic semantics when partially removing bold', () => {
    const state = rawState('**a*bcde*f**', 5, 7);
    const next = run(state, toggleInlineSourceFormat('strong'));
    const projected = projectInlineSource(next.doc);
    let italic = '';
    let bold = '';
    projected.descendants((node) => {
      if (!node.isText) return;
      if (node.marks.some((mark) => mark.type.name === 'em')) italic += node.text;
      if (node.marks.some((mark) => mark.type.name === 'strong')) bold += node.text;
    });
    expect(projected.textContent).toBe('abcdef');
    expect(italic).toBe('bcde');
    expect(bold).toBe('abef');
  });

  it('round-trips formatting that starts inside a link and ends outside it', () => {
    const source = schema.marks.inline_source.create();
    const link = schema.marks.link.create({ href: 'https://example.com' });
    const doc = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('abc', [source, link]),
        schema.text('de', [source]),
      ]),
    );
    const state = EditorState.create({ doc, selection: TextSelection.create(doc, 2, 5) });
    const next = run(state, toggleInlineSourceFormat('strong'));
    const reopened = projectInlineSource(parseMarkdown(serializeMarkdown(next.doc)));
    let bold = '';
    let linked = '';
    reopened.descendants((node) => {
      if (!node.isText) return;
      if (node.marks.some((mark) => mark.type.name === 'strong')) bold += node.text;
      if (node.marks.some((mark) => mark.type.name === 'link')) linked += node.text;
    });
    expect(reopened.textContent).toBe('abcde');
    expect(bold).toBe('bcd');
    expect(linked).toBe('abc');
  });

  it('clears only selected portions of nested styles', () => {
    const state = rawState('***abcd***', 4, 6);
    const tr = state.tr;
    expect(appendClearInlineSourceStyles(state, tr)).toBe(true);
    const projected = projectInlineSource(tr.doc);
    let bold = '';
    let italic = '';
    projected.descendants((node) => {
      if (!node.isText) return;
      if (node.marks.some((mark) => mark.type.name === 'em')) italic += node.text;
      if (node.marks.some((mark) => mark.type.name === 'strong')) bold += node.text;
    });
    expect(projected.textContent).toBe('abcd');
    expect(bold).toBe('ad');
    expect(italic).toBe('ad');
  });

  it('toggles a tracked empty template off without treating hand-written symbols as templates', () => {
    const initial = rawState('', 0);
    const state = initial.reconfigure({ plugins: [inlineSourceEditingPlugin()] });
    const inserted = run(state, toggleInlineSourceFormat('strong'));
    expect(inserted.doc.textContent).toBe('****');
    expect(getEmptyInlineSourceTemplates(inserted)).toHaveLength(1);
    expect(getActiveInlineSourceFormats(inserted).has('strong')).toBe(true);
    const removed = run(inserted, toggleInlineSourceFormat('strong'));
    expect(removed.doc.textContent).toBe('');
    expect(getEmptyInlineSourceTemplates(removed)).toHaveLength(0);
    const handwritten = rawState('****', 2).reconfigure({ plugins: [inlineSourceEditingPlugin()] });
    expect(getEmptyInlineSourceTemplates(handwritten)).toHaveLength(0);
  });

  it('does not format inside code blocks', () => {
    for (const name of ['code_block']) {
      const doc = schema.nodes.doc.create(
        null,
        schema.nodes[name].create(null, schema.text('text')),
      );
      const state = EditorState.create({ doc, selection: TextSelection.create(doc, 1, 5) });
      expect(toggleInlineSourceFormat('strong')(state)).toBe(false);
    }
  });

  it('keeps the existing semantic-mark commands inside standalone HTML blocks', () => {
    const doc = schema.nodes.doc.create(
      null,
      schema.nodes.html_block.create(null, schema.text('text')),
    );
    const state = EditorState.create({ doc, selection: TextSelection.create(doc, 1, 5) });
    const next = run(state, toggleInlineSourceFormat('strong'));
    expect(next.doc.textContent).toBe('text');
    expect(next.doc.firstChild!.firstChild!.marks.some((mark) => mark.type.name === 'strong')).toBe(
      true,
    );
    expect(
      next.doc.firstChild!.firstChild!.marks.some((mark) => mark.type.name === 'inline_source'),
    ).toBe(false);
    expect(getActiveInlineSourceFormats(next).has('strong')).toBe(true);
  });

  it.each(['[abc](https://example.com)de', 'a$x$b', 'a![image](image.png)b'])(
    'keeps structured components stable when code formatting is saved and reopened: %s',
    (markdown) => {
      const doc = parseMarkdown(markdown);
      const state = EditorState.create({
        doc,
        selection: TextSelection.create(doc, 1, doc.content.size - 1),
        plugins: [history()],
      });
      const next = run(state, toggleInlineSourceFormat('code'));
      const direct = projectInlineSource(next.doc);
      const reopened = projectInlineSource(parseMarkdown(serializeMarkdown(next.doc)));
      expect(reopened.toJSON()).toEqual(direct.toJSON());
      expect(direct.textContent).toBe(projectInlineSource(doc).textContent);
      const cleared = run(next, toggleInlineSourceFormat('code'));
      expect(projectInlineSource(cleared.doc).eq(projectInlineSource(doc))).toBe(true);
      expect(run(next, undo).doc.eq(doc)).toBe(true);
    },
  );

  it('places code fences inside a link when only its label is selected', () => {
    const doc = parseMarkdown('[abc](https://example.com)de');
    const state = EditorState.create({ doc, selection: TextSelection.create(doc, 1, 4) });
    const next = run(state, toggleInlineSourceFormat('code'));
    expect(serializeMarkdown(next.doc)).toBe('[`abc`](https://example.com)de');
    expect(
      projectInlineSource(parseMarkdown(serializeMarkdown(next.doc))).eq(
        projectInlineSource(next.doc),
      ),
    ).toBe(true);
  });

  it('preserves literal entities and a closing code tag across linked code segments', () => {
    const source = schema.marks.inline_source.create();
    const link = schema.marks.link.create({ href: 'https://example.com' });
    const doc = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('x</code>&amp;', [link, source]),
        schema.text(' next', [source]),
      ]),
    );
    const state = EditorState.create({
      doc,
      selection: TextSelection.create(doc, 1, doc.content.size - 1),
    });
    const next = run(state, toggleInlineSourceFormat('code'));
    const direct = projectInlineSource(next.doc);
    expect(direct.textContent).toBe('x</code>&amp; next');
    expect(projectInlineSource(parseMarkdown(serializeMarkdown(next.doc))).toJSON()).toEqual(
      direct.toJSON(),
    );
  });

  it('turning off an empty outer template preserves a different empty inner template', () => {
    const state = rawState('', 0).reconfigure({ plugins: [inlineSourceEditingPlugin()] });
    const bold = run(state, toggleInlineSourceFormat('strong'));
    const italic = run(bold, toggleInlineSourceFormat('em'));
    expect(getEmptyInlineSourceTemplates(italic)).toHaveLength(2);
    const withoutBold = run(italic, toggleInlineSourceFormat('strong'));
    expect(withoutBold.doc.textContent).toBe('**');
    expect(getEmptyInlineSourceTemplates(withoutBold).map((template) => template.type)).toEqual([
      'em',
    ]);
    expect(withoutBold.selection.from).toBe(2);
  });

  it('clearing selections inside delimiter characters stays in bounds and keeps all content', () => {
    for (const source of [
      '**abcd**',
      '***abcd***',
      '<u>ab</u>',
      '<code>ab</code>',
      '**a *b* c**',
    ]) {
      for (let from = 0; from <= source.length; from++) {
        for (let to = from; to <= source.length; to++) {
          const state = rawState(source, from, to);
          const tr = state.tr;
          appendClearInlineSourceStyles(state, tr);
          expect(projectInlineSource(tr.doc).textContent, `${source}: ${from}..${to}`).toBe(
            projectInlineSource(state.doc).textContent,
          );
          expect(tr.selection.from).toBeGreaterThanOrEqual(1);
          expect(tr.selection.to).toBeLessThan(tr.doc.content.size);
        }
      }
    }
  });
});
