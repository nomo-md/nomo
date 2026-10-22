import type Token from 'markdown-it/lib/token.mjs';
import { createMarkdownTokenizer } from '../editor-core/markdownTokenizer';
import { readDetailsSource } from '../editor-core/details/detailsSyntax';
import { extractFrontMatterBlock } from '../markdown/frontMatter';
import { utf8Length } from './utf8Text';
import { emptyEnhancements, enhancementTotal, type MarkdownEnhancements } from './enhancements';

export interface MarkdownStructureBlock {
  id: string;
  version: number;
  /** 源文/结构内容版本；位置移动或发布到新文档修订不改变它。 */
  contentVersion?: number;
  fromByte: number;
  toByte: number;
  fromLine: number;
  toLine: number;
  kind: string;
  mode: 'semantic' | 'raw';
  layoutUnits: number;
  enhancementObjects: number;
  enhancements: MarkdownEnhancements;
  /** 巨型原文区域尚无完整语义计数时，不能据此退回全文挂载。 */
  metricsComplete: boolean;
  headings: Array<{ level: number; text: string; byte: number; line: number }>;
  references: string[];
  definitions?: Record<string, MarkdownReference>;
  /** 只作增量重扫的快速筛选；复用后缀之前仍核对实际源文。 */
  sourceHash?: number;
}
export interface MarkdownReference {
  href: string;
  title: string;
  map?: number[];
}
export interface MarkdownStructureBatch {
  blocks: MarkdownStructureBlock[];
  references: Record<string, MarkdownReference>;
  consumedBytes: number;
  complete: boolean;
}

const MAX_SEMANTIC_BYTES = 32 * 1024;
const tokenizer = createMarkdownTokenizer({ inlineSource: true });

function lineOffsets(text: string) {
  const offsets = [0];
  for (const match of text.matchAll(/\r\n|\n|\r/g)) offsets.push(match.index! + match[0].length);
  return offsets;
}
function blockStarts(tokens: readonly Token[]) {
  return tokens.filter((token) => token.level === 0 && token.nesting >= 0 && token.map);
}
function tokenCounts(tokens: readonly Token[]) {
  let layoutUnits = 0;
  const enhancements = emptyEnhancements();
  const headings: Array<{ level: number; text: string; line: number }> = [];
  const references = new Set<string>();
  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if (
      [
        'paragraph_open',
        'heading_open',
        'td_open',
        'th_open',
        'fence',
        'code_block',
        'math_display',
      ].includes(token.type)
    )
      layoutUnits++;
    if (token.type === 'fence' && token.info.trim().split(/\s+/)[0]?.toLowerCase() === 'mermaid') enhancements.diagrams++;
    else if (token.type === 'fence' || token.type === 'code_block') enhancements.codeBlocks++;
    else if (token.type === 'math_display') enhancements.blockMath++;
    else if (['html_block', 'details_html_body', 'details_summary_html'].includes(token.type)) enhancements.htmlDetails++;
    if (token.type === 'heading_open')
      headings.push({
        level: Number(token.tag.slice(1)),
        text: tokens[index + 1]?.content ?? '',
        line: token.map?.[0] ?? 0,
      });
    if (token.type === 'inline') {
      for (const child of token.children ?? []) {
        if (child.type === 'image') enhancements.images++;
        else if (child.type === 'math_inline') enhancements.inlineMath++;
        else if (child.type === 'code_inline') enhancements.inlineCode++;
      }
      // 依赖候选允许多报；只有实际 tokenizer 解析结果能决定链接语义。
      for (const match of token.content.matchAll(/\[([^\]\n]+)\](?:\[([^\]\n]*)\])?/g))
        references.add((match[2] || match[1]).trim().replace(/\s+/g, ' ').toUpperCase());
    }
  }
  return { layoutUnits, enhancements, enhancementObjects: enhancementTotal(enhancements), headings, references: [...references] };
}

type RawState = {
  kind: string;
  fence?: string;
  fenceOpened?: boolean;
  blank: boolean;
  detailsDepth?: number;
  htmlEnd?: string;
};

/**
 * 真正 tokenizer 决定块边界，最后一个块留作后文上下文。Details、TOC 和前置信息
 * 在闭合之前也不能发布其内部片段。巨型块不反复送入同步 tokenizer，而保留原文区域。
 */
