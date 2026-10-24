import MarkdownIt from 'markdown-it';
import Token from 'markdown-it/lib/token.mjs';
import { unescapeAll } from 'markdown-it/lib/common/utils.mjs';
import { normalizeLinkHref } from './link';
import { transformCalloutTokens } from './callout/calloutParser';
import { registerDetailsRule } from './details/detailsSyntax';
import { registerGfmAutolinks } from './gfmAutolinks';

export const WRAPPED_IMAGE_HTML =
  /<p\s+align="(left|center|right)"\s*>\s*(<img\s+[^>]+(?:\/>|>))\s*<\/p>/gi;
export const STANDALONE_IMAGE_HTML = /^<img\s+[^>]+(?:\/>|>)\s*$/gim;

/** 与编辑器图片预处理共用匹配规则，避免把包含图片的普通 HTML 字面文本整块排除。 */
export function isRenderedImageHtml(content: string): boolean {
  const trimmed = content.trim();
  for (const match of trimmed.matchAll(WRAPPED_IMAGE_HTML)) {
    if (match[0] === trimmed && parseHtmlImgAttrs(match[2]).src) return true;
  }
  for (const match of trimmed.matchAll(STANDALONE_IMAGE_HTML)) {
    if (match[0] === trimmed && parseHtmlImgAttrs(match[0]).src) return true;
  }
  return false;
}

// ——— 图片 HTML / 属性解析工具 ———

/** HTML 属性值转义：& " < > */
export function escapeHtmlAttr(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\r/g, '&#13;')
    .replace(/\n/g, '&#10;');
}

