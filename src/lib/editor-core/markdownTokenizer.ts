import MarkdownIt from 'markdown-it';
import Token from 'markdown-it/lib/token.mjs';
import { normalizeLinkHref } from './link';
import { transformCalloutTokens } from './callout/calloutParser';

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
    .replace(/>/g, '&gt;');
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
  // 匹配 key="value" | key='value' | key=value（value 不含空白和 >）
  const attrRegex = /([a-zA-Z][\w-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/g;
  let match: RegExpExecArray | null;
  while ((match = attrRegex.exec(tagContent)) !== null) {
    const name = match[1].toLowerCase();
    const value = match[2] ?? match[3] ?? match[4];
    if (name === 'src') result.src = value;
    else if (name === 'alt') result.alt = value;
    else if (name === 'title') result.title = value;
    else if (name === 'width') result.width = value;
  }
  return result;
}

export function createMarkdownTokenizer(): MarkdownIt {
  const markdownIt = MarkdownIt('commonmark', { html: true }).enable(['table', 'strikethrough']);
  markdownIt.validateLink = (url: string) => normalizeLinkHref(url) !== null;

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
    state.pos = end + 1;
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

    const tex = src
      .slice(pos + 1, end)
      .trim()
      .replace(/\\\$/g, '$');
    if (!tex.trim()) return false;

    if (!silent) {
      const token = state.push('math_inline', '', 0);
      token.content = tex;
      token.markup = '$';
      state.pos = end + 1;
    }
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

    // 多行 $$ 形式：从 startLine+1 向下扫描闭合 $$ 行
    const texLines: string[] = [];
    let foundClose = false;
    let nextLine = startLine + 1;

    for (let i = startLine + 1; i < endLine; i++) {
      const lineStart = state.bMarks[i] + state.tShift[i];
      const line = state.src.slice(lineStart, state.eMarks[i]).trim();
      if (line === '$$') {
        foundClose = true;
        nextLine = i + 1;
        break;
      }
      texLines.push(state.src.slice(state.bMarks[i], state.eMarks[i]));
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
    transformCalloutTokens(result);

    return result;
  };

  return markdownIt;
}

function restoreBlankParagraphTokens(tokens: Token[]): Token[] {
  // markdown-it 会把连续空行当作块分隔符丢弃；这里按顶层块的行号映射恢复空段落。
  const result: Token[] = [];
  let previousTopLevelBlockEnd = -1;
  const listItemStack: ListItemBlankParagraphContext[] = [];

  for (const token of tokens) {
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
    tocToken.content = lines
      .slice(startLine + 1, endLine)
      .join('\n')
      .trim();
    tocToken.map = [startLine, tokens[endIndex].map?.[1] ?? endLine + 1];
    result.push(tocToken);
    index = endIndex;
  }

  return result;
}

function isTocStartToken(token: Token): boolean {
  return token.type === 'html_block' && /^<!--\s*toc\s*-->\s*$/i.test(token.content.trim());
}

function isTocEndToken(token: Token): boolean {
  return token.type === 'html_block' && /^<!--\s*\/toc\s*-->\s*$/i.test(token.content.trim());
}

function findTocEndTokenIndex(tokens: Token[], from: number): number {
  for (let index = from; index < tokens.length; index++) {
    if (isTocEndToken(tokens[index])) {
      return index;
    }
  }
  return -1;
}