export class MarkdownStructureScanner {
  private pending = '';
  private fromByte: number;
  private fromLine: number;
  private references: Record<string, MarkdownReference>;
  private raw: RawState | null = null;
  private rawBlock: MarkdownStructureBlock | null = null;
  private finished = false;
  constructor(
    readonly revision: number,
    options: {
      fromByte?: number;
      fromLine?: number;
      references?: Record<string, MarkdownReference>;
      maxSemanticBytes?: number;
    } = {},
  ) {
    this.fromByte = options.fromByte ?? 0;
    this.fromLine = options.fromLine ?? 1;
    this.references = Object.assign(Object.create(null), options.references);
    this.limit = options.maxSemanticBytes ?? MAX_SEMANTIC_BYTES;
  }
  private readonly limit: number;

  push(text: string, complete = false): MarkdownStructureBatch {
    if (this.finished) throw new Error('结构扫描已结束。');
    this.pending += text;
    const blocks: MarkdownStructureBlock[] = [];
    for (;;) {
      if (this.raw) {
        if (!this.consumeRaw(blocks, complete)) break;
        continue;
      }
      if (!this.pending) break;
      // 一次扫描只对有界正文工作；读入端每次最多发送 16 KiB。
      const oversized = utf8Length(this.pending) > this.limit;
      let candidate = oversized
        ? this.pending.slice(0, this.safePrefixLength(this.pending, this.limit))
        : this.pending;
      if (!(complete && !oversized)) {
        const completeLines = lineOffsets(candidate);
        let end = completeLines.at(-1) ?? 0;
        if (candidate.endsWith('\r')) end = completeLines.at(-2) ?? 0;
        candidate = candidate.slice(0, end);
        if (!candidate && !oversized) break;
      }
      const offsets = lineOffsets(candidate);
      const env = { references: { ...this.references } };
      const front = this.fromByte === 0 ? extractFrontMatterBlock(candidate) : null;
      if (front) {
        const block = this.block(front.raw, 'front_matter', [], true);
        blocks.push(block);
        this.consume(front.raw.length);
        continue;
      }
      const tokens = tokenizer.parse(candidate, env);
      const starts = blockStarts(tokens);
      let cutoff =
        complete && !oversized ? candidate.length : (offsets[starts.at(-1)?.map?.[0] ?? 0] ?? 0);
      // 第一行 --- 需要等闭合行；未闭合到 EOF 时按普通 Markdown 解释。
      if (this.fromByte === 0 && /^---\r?\n/.test(candidate) && !front && !(complete && !oversized))
        cutoff = 0;
      for (const token of tokens) {
        if (token.type !== 'html_block' || !token.map) continue;
        const start = offsets[token.map[0]] ?? 0;
        const details = /^\s*<details(?:\s|>)/i.test(token.content);
        const toc = /^\s*<!--\s*toc\s*-->\s*$/i.test(token.content);
        if ((details && !readDetailsSource(candidate.slice(start).trimStart())) || toc) {
          if (complete && !oversized) continue;
          const parent = starts.find(
            (block) => block.map![0] <= token.map![0] && block.map![1] >= token.map![1],
          );
          cutoff = Math.min(cutoff, offsets[parent?.map?.[0] ?? token.map[0]] ?? 0);
        }
      }
      if (cutoff > 0) {
        const source = candidate.slice(0, cutoff);
        const committedEnv = { references: { ...this.references } };
        const committed = tokenizer.parse(source, committedEnv);
        this.references = committedEnv.references;
        const starts = blockStarts(committed);
        const sourceOffsets = lineOffsets(source);
        const boundaries = [
          ...new Set([
            0,
            ...starts.slice(1).map((token) => sourceOffsets[token.map![0]] ?? 0),
            source.length,
          ]),
        ];
        // 顶层容器保持完整；相邻窗口仍按连续 source 范围合并读取，保留中间空行。
        for (let index = 0; index < boundaries.length - 1; index++) {
          const piece = source.slice(boundaries[index], boundaries[index + 1]);
          const tokens = tokenizer.parse(piece, { references: { ...this.references } });
          blocks.push(this.block(piece, blockStarts(tokens)[0]?.type ?? 'fragment', tokens, true));
          this.consume(piece.length);
        }
        continue;
      }
      if (!oversized) break;
      const first = starts[0];
      this.raw = { kind: first?.type ?? 'unknown', blank: false };
      if (this.fromByte === 0 && /^---\r?\n/.test(this.pending) && !front)
        this.raw.kind = 'unknown';
      if (first?.type === 'fence') this.raw.fence = first.markup;
      if (/^\s*<details(?:\s|>)/i.test(candidate)) this.raw.detailsDepth = 0;
      const html = /^\s*<(script|pre|style|textarea)\b/i.exec(candidate);
      if (html) this.raw.htmlEnd = `</${html[1].toLowerCase()}`;
      if (/^\s*<!--\s*toc\s*-->/i.test(candidate)) this.raw.htmlEnd = '<!-- /toc -->';
    }
    this.finished = complete && !this.pending;
    return {
      blocks,
      references: { ...this.references },
      consumedBytes: this.fromByte,
      complete: this.finished,
    };
  }