/** 完整消费每个属性及其引号值，保留顺序供调用方选择重复属性语义。 */
export function parseHtmlAttributes(tagContent: string): Array<[string, string]> {
  const attrs: Array<[string, string]> = [];
  const tag = /^<\/?[a-zA-Z][\w:-]*/.exec(tagContent);
  const attrRegex = /\s*([^\s"'<>\/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>\x60]+)))?/y;
  attrRegex.lastIndex = tag?.[0].length ?? 0;
  let match: RegExpExecArray | null;
  while ((match = attrRegex.exec(tagContent)) !== null) {
    const value = match[2] ?? match[3] ?? match[4];
    if (value === undefined) continue;
    // HTML 属性不使用 Markdown 反斜杠转义，只解码一次字符实体。
    attrs.push([match[1].toLowerCase(), unescapeAll(value.replace(/\\/g, '\\\\'))]);
  }
  return attrs;
}

/** 从 <img ...> 标签内容中提取 src / alt / title / width */
export function parseHtmlImgAttrs(tagContent: string): {
  src: string | null;
  alt: string | null;
  title: string | null;
  width: string | null;
} {
  const result: {
    src: string | null;
    alt: string | null;
    title: string | null;
    width: string | null;
  } = { src: null, alt: null, title: null, width: null };
  for (const [name, value] of parseHtmlAttributes(tagContent)) {
    if (name === 'src') result.src = value;
    else if (name === 'alt') result.alt = value;
    else if (name === 'title') result.title = value;
    else if (name === 'width') result.width = value;
  }
  return result;
}

export interface InlineTokenSourceRange {
  from: number;
  to: number;
  point?: number;
}

export function inlineTokenSourceRange(token: Token): InlineTokenSourceRange | undefined {
  return token.meta?.inlineSourceRange;
}

/** 在规则消费原文时记录范围，避免从已解码、裁剪的 token.content 反推源码。 */
export function recordInlineSourceRanges(markdown: MarkdownIt): void {
  markdown.inline.ruler2.disable('fragments_join');
  markdown.core.ruler.disable('text_join');
  markdown.inline.tokenize = (state) => {
    const rules = state.md.inline.ruler.getRules('');
    const end = state.posMax;
    const originalPush = state.push;
    state.push = function (type, tag, nesting) {
      const token = originalPush.call(this, type, tag, nesting);
      token.meta = { ...token.meta, inlineSourcePoint: this.pos };
      return token;
    };
    try {
      while (state.pos < end) {
        const from = state.pos;
        const first = state.tokens.length;
        let accepted = false;
        if (state.level < (state.md.options as { maxNesting: number }).maxNesting) {
          for (const rule of rules) {
            if (rule(state, false)) { accepted = true; break; }
          }
        }
        if (!accepted) state.pending += state.src[state.pos++];
        if (state.pos <= from) throw new Error('Inline tokenizer did not advance');
        if (!accepted) continue;
        const to = state.pos;
        const added = state.tokens.slice(first);
        const markerRun = /^(?:\*+|_+|~{2,})$/.test(state.src.slice(from, to));
        let markerPos = from;
        for (const token of added) {
          if (inlineTokenSourceRange(token) || (token.type === 'text' && token.meta?.inlineSourcePoint == null)) continue;
          // 自定义规则可能在 push 后整体重写 meta（例如 footnote_ref）。
          const point = (token.meta?.inlineSourcePoint ?? from) as number;
          let range: InlineTokenSourceRange = { from, to, point };
          if (markerRun && token.type === 'text' && /^[*_~]+$/.test(token.content)) {
            range = { from: markerPos, to: markerPos + token.content.length, point };
            markerPos = range.to;
          } else if (token.type === 'link_open') {
            range.to = point > from ? point : from + (state.src[from] === '<' ? 1 : 0);
          } else if (token.type === 'link_close') {
            range.from = point > from ? point : to - (state.src[from] === '<' ? 1 : 0);
          }
          token.meta = { ...token.meta, inlineSourceRange: range };
        }
        // 图片属性规则延长前一个 image 的源码范围，本身不产生 token。
        if (!added.length && state.src[from] === '{') {
          const previous = state.tokens[state.tokens.length - 1];
          const range = previous && inlineTokenSourceRange(previous);
          if (previous?.type === 'image' && range) range.to = to;
        }
      }
      if (state.pending) state.pushPending();
    } finally {
      state.push = originalPush;
    }
  };
}

export function createMarkdownTokenizer(options: { inlineSource?: boolean; literalHtmlCode?: boolean } = {}): MarkdownIt {
  const markdownIt = MarkdownIt('commonmark', { html: true }).enable(['table', 'strikethrough']);
  registerGfmAutolinks(markdownIt);
  markdownIt.validateLink = (url: string) => normalizeLinkHref(url) !== null;
  let scriptDepth = 0;
  markdownIt.inline.ruler.before('escape', 'script_space', (state, silent) => {
    if (!scriptDepth || state.src[state.pos] !== '\\' || !/[ \t]/.test(state.src[state.pos + 1] ?? '')) return false;
    if (!silent) {
      const token = state.push('text_special', '', 0);
      token.content = state.src[state.pos + 1];
      token.info = 'escape';
    }
    state.pos += 2;
    return true;
  });

  // 单字符定界符不消费删除线的 ~~；范围直接记录，避免嵌套解析后反推偏移。
  markdownIt.inline.ruler.before('strikethrough', 'script_format', (state, silent) => {
    const start = state.pos;
    const marker = state.src[start];
    if (marker !== '^' && marker !== '~') return false;
    if (state.src[start + 1] === marker) return false;
    let end = start + 1;
    for (; end < state.posMax; end++) {
      const char = state.src[end];
      // 代码中的 ^/~ 是字面量，不能截断外层上/下标并破坏后续行内解析。
      if (char === '`') {
        const run = /^`+/.exec(state.src.slice(end))![0];
        let closing = end + run.length;
        while ((closing = state.src.indexOf(run, closing)) >= 0 && closing < state.posMax) {
          if (state.src[closing - 1] !== '`' && state.src[closing + run.length] !== '`') break;
          closing += run.length;
        }
        if (closing >= 0 && closing + run.length <= state.posMax &&
            !/[\r\n]/.test(state.src.slice(end, closing))) {
          end = closing + run.length - 1;
          continue;
        }
      }
      if (char === '<') {
        const code = /^<code(?:\s(?:[^"'<>]|"[^"]*"|'[^']*')*)?>[^\r\n]*?<\/code\s*>/i.exec(state.src.slice(end, state.posMax));
        if (code) { end += code[0].length - 1; continue; }
      }
      if (char === '\n' || char === '\r') return false;
      if (char === '\\' && end + 1 < state.posMax) {
        if (/[\r\n]/.test(state.src[end + 1])) return false;
        end++;
        continue;
      }
      if (/\s/.test(char)) return false;
      if (char === marker) break;
    }
    if (end === start + 1 || end >= state.posMax || state.src[end + 1] === marker) return false;
    if (silent) { state.pos = end + 1; return true; }
    const type = marker === '^' ? 'superscript' : 'subscript';
    const tag = marker === '^' ? 'sup' : 'sub';
    const open = state.push(`${type}_open`, tag, 1);
    open.markup = marker;
    open.meta = { ...open.meta, inlineSourceRange: { from: start, to: start + 1 } };
    const max = state.posMax;
    state.pos = start + 1;
    state.posMax = end;
    scriptDepth++;
    try { state.md.inline.tokenize(state); }
    finally { state.posMax = max; scriptDepth--; }
    const close = state.push(`${type}_close`, tag, -1);
    close.markup = marker;
    close.meta = { ...close.meta, inlineSourceRange: { from: end, to: end + 1 } };
    state.pos = end + 1;
    return true;
  });

  if (options.inlineSource || options.literalHtmlCode) {
    // HTML code 与反引号代码使用同一字面内容边界，内部不再生成公式、链接或格式 token。
    markdownIt.inline.ruler.before('backticks', 'inline_source_html_code', (state, silent) => {
      const opening = /^<code(?:\s(?:[^"'<>]|"[^"]*"|'[^']*')*)?>/i.exec(state.src.slice(state.pos));
      if (!opening) return false;
      const bodyFrom = state.pos + opening[0].length;
      const closing = /<\/code\s*>/i.exec(state.src.slice(bodyFrom, state.posMax));
      if (!closing) return false;
      const bodyTo = bodyFrom + closing.index;
      const to = bodyTo + closing[0].length;
      if (!silent) {
        const token = state.push('inline_source_code', 'code', 0);
        const body = state.src.slice(bodyFrom, bodyTo);
        const spans: Array<{ text: string; from: number; to: number }> = [];
        let cursor = 0;
        for (const match of body.matchAll(/&(?:#x[\da-f]+|#\d+|[a-z][a-z\d]+);/gi)) {
          if (match.index! > cursor) spans.push({ text: body.slice(cursor, match.index), from: bodyFrom + cursor, to: bodyFrom + match.index! });
          spans.push({ text: markdownIt.utils.unescapeAll(match[0]), from: bodyFrom + match.index!, to: bodyFrom + match.index! + match[0].length });
          cursor = match.index! + match[0].length;
        }
        if (cursor < body.length) spans.push({ text: body.slice(cursor), from: bodyFrom + cursor, to: bodyTo });
        token.content = spans.map((span) => span.text).join('');
        token.meta = { ...token.meta, statsSpans: spans, inlineSourceCode: { openLength: opening[0].length, closeLength: closing[0].length } };
      }
      state.pos = to;
      return true;
    });
  }

  markdownIt.inline.ruler.before('link', 'footnote_ref', (state, silent) => {
    const src = state.src;
    const pos = state.pos;
    if (src.charCodeAt(pos) !== 0x5b || src.charCodeAt(pos + 1) !== 0x5e) return false;

    const end = src.indexOf(']', pos + 2);
    if (end === -1) return false;

    const id = src.slice(pos + 2, end).trim();
    if (!id || /\s/.test(id)) return false;

    if (!silent) {
      const token = state.push('footnote_ref', 'sup', 0);
      token.content = id;
      token.markup = '[^]';
      token.meta = { id };
    }
    // 静默扫描只推进左括号，让链接标签按文本括号平衡，避免被当成嵌套链接。
    state.pos = silent ? pos + 1 : end + 1;
    return true;
  });

  markdownIt.inline.ruler.after('backticks', 'math_inline', (state, silent) => {
    const src = state.src;
    const pos = state.pos;
    if (src.charCodeAt(pos) !== 0x24) return false;
    if (pos + 1 < src.length && src.charCodeAt(pos + 1) === 0x24) return false; // $$ display
    // 属于 $$ 的第二个 $：仅当 pos-1 和 pos-2 都是 $ 时才跳过（即 $$$ 三连），
    // 避免误判相邻行内公式 $a$$b$ 的情况（pos-1 是前一个公式的闭合 $）
    if (pos > 0 && src.charCodeAt(pos - 1) === 0x24) {
      if (pos === 1 || src.charCodeAt(pos - 2) === 0x24) return false;
    }

    let end = pos + 1;
    while (end < src.length) {
      if (src.charCodeAt(end) === 0x24) {
        let bsCount = 0;
        let i = end - 1;
        while (i > pos && src.charCodeAt(i) === 0x5c) {
          bsCount++;
          i--;
        }
        if (bsCount % 2 === 0) break;
      }
      end++;
    }
    if (end >= src.length || end === pos + 1) return false;

    const tex = src.slice(pos + 1, end).trim();
    if (!tex.trim()) return false;

    if (!silent) {
      const token = state.push('math_inline', '', 0);
      token.content = tex;
      token.markup = '$';
    }
    state.pos = end + 1;
    return true;
  });

  markdownIt.block.ruler.before(
    'reference',
    'footnote_def',
    (state, startLine, _endLine, silent) => {
      const startPos = state.bMarks[startLine] + state.tShift[startLine];
      const lineText = state.src.slice(startPos, state.eMarks[startLine]);
      const match = /^\[\^([^\]\s]+)\]:[ \t]*(.*)$/.exec(lineText);
      if (!match) return false;

      if (silent) return true;

      const id = match[1];
      const content = match[2] ?? '';
      const openToken = state.push('footnote_def_open', 'div', 1);
      openToken.block = true;
      openToken.map = [startLine, startLine + 1];
      openToken.markup = '[^]:';
      openToken.meta = { id };

      const inlineToken = state.push('inline', '', 0);
      inlineToken.content = content;
      inlineToken.children = [];
      inlineToken.map = [startLine, startLine + 1];
      inlineToken.meta = {
        statsContentColumn: state.tShift[startLine] + lineText.length - content.length,
      };

      const closeToken = state.push('footnote_def_close', 'div', -1);
      closeToken.block = true;
      closeToken.markup = '[^]:';
      closeToken.meta = { id };

      state.line = startLine + 1;
      return true;
    },
  );

  // ——— 图片属性解析：支持 ![alt](src){align=center width=60%} 语法 ———

  // 1. 行内规则：在 image token 后方检测 {key=value ...} 属性块
  markdownIt.inline.ruler.after('image', 'image_attrs', (state, silent) => {
    const pos = state.pos;
    if (state.src.charCodeAt(pos) !== 0x7b) return false; // '{'

    // 前一个 token 必须是 image
    const prevToken = state.tokens[state.tokens.length - 1];
    if (!prevToken || prevToken.type !== 'image') return false;

    const closeBrace = state.src.indexOf('}', pos + 1);
    if (closeBrace === -1) return false;

    const attrsStr = state.src.slice(pos + 1, closeBrace).trim();
    if (!attrsStr) return false;

    const attrs: Record<string, string> = {};
    for (const part of attrsStr.split(/\s+/)) {
      const eq = part.indexOf('=');
      if (eq <= 0) continue;
      attrs[part.slice(0, eq)] = part.slice(eq + 1);
    }

    if (silent) return true;

    // 回写到 image token 的 attrs 中（attrs 是 [name, value] 数组，需用 attrSet）
    if (attrs.align) prevToken.attrSet('align', attrs.align);
    if (attrs.width) prevToken.attrSet('width', attrs.width);

    state.pos = closeBrace + 1;
    return true;
  });

  // 2. 行内规则：在 html_inline 之前检测 <img src="..." ...> 内联 HTML 图片
  markdownIt.inline.ruler.before('html_inline', 'image_html_inline', (state, silent) => {
    const pos = state.pos;
    if (state.src.slice(pos, pos + 4).toLowerCase() !== '<img') return false;

    // 找到结束的 >（简单匹配，不处理属性值内含 > 的极端情况）
    const end = state.src.indexOf('>', pos + 4);
    if (end === -1) return false;

    // 检查闭合前没有未转义的 <（防止误匹配 <img ...><script>）
    const tagContent = state.src
      .slice(pos + 4, end)
      .replace(/\/$/, '')
      .trim();
    if (tagContent.includes('<')) return false;

    const imgAttrs = parseHtmlImgAttrs(tagContent);
    if (!imgAttrs.src) return false;

    state.pos = end + 1;
    if (silent) return true;

    const token = state.push('image', 'img', 0);
    token.attrSet('src', imgAttrs.src);
    if (imgAttrs.alt !== null) token.attrSet('alt', imgAttrs.alt);
    if (imgAttrs.title) token.attrSet('title', imgAttrs.title);
    if (imgAttrs.width) token.attrSet('width', imgAttrs.width);

    // 删除属性块空行（如果有）
    state.pos = end + 1;
    return true;
  });

  // 注册 markdown-it block rule 识别 $$...$$ 跨行公式
  markdownIt.block.ruler.after('fence', 'math_display', (state, startLine, endLine, silent) => {
    if (state.sCount[startLine] - state.blkIndent >= 4) return false;
    const startPos = state.bMarks[startLine] + state.tShift[startLine];
    const lineText = state.src.slice(startPos, state.eMarks[startLine]).trim();

    // 当前行必须以 $$ 开头（允许前导空格，trim 后判断）
    if (!lineText.startsWith('$$')) return false;

    // 单行 $$...$$ 形式（同行闭合）：如 $$ E=mc^2 $$
    const singleLineContent = lineText.slice(2);
    if (singleLineContent.endsWith('$$') && singleLineContent.length > 2) {
      const tex = singleLineContent.slice(0, -2).trim();
      if (tex) {
        if (silent) return true;
        const token = state.push('math_display', 'math', 0);
        token.content = tex;
        token.markup = '$$';
        token.map = [startLine, startLine + 1];
        state.line = startLine + 1;
        return true;
      }
    }

    // 多行公式允许首尾内容与 $$ 同行，收集时只去掉分隔符。
    const texLines: string[] = [state.src.slice(startPos + 2, state.eMarks[startLine])];
    let foundClose = false;
    let nextLine = startLine + 1;

    for (let i = startLine + 1; i < endLine; i++) {
      const linePos = state.bMarks[i] + state.tShift[i];
      // 非空行退出当前列表或引用容器后，不能在相邻项中继续寻找闭合符。
      if (linePos < state.eMarks[i] && state.sCount[i] < state.blkIndent) break;
      const contentLine = state.getLines(i, i + 1, state.blkIndent, false);
      const line = contentLine.trimEnd();
      if (line.endsWith('$$')) {
        texLines.push(line.slice(0, -2));
        foundClose = true;
        nextLine = i + 1;
        break;
      }
      texLines.push(contentLine);
    }

    if (!foundClose) return false;

    const content = texLines.join('\n').trim();
    if (silent) return true;

    const token = state.push('math_display', 'math', 0);
    token.content = content;
    token.markup = '$$';
    token.map = [startLine, nextLine];
    state.line = nextLine;
    return true;
  });

  registerDetailsRule(markdownIt);
  const tokenizeBlocks = markdownIt.block.tokenize.bind(markdownIt.block);
  markdownIt.block.tokenize = (state, startLine, endLine) => {
    const firstTokenIndex = state.tokens.length;
    tokenizeBlocks(state, startLine, endLine);
    for (let index = firstTokenIndex; index < state.tokens.length; index++) {
      const token = state.tokens[index];
      if (!isTocStartToken(token) || typeof token.meta?.tocContainerContent === 'string') continue;
      const endIndex = findTocEndTokenIndex(state.tokens, index + 1);
      const start = token.map?.[0];
      const end = state.tokens[endIndex]?.map?.[0];
      if (start == null || end == null) continue;
      // 此时引用逐行前缀、列表缩进仍在 StateBlock 中生效；避免从 marker 空格反推容器。
      token.meta = {
        ...token.meta,
        tocContainerContent: state.getLines(start + 1, end, state.blkIndent, false),
      };
    }
  };

  const parseMarkdownTokens = markdownIt.parse.bind(markdownIt);
  markdownIt.parse = (src, env) => {
    const rawTokens = collapseTocTokens(parseMarkdownTokens(src, env), src);
    const normalized = [];
    for (const token of rawTokens) {
      if (['thead_open', 'thead_close', 'tbody_open', 'tbody_close'].includes(token.type)) {
        continue;
      }
      if (token.type === 'th_close' || token.type === 'td_close') {
        normalized.push(new Token('paragraph_close', 'p', -1));
      }
      normalized.push(token);
      if (token.type === 'th_open' || token.type === 'td_open') {
        normalized.push(new Token('paragraph_open', 'p', 1));
      }
    }

    const result = restoreBlankParagraphTokens(normalized);

    // 将匹配 [!TYPE] 的 blockquote 改写为 callout
    const calloutSources = options.inlineSource
      ? new Map(result.filter((token) => token.type === 'inline').map((token) => [token, token.content]))
      : null;
    transformCalloutTokens(result);
    if (calloutSources) {
      for (const token of result) {
        const original = calloutSources.get(token);
        if (original == null || original === token.content || !/^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]/.test(original)) continue;
        token.content = original.replace(/^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\](?:\r?\n)?/, '');
        token.children = [];
        markdownIt.inline.parse(token.content, markdownIt, env, token.children);
      }
    }

    return result;
  };

  if (options.inlineSource) recordInlineSourceRanges(markdownIt);
  return markdownIt;
}

function restoreBlankParagraphTokens(tokens: Token[]): Token[] {
  // markdown-it 会把连续空行当作块分隔符丢弃；这里按顶层块的行号映射恢复空段落。
  const result: Token[] = [];
  let previousTopLevelBlockEnd = -1;
  const listItemStack: ListItemBlankParagraphContext[] = [];
  const detailsBodies: Array<ListItemBlankParagraphContext & { hasChild: boolean }> = [];

  for (const token of tokens) {
    const detailsBody = detailsBodies.at(-1);
    if (detailsBody) {
      const range = getDirectListItemChildBlockRange(token, detailsBody);
      if (range) {
        appendBlankParagraphTokens(result, detailsBody.previousChildBlockEnd, range[0]);
        detailsBody.previousChildBlockEnd = range[1];
        detailsBody.hasChild = true;
      }
      if (token.type === 'details_body_close') {
        if (detailsBody.hasChild) appendBlankParagraphTokens(result, detailsBody.previousChildBlockEnd, detailsBody.endLine);
        detailsBodies.pop();
      }
    }
    const listItemContext = getCurrentListItemContext(listItemStack);
    const listItemChildRange = listItemContext
      ? getDirectListItemChildBlockRange(token, listItemContext)
      : null;
    if (listItemContext && listItemChildRange) {
      if (listItemContext.previousChildBlockEnd >= 0) {
        appendBlankParagraphTokens(
          result,
          listItemContext.previousChildBlockEnd,
          listItemChildRange[0],
        );
      }
      listItemContext.previousChildBlockEnd = listItemChildRange[1];
    }

    if (token.type === 'list_item_close') {
      const context = listItemStack.pop();
      if (context && context.previousChildBlockEnd >= 0) {
        appendBlankParagraphTokens(result, context.previousChildBlockEnd, context.endLine);
      }
    }

    const blockRange = getTopLevelBlockRange(token);
    if (blockRange) {
      if (previousTopLevelBlockEnd >= 0) {
        appendBlankParagraphTokens(result, previousTopLevelBlockEnd, blockRange[0]);
      }
      previousTopLevelBlockEnd = blockRange[1];
    }

    result.push(token);
    if (token.type === 'details_body_open' && token.map) detailsBodies.push({
      childLevel: token.level + 1,
      previousChildBlockEnd: token.map[0] + 1,
      endLine: token.map[1] - 1,
      hasChild: false,
    });

    const newListItemContext = createListItemBlankParagraphContext(token);
    if (newListItemContext) {
      listItemStack.push(newListItemContext);
    }
  }

  return result;
}

type ListItemBlankParagraphContext = {
  childLevel: number;
  endLine: number;
  previousChildBlockEnd: number;
};

function createListItemBlankParagraphContext(token: Token): ListItemBlankParagraphContext | null {
  if (token.type !== 'list_item_open' || !token.map) {
    return null;
  }
  return {
    childLevel: token.level + 1,
    endLine: token.map[1],
    previousChildBlockEnd: -1,
  };
}

function getCurrentListItemContext(
  stack: ListItemBlankParagraphContext[],
): ListItemBlankParagraphContext | null {
  return stack.length > 0 ? stack[stack.length - 1] : null;
}

function getDirectListItemChildBlockRange(
  token: Token,
  context: ListItemBlankParagraphContext,
): [number, number] | null {
  if (token.level !== context.childLevel || !token.map) {
    return null;
  }
  if (token.nesting === -1 || token.type === 'inline' || token.type === 'list_item_open') {
    return null;
  }
  return [token.map[0], token.map[1]];
}

function getTopLevelBlockRange(token: Token): [number, number] | null {
  if (token.level !== 0 || !token.map) {
    return null;
  }
  if (token.nesting === -1) {
    return null;
  }
  if (token.type === 'inline') {
    return null;
  }
  return [token.map[0], token.map[1]];
}

function appendBlankParagraphTokens(
  result: Token[],
  previousEndLine: number,
  nextStartLine: number,
) {
  const gap = nextStartLine - previousEndLine - 1;
  for (let index = 0; index < gap; index++) {
    const line = previousEndLine + 1 + index;
    result.push(
      createEmptyParagraphOpen(line),
      createEmptyInlineToken(line),
      createEmptyParagraphClose(),
    );
  }
}

function createEmptyParagraphOpen(line: number): Token {
  const emptyOpen = new Token('paragraph_open', 'p', 1);
  emptyOpen.map = [line, line + 1];
  return emptyOpen;
}

function createEmptyInlineToken(line: number): Token {
  const emptyInline = new Token('inline', '', 0);
  emptyInline.content = '';
  emptyInline.children = [];
  emptyInline.map = [line, line + 1];
  return emptyInline;
}

function createEmptyParagraphClose(): Token {
  return new Token('paragraph_close', 'p', -1);
}

function collapseTocTokens(tokens: Token[], markdown: string): Token[] {
  const result: Token[] = [];
  const lines = markdown.split(/\r?\n/);

  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if (!isTocStartToken(token)) {
      result.push(token);
      continue;
    }

    const endIndex = findTocEndTokenIndex(tokens, index + 1);
    if (endIndex === -1) {
      result.push(token);
      continue;
    }

    const startLine = token.map?.[0] ?? 0;
    const endLine = tokens[endIndex].map?.[0] ?? startLine;
    const tocToken = new Token('toc_block', '', 0);
    tocToken.level = token.level;
    // 容器前缀由 block 解析阶段移除，序列化时再逐行添加。
    const containerContent =
      token.meta?.tocContainerContent ?? lines.slice(startLine + 1, endLine).join('\n');
    tocToken.content = containerContent.replace(/^(?:[ \t]*\r?\n)+|(?:\r?\n[ \t]*)+$/g, '');
    tocToken.map = [startLine, tokens[endIndex].map?.[1] ?? endLine + 1];
    result.push(tocToken);
    index = endIndex;
  }

  return result;
}

function isTocStartToken(token: Token): boolean {
  return token.type === 'html_block' && /^<!--[^\S\r\n]*toc[^\S\r\n]*-->\s*$/i.test(token.content.trim());
}

function isTocEndToken(token: Token): boolean {
  return token.type === 'html_block' && /^<!--[^\S\r\n]*\/toc[^\S\r\n]*-->\s*$/i.test(token.content.trim());
}

function findTocEndTokenIndex(tokens: Token[], from: number): number {
  for (let index = from; index < tokens.length; index++) {
    if (isTocEndToken(tokens[index])) {
      return index;
    }
  }
  return -1;
}
