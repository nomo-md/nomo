import type MarkdownIt from 'markdown-it';
import type Token from 'markdown-it/lib/token.mjs';
import tableRule from 'markdown-it/lib/rules_block/table.mjs';

export interface TextSourceSpan {
  text: string;
  from: number;
  to: number;
}

/** 只在统计 tokenizer 上记录原始位置；语法判定仍完全交给共享规则。 */
export function recordInlinePositions(markdown: MarkdownIt): void {
  // table 解码 \| 的时间早于 inline。直接记录单元格消费的源码范围，
  // 不能再用已经解码的 token.content 搜索原文（重复单元格也会错位）。
  markdown.block.ruler.at('table', (state, start, end, silent) => {
    const first = state.tokens.length;
    if (!tableRule(state, start, end, silent)) return false;
    if (silent) return true;
    let cells: TextSourceSpan[][] = [];
    let cell = 0;
    let line = start;
    for (const token of state.tokens.slice(first)) {
      if (token.type === 'tr_open' && token.map) {
        line = token.map[0];
        const from = state.bMarks[line] + state.tShift[line];
        const raw = state.src.slice(from, state.eMarks[line]);
        const left = raw.length - raw.trimStart().length;
        const text = raw.trim();
        cells = [[]];
        for (let i = 0; i < text.length; i++) {
          const column = from - state.bMarks[line] + left + i;
          if (text[i] === '|' && text[i - 1] !== '\\') { cells.push([]); continue; }
          if (text[i] === '\\' && text[i + 1] === '|') continue;
          cells.at(-1)!.push({ text: text[i], from: column - (text[i] === '|' ? 1 : 0), to: column + 1 });
        }
        if (!cells[0]?.length) cells.shift();
        if (!cells.at(-1)?.length) cells.pop();
        for (const spans of cells) {
          while (spans.length && /^\s$/.test(spans[0].text)) spans.shift();
          while (spans.length && /^\s$/.test(spans.at(-1)!.text)) spans.pop();
        }
        cell = 0;
      } else if (token.type === 'inline') {
        token.meta = { ...token.meta, statsTableLine: line, statsTableSpans: cells[cell++] ?? [] };
      }
    }
    return true;
  }, { alt: ['paragraph', 'reference'] });
  markdown.inline.ruler2.disable('fragments_join');
  markdown.core.ruler.disable('text_join');
  markdown.core.ruler.after('inline', 'stats_original_inline', (state) => {
    for (const token of state.tokens) {
      if (token.type === 'inline') token.meta = { ...token.meta, statsContent: token.content };
    }
  });
  markdown.inline.tokenize = (state) => {
    const pending: TextSourceSpan[] = [];
    const pushPending = state.pushPending;
    state.pushPending = function () {
      const token = pushPending.call(this);
      token.meta = { ...token.meta, statsSpans: pending.splice(0) };
      return token;
    };
    const rules = state.md.inline.ruler.getRules('');
    while (state.pos < state.posMax) {
      const from = state.pos;
      const oldPending = state.pending;
      const oldTokens = state.tokens.length;
      let accepted = false;
      if (state.level < (state.md.options as { maxNesting?: number }).maxNesting!) {
        for (const rule of rules) {
          if (rule(state, false)) {
            accepted = true;
            break;
          }
        }
      }
      if (!accepted) state.pending += state.src[state.pos++];
      if (state.pos <= from) throw new Error('Inline tokenizer did not advance');
      const added = state.pending.startsWith(oldPending)
        ? state.pending.slice(oldPending.length)
        : state.pending;
      if (added) pending.push({ text: added, from, to: state.pos });
      let textCursor = from;
      for (let i = oldTokens; i < state.tokens.length; i++) {
        const token = state.tokens[i];
        if (token.meta?.statsSpans) continue;
        let start = from;
        let end = state.pos;
        if (token.type === 'text' && token.content) {
          const found = state.src.indexOf(token.content, textCursor);
          if (found >= textCursor && found + token.content.length <= end) {
            start = found;
            end = found + token.content.length;
            textCursor = end;
          }
        }
        if (token.type === 'code_inline') {
          start += token.markup.length;
          end -= token.markup.length;
          // markdown-it removes one padding space when both ends are padded.
          if (state.src.slice(start, end).replace(/\n/g, ' ').length > token.content.length) {
            start++;
            end--;
          }
        }
        token.meta = { ...token.meta, statsSpans: [{ text: token.content, from: start, to: end }] };
      }
    }
    if (state.pending) state.pushPending();
    state.pushPending = pushPending;
  };
}

export function tokenSpans(token: Token): TextSourceSpan[] {
  const spans: TextSourceSpan[] = token.meta?.statsSpans ?? [];
  const original = spans.map((span) => span.text).join('');
  // Callout postprocessing may remove a prefix after inline parsing.
  const offset = original.indexOf(token.content);
  if (offset <= 0) return spans;
  let skip = offset;
  return spans.flatMap((span) => {
    if (skip >= span.text.length) {
      skip -= span.text.length;
      return [];
    }
    const next = { ...span, text: span.text.slice(skip), from: span.from + skip };
    skip = 0;
    return [next];
  });
}
