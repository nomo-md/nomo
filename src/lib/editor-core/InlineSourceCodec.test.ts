import { describe, expect, it } from 'vitest';
import { schema } from './schema';
import { parseMarkdown, serializeMarkdown } from './markdown';
import {
  analyzeInlineSource,
  analyzeInlineSourceText,
  escapeInlineSourceLiteral,
  isInlineSourceText,
  materializeInlineSource,
  materializeInlineSourceWithMapping,
  projectInlineSource,
  projectInlineSourceWithMapping,
} from './InlineSourceCodec';

describe('inline source codec', () => {
  it.each([
    '**粗体**',
    '__粗体__',
    '*斜体*',
    '_斜体_',
    '***粗斜***',
    '~~删除~~',
    '<u>下划线</u>',
    '<mark>高亮</mark>',
    '<b>加粗</b> <i>斜体</i>',
    '`` a`b ``',
    '` a  b `',
    '`a\nb`',
    '\\*字\\*',
    '**尚未闭合',
  ])('keeps source characters for %s', (source) => {
    const doc = parseMarkdown(source);
    expect(doc.firstChild?.textContent).toBe(source);
    doc.firstChild?.forEach((child) => {
      if (child.isText) {
        expect(isInlineSourceText(child)).toBe(true);
        expect(
          child.marks.some((mark) =>
            ['strong', 'em', 'code', 'strikethrough', 'underline', 'highlight'].includes(
              mark.type.name,
            ),
          ),
        ).toBe(false);
      }
    });
    expect(serializeMarkdown(doc)).toBe(source);
  });

  it('recognizes nested exact delimiter ranges without consuming characters', () => {
    const analysis = analyzeInlineSourceText('***粗斜***');
    expect(analysis.spans).toEqual([
      { type: 'em', from: 1, to: 7, openFrom: 0, openTo: 1, closeFrom: 7, closeTo: 8 },
      { type: 'strong', from: 3, to: 5, openFrom: 1, openTo: 3, closeFrom: 5, closeTo: 7 },
    ]);
    expect(analysis.visibleText).toBe('粗斜');
    expect(analysis.visiblePositions).toEqual([3, 4]);
  });

  it('preserves escaped and unfinished source while projecting the visible text', () => {
    expect(analyzeInlineSourceText('\\*字\\*').spans).toEqual([]);
    expect(analyzeInlineSourceText('\\*字\\*').visibleText).toBe('*字*');
    expect(analyzeInlineSourceText('**尚未闭合').visibleText).toBe('**尚未闭合');
    expect(analyzeInlineSourceText('x &amp; y').visibleText).toBe('x & y');
  });

  it('keeps existing entity display and ordinary ampersands', () => {
    expect(parseMarkdown('A &amp; B').textContent).toBe('A & B');
    const doc = parseMarkdown('&#42;&#42;literal&#42;&#42;');
    expect(analyzeInlineSource(doc.firstChild!).spans).toEqual([]);
    expect(projectInlineSource(doc).textContent).toBe('**literal**');
    expect(escapeInlineSourceLiteral('A&B')).toBe('A&B');
    expect(analyzeInlineSourceText(escapeInlineSourceLiteral('&amp;')).visibleText).toBe('&amp;');
  });

  it('projects code whitespace using original source positions', () => {
    const analysis = analyzeInlineSourceText('`` a`b ``');
    expect(analysis.visibleText).toBe('a`b');
    expect(analysis.visiblePositions).toEqual([3, 4, 5]);
    expect(analyzeInlineSourceText('`a\nb`').visibleText).toBe('a b');
    expect(analyzeInlineSourceText('`   `').visibleText).toBe('   ');
    expect(analyzeInlineSourceText('` \n `').visibleText).toBe('   ');
  });

  it('treats complete HTML code content as literal code, including atom-like syntax', () => {
    const source = '<code>**x** $y$ [z](https://example.com) ![p](p.png) &#96;</code>';
    const doc = parseMarkdown(source);
    expect(doc.firstChild!.childCount).toBe(1);
    expect(doc.textContent).toBe(source);
    const analysis = analyzeInlineSource(doc.firstChild!);
    expect(analysis.spans.map((span) => span.type)).toEqual(['code']);
    expect(analysis.visibleText).toBe('**x** $y$ [z](https://example.com) ![p](p.png) `');
    expect(
      projectInlineSource(doc).firstChild!.firstChild!.marks.map((mark) => mark.type.name),
    ).toEqual(['code']);
    expect(serializeMarkdown(doc)).toBe(source);
  });

  it('retains links and inline atoms inside real format delimiters', () => {
    const doc = parseMarkdown('**a [b](https://example.com) $x$ ![p](p.png) c**');
    const block = doc.firstChild!;
    expect(block.textContent).toBe('**a b   c**');
    expect(block.child(1).marks.some((mark) => mark.type.name === 'link')).toBe(true);
    const atoms: unknown[] = [];
    block.forEach((child) => {
      if (!child.isText) atoms.push(child);
    });
    expect(atoms).toHaveLength(2);
    const projected = projectInlineSource(doc);
    const projectedAtoms: unknown[] = [];
    projected.firstChild!.forEach((child) => {
      if (!child.isText) projectedAtoms.push(child);
    });
    expect(projectedAtoms).toEqual(atoms);
    projectedAtoms.forEach((node, i) => expect(node).toBe(atoms[i]));
    expect(projected.firstChild!.textContent).toBe('a b   c');
  });

  it('retains footnote atoms whose parser rule writes its own metadata', () => {
    const doc = parseMarkdown('**正文[^1]继续**');
    expect(doc.firstChild!.child(1).type.name).toBe('footnote_ref');
    expect(doc.firstChild!.child(1).attrs.id).toBe('1');
    expect(projectInlineSource(doc).textContent).toBe('正文继续');
    expect(serializeMarkdown(doc)).toBe('**正文[^1]继续**');
  });

  it('keeps fallback HTML literal while retaining its original source', () => {
    const source = '<p><strong>字面 HTML</strong></p>';
    const doc = parseMarkdown(source);
    expect(analyzeInlineSource(doc.firstChild!).spans).toEqual([]);
    expect(projectInlineSource(doc).textContent).toBe(source);
    expect(serializeMarkdown(doc)).toBe(source);
  });

  it('maps a selected body through full-document projection', () => {
    const doc = parseMarkdown('**hello** tail');
    const projected = projectInlineSourceWithMapping(doc);
    expect(projected.doc.textContent).toBe('hello tail');
    expect(projected.mapPosition(3)).toBe(1);
    expect(projected.mapPosition(8)).toBe(6);
    expect(
      projected.doc.firstChild!.firstChild!.marks.some((mark) => mark.type.name === 'strong'),
    ).toBe(true);
  });

  it('materializes old marks once and retains code blocks', () => {
    const code = schema.nodes.code_block.create(null, schema.text('**code**'));
    const doc = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('old', [schema.marks.strong.create()]),
        schema.text(' * literal'),
      ]),
      code,
    ]);
    const converted = materializeInlineSource(doc);
    expect(converted.firstChild!.textContent).toBe('**old** \\* literal');
    expect(converted.child(1)).toBe(code);
    expect(materializeInlineSource(converted)).toBe(converted);
    expect(projectInlineSource(converted).firstChild!.textContent).toBe('old * literal');
  });

  it('materializes legacy punctuation formatting without violating Markdown delimiter rules', () => {
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('a'),
        schema.text('(', [schema.marks.strong.create()]),
        schema.text('b'),
      ]),
    );
    const converted = materializeInlineSource(original);
    expect(converted.textContent).toBe('a<strong>(</strong>b');
    expect(projectInlineSource(parseMarkdown(serializeMarkdown(converted))).eq(original)).toBe(
      true,
    );
  });

  it('preserves partial-link formatting when converting pasted semantic marks', () => {
    const link = schema.marks.link.create({ href: 'https://example.com', title: null });
    const strong = schema.marks.strong.create();
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('a', [link]),
        schema.text('b', [link, strong]),
        schema.text('c', [strong]),
      ]),
    );
    const converted = materializeInlineSource(original);
    const saved = serializeMarkdown(converted);
    expect(saved).toBe('[a](https://example.com)**[b](https://example.com)c**');
    expect(projectInlineSource(parseMarkdown(saved)).eq(original)).toBe(true);
  });

  it('keeps legacy code inside each link identity and does not swallow inline atoms', () => {
    const link = schema.marks.link.create({ href: 'https://example.com', title: null });
    const code = schema.marks.code.create();
    const math = schema.nodes.math_inline.create({ tex: 'x' });
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('plain', [code]),
        schema.text('linked', [link, code]),
        math,
        schema.text('tail', [code]),
      ]),
    );
    const converted = materializeInlineSource(original);
    const saved = serializeMarkdown(converted);
    expect(saved).toBe('`plain`[`linked`](https://example.com)$x$`tail`');
    expect(projectInlineSource(parseMarkdown(saved)).eq(original)).toBe(true);
    const atomWithCode = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [math.mark([code])]),
    );
    const clean = materializeInlineSource(atomWithCode);
    expect(serializeMarkdown(clean)).toBe('$x$');
    expect(clean.firstChild!.firstChild!.type.name).toBe('math_inline');
  });

  it('uses valid HTML delimiters for legacy strikethrough with edge whitespace', () => {
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text(' leading ', [schema.marks.strikethrough.create()]),
      ]),
    );
    const converted = materializeInlineSource(original);
    expect(converted.textContent).toBe('<del> leading </del>');
    expect(projectInlineSource(parseMarkdown(serializeMarkdown(converted))).eq(original)).toBe(
      true,
    );
  });

  it('maps old selections exactly through HTML fallback delimiter insertion', () => {
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, [
        schema.text('a'),
        schema.text('(', [schema.marks.strong.create()]),
        schema.text('b'),
      ]),
    );
    const result = materializeInlineSourceWithMapping(original);
    expect(result.doc.textContent).toBe('a<strong>(</strong>b');
    expect(result.mapPosition(2, -1)).toBe(2);
    expect(result.mapPosition(2, 1)).toBe(10);
    expect(result.mapPosition(3, -1)).toBe(11);
    expect(result.mapPosition(3, 1)).toBe(20);
    expect(result.doc.textBetween(result.mapPosition(2, 1), result.mapPosition(3, -1))).toBe('(');
    expect(result.mapPosition(original.content.size)).toBe(result.doc.content.size);
  });

  it('maps multiple blocks, link code runs, and atoms without source-text searches', () => {
    const link = schema.marks.link.create({ href: 'https://example.com', title: null });
    const code = schema.marks.code.create();
    const atom = schema.nodes.math_inline.create({ tex: 'x' });
    const original = schema.nodes.doc.create(null, [
      schema.nodes.paragraph.create(null, [
        schema.text('a', [link, code]),
        atom,
        schema.text('b', [code]),
      ]),
      schema.nodes.paragraph.create(null, schema.text('尾', [schema.marks.strong.create()])),
    ]);
    const result = materializeInlineSourceWithMapping(original);
    expect(result.mapPosition(1, 1)).toBe(2);
    expect(result.mapPosition(2, -1)).toBe(3);
    expect(result.mapPosition(2, 1)).toBe(4);
    expect(result.mapPosition(3, -1)).toBe(5);
    expect(result.mapPosition(3, 1)).toBe(6);
    expect(result.mapPosition(6, 1)).toBe(12);
    expect(result.mapPosition(7, -1)).toBe(13);
    expect(result.mapPosition(8)).toBe(16);
    expect(result.doc.nodeAt(result.mapPosition(2, 1))).toBe(atom);
    const projection = projectInlineSourceWithMapping(result.doc);
    for (let pos = 0; pos <= original.content.size; pos++) {
      expect(projection.mapPosition(result.mapPosition(pos, -1))).toBe(pos);
      expect(projection.mapPosition(result.mapPosition(pos, 1))).toBe(pos);
    }
  });

  it('maps literal escape prefixes and keeps already-source node identity', () => {
    const original = schema.nodes.doc.create(
      null,
      schema.nodes.paragraph.create(null, schema.text('* &amp;')),
    );
    const result = materializeInlineSourceWithMapping(original);
    expect(result.doc.textContent).toBe('\\* \\&amp;');
    expect(result.mapPosition(1, -1)).toBe(1);
    expect(result.mapPosition(1, 1)).toBe(2);
    expect(result.mapPosition(2, -1)).toBe(3);
    expect(result.mapPosition(3, 1)).toBe(5);
    const repeated = materializeInlineSourceWithMapping(result.doc);
    expect(repeated.doc).toBe(result.doc);
    for (let pos = 0; pos <= result.doc.content.size; pos++)
      expect(repeated.mapPosition(pos)).toBe(pos);
  });

  it('keeps source in table cells and callout bodies without changing their structure', () => {
    const doc = parseMarkdown(
      '| A | B |\n| --- | --- |\n| **粗** | `代码` |\n\n> [!NOTE]\n> **提醒**',
    );
    expect(doc.child(0).type.name).toBe('table');
    expect(doc.child(0).child(1).child(0).textContent).toBe('**粗**');
    expect(doc.child(1).type.name).toBe('callout');
    expect(doc.child(1).textContent).toBe('**提醒**');
    expect(analyzeInlineSource(doc.child(0).child(1).child(0).firstChild!).visibleText).toBe('粗');
  });

  it('escapes literal paste so syntax remains literal on display', () => {
    const literal = '**literal** `code` <u>x</u> &';
    expect(analyzeInlineSourceText(escapeInlineSourceLiteral(literal)).visibleText).toBe(literal);
    expect(analyzeInlineSourceText(escapeInlineSourceLiteral(literal)).spans).toEqual([]);
  });
});