  private safePrefixLength(source: string, bytes: number) {
    let offset = 0;
    let count = 0;
    for (const char of source) {
      const next = count + utf8Length(char);
      if (next > bytes) break;
      count = next;
      offset += char.length;
    }
    if (source[offset - 1] === '\r' && source[offset] === '\n') offset--;
    return offset;
  }
  private block(
    source: string,
    kind: string,
    tokens: readonly Token[],
    semantic: boolean,
    offsets = lineOffsets(source),
  ): MarkdownStructureBlock {
    const counts = tokenCounts(tokens);
    const definitionEnv: { references: Record<string, MarkdownReference> } = { references: {} };
    if (semantic) tokenizer.parse(source, definitionEnv);
    let sourceHash = 2166136261;
    for (let index = 0; index < source.length; index++)
      sourceHash = Math.imul(sourceHash ^ source.charCodeAt(index), 16777619) >>> 0;
    return {
      id: globalThis.crypto.randomUUID(),
      version: this.revision,
      contentVersion: this.revision,
      fromByte: this.fromByte,
      toByte: this.fromByte + utf8Length(source),
      fromLine: this.fromLine,
      toLine: this.fromLine + offsets.length - 1,
      kind,
      mode: semantic ? 'semantic' : 'raw',
      layoutUnits: counts.layoutUnits,
      enhancementObjects: counts.enhancementObjects,
      enhancements: counts.enhancements,
      metricsComplete:
        semantic &&
        !tokens.some((token) => ['html_block', 'details_html_body'].includes(token.type)),
      definitions: definitionEnv.references,
      sourceHash,
      headings: counts.headings.map((heading) => ({
        ...heading,
        line: this.fromLine + heading.line,
        byte: this.fromByte + utf8Length(source.slice(0, offsets[heading.line] ?? 0)),
      })),
      references: counts.references,
    };
  }
  private consume(characters: number) {
    const source = this.pending.slice(0, characters);
    this.fromByte += utf8Length(source);
    this.fromLine += lineOffsets(source).length - 1;
    this.pending = this.pending.slice(characters);
  }
  private consumeRaw(blocks: MarkdownStructureBlock[], complete: boolean): boolean {
    const raw = this.raw!;
    const newline = /\r\n|\n|\r/.exec(this.pending);
    let length = newline ? newline.index + newline[0].length : complete ? this.pending.length : 0;
    if (!length && utf8Length(this.pending) > this.limit)
      length = this.safePrefixLength(this.pending, this.limit);
    if (!length) {
      if (complete) {
        this.raw = null;
        this.rawBlock = null;
      }
      return false;
    }
    const line = this.pending.slice(0, length);
    const closedLine = !!newline || complete;
    let ends = false;
    const alreadyStarted = !!this.rawBlock;
    if (raw.detailsDepth !== undefined) {
      // 巨型混合 HTML 的安全闭合尚未被证明，保留原文后缀；不能把代码/注释中的标签当边界。
      ends = false;
    } else if (raw.fence) {
      const closing = /^ {0,3}(`{3,}|~{3,})[ \t]*(?:\r?\n|\r)?$/.exec(line);
      ends =
        !!raw.fenceOpened &&
        !!closing &&
        closing[1][0] === raw.fence[0] &&
        closing[1].length >= raw.fence.length;
      if (!raw.fenceOpened && /^ {0,3}(`{3,}|~{3,})/.test(line)) raw.fenceOpened = true;
    } else if (raw.htmlEnd) ends = line.toLowerCase().includes(raw.htmlEnd);
    else if (
      raw.kind !== 'unknown' &&
      alreadyStarted &&
      closedLine &&
      raw.blank &&
      /^\S/.test(line) &&
      !/^(?:[-+*]|\d+[.)])\s|^>/.test(line)
    ) {
      this.raw = null;
      this.rawBlock = null;
      return true;
    }
    const part = this.block(line, raw.kind, [], false);
    if (!this.rawBlock) this.rawBlock = part;
    else
      this.rawBlock = {
        ...this.rawBlock,
        toByte: part.toByte,
        toLine: part.toLine,
        sourceHash: undefined,
      };
    const previous = blocks.at(-1);
    if (previous?.id === this.rawBlock.id) blocks[blocks.length - 1] = this.rawBlock;
    else blocks.push(this.rawBlock);
    this.consume(length);
    raw.blank = closedLine && !line.trim();
    if (ends) {
      this.raw = null;
      this.rawBlock = null;
    }
    return !!this.pending;
  }
}
