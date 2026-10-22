import { Fragment, Slice, type Node as ProseMirrorNode } from 'prosemirror-model';
import {
  parseMarkdown,
  parseMarkdownWithSyncAnchors,
  serializeMarkdown,
  type MarkdownParseContext,
} from '../editor-core/markdown';
import { removeEmptyTrailingParagraph } from '../editor-core/plugins/trailingParagraph';
import { getSourceTextChanges } from './textChanges';
import { utf16ToUtf8, utf8ToUtf16 } from './utf8Text';
import type { SourceEdit } from './protocol';

type TextChange = ReturnType<typeof getSourceTextChanges>[number];
export class SourceProjectionError extends Error {
  constructor() {
    super('当前结构的原文映射无法安全写回，请保留编辑并切换该区域的原文视图。');
  }
}

function serialized(doc: ProseMirrorNode) {
  return serializeMarkdown(removeEmptyTrailingParagraph(doc));
}

/** 只接受精确相同的文本跨度及变换端点，不按长度比例猜测格式符号中的位置。 */
function mapBoundary(
  position: number,
  changes: readonly TextChange[],
  association: -1 | 1,
): number | null {
  let delta = 0;
  for (const change of changes) {
    if (position < change.from) break;
    if (position > change.to) {
      delta += change.insert.length - (change.to - change.from);
      continue;
    }
    if (change.from === change.to)
      return change.from + delta + (association > 0 ? change.insert.length : 0);
    if (position === change.from) return change.from + delta;
    if (position === change.to) return change.from + delta + change.insert.length;
    return null;
  }
  return position + delta;
}

function completeBoundary(text: string, offset: number) {
  return (
    !(offset > 0 && text[offset - 1] === '\r' && text[offset] === '\n') &&
    !(
      offset > 0 &&
      /[\uD800-\uDBFF]/.test(text[offset - 1]) &&
      /[\uDC00-\uDFFF]/.test(text[offset] ?? '')
    )
  );
}

function newlineAt(source: string, position: number) {
  const following = /\r\n|\r|\n/.exec(source.slice(position));
  if (following) return following[0];
  return source.match(/\r\n|\r|\n/g)?.at(-1) ?? '\n';
}

export interface SourceProjectionChange {
  source: string;
  edits: SourceEdit[];
}

/**
 * 一段有界语义视图的源文投影。canonical 仅用于比较，不作为保存正文。
 * 未改变的文本直接引用 source；结果必须重新解析成目标语义，否则拒绝猜测写回。
 */
export class MarkdownSourceProjection {
  private readonly canonical: string;
  private readonly toSource: TextChange[];
  // 只缓存当前不可变局部文档的标量坐标，最多各 32 项；随视图/修订释放，不保留 DOM。
  private readonly bytePositions = new Map<number, number | null>();
  private readonly positionBytes = new Map<number, number | null>();
  constructor(
    readonly source: string,
    readonly doc: ProseMirrorNode,
    public fromByte = 0,
    private readonly context: MarkdownParseContext = {},
  ) {
    this.canonical = serialized(doc);
    this.toSource = getSourceTextChanges(this.canonical, source, false);
  }

  /** 导航只接受能往返验证的光标位置；格式符号内的源码选区由原文视图承接。 */
  byteToPosition(byte: number): number | null {
    if (byte === this.fromByte) return 0;
    if (byte === this.fromByte + utf16ToUtf8(this.source, this.source.length))
      return this.doc.content.size;
    const relative = byte - this.fromByte;
    if (this.bytePositions.has(relative)) return this.bytePositions.get(relative)!;
    let found: number | null = null;
    try {
      const offset = utf8ToUtf16(this.source, relative);
      if (!completeBoundary(this.source, offset)) return null;
      // 从唯一原文位置正向解析一次，避免对每个文本节点二分并反复序列化整个窗口。
      // 标记只存在于临时字符串/不可变节点中，绝不进入正文、事务或撤销历史。
      const marker = `NOMOCARET${globalThis.crypto.randomUUID().replace(/-/g, '')}`;
      const marked = parseMarkdown(this.source.slice(0, offset) + marker + this.source.slice(offset), this.context);
      marked.descendants((node, start) => {
        if (found !== null) return false;
        const index = node.isText ? node.text!.indexOf(marker) : -1;
        if (index >= 0) found = start + index;
      });
      if (found !== null) {
        const restored = marked.replace(found, found + marker.length, Slice.empty);
        // 语法符号、链接地址等位置不能误当正文；去掉标记后必须仍是完全相同的语义文档。
        if (!removeEmptyTrailingParagraph(restored).eq(removeEmptyTrailingParagraph(this.doc))) found = null;
      }
    } catch (error) {
      if (!(error instanceof RangeError)) throw error;
    }
    this.remember(this.bytePositions, relative, found);
    return found;
  }

  positionToByte(position: number): number | null {
    if (this.positionBytes.has(position)) {
      const relative = this.positionBytes.get(position)!;
      return relative === null ? null : this.fromByte + relative;
    }
    const byte = this.resolvePositionByte(position);
    this.remember(this.positionBytes, position, byte === null ? null : byte - this.fromByte);
    return byte;
  }

  private remember(cache: Map<number, number | null>, key: number, value: number | null) {
    if (cache.size >= 32) cache.delete(cache.keys().next().value!);
    cache.set(key, value);
  }

  private resolvePositionByte(position: number): number | null {
    if (position < 0 || position > this.doc.content.size) return null;
    const resolved = this.doc.resolve(position);
    if (resolved.parent.inlineContent) {
      // 用实际序列化规则定位文本光标；插入仅在临时不可变节点上发生，不进入会话或历史。
      const marker = `NOMOCARET${globalThis.crypto.randomUUID().replace(/-/g, '')}`;
      const marked = this.doc.replace(
        position,
        position,
        new Slice(Fragment.from(this.doc.type.schema.text(marker, resolved.marks())), 0, 0),
      );
      try {
        const change = this.project(marked);
        const insertion = change.edits.find(
          (edit) => edit.fromByte === edit.toByte && edit.insertedText.includes(marker),
        );
        if (insertion)
          return (
            insertion.fromByte +
            utf16ToUtf8(insertion.insertedText, insertion.insertedText.indexOf(marker))
          );
      } catch {
        /* 空尾段或不可内联的原子节点使用已验证的块边界。 */
      }
    }
    if (position >= removeEmptyTrailingParagraph(this.doc).content.size)
      return this.fromByte + utf16ToUtf8(this.source, this.source.length);
    if (position === 0) return this.fromByte;
    const anchors = parseMarkdownWithSyncAnchors(this.source, this.context).anchors;
    const anchor =
      anchors.find((anchor) => anchor.edge === 'start' && anchor.pos === position) ??
      anchors.find((anchor) => anchor.edge === 'end' && anchor.endPos === position);
    if (!anchor) return null;
    let line = 1;
    let offset = 0;
    for (; offset < this.source.length && line < anchor.fromLine; offset++)
      if (this.source[offset] === '\n') line++;
    return this.fromByte + utf16ToUtf8(this.source, offset);
  }

  project(next: ProseMirrorNode): SourceProjectionChange {
    const canonical = serialized(next);
    const changes = getSourceTextChanges(this.canonical, canonical);
    if (!changes.length) return { source: this.source, edits: [] };
    let reparsedCanonical: string | undefined;
    // 行尾空白应插在该段落后、原文已有空行/引用定义前，不能落到被省略的尾部之后。
    const whitespaceOnly = changes.every(change => /^[ \t\r\n]*$/.test(change.insert) &&
      /^[ \t\r\n]*$/.test(this.canonical.slice(change.from, change.to)));
    const boundaries = whitespaceOnly
      ? [[-1, -1], [1, -1], [1, 1], [-1, 1]] as const
      : [[1, -1], [-1, -1], [1, 1], [-1, 1]] as const;
    // 端点可能与被序列化器省略的空行/引用定义相邻；只尝试有限的精确边界组合。
    for (const toSource of [this.toSource, getSourceTextChanges(this.canonical, this.source)]) {
      for (const [startSide, endSide] of boundaries) {
        const patches: TextChange[] = [];
        let valid = true;
        for (const change of changes) {
          const from = mapBoundary(change.from, toSource, startSide);
          const to = mapBoundary(
            change.to,
            toSource,
            change.from === change.to ? startSide : endSide,
          );
          if (
            from === null ||
            to === null ||
            from > to ||
            !completeBoundary(this.source, from) ||
            !completeBoundary(this.source, to)
          ) {
            valid = false;
            break;
          }
          patches.push({
            from,
            to,
            insert: change.insert.replace(/\n/g, newlineAt(this.source, from)),
          });
        }
        if (!valid || patches.some((patch, i) => i > 0 && patch.from < patches[i - 1].to)) continue;
        let source = this.source;
        for (const patch of [...patches].reverse())
          source = source.slice(0, patch.from) + patch.insert + source.slice(patch.to);
        const projected = serialized(parseMarkdown(source, this.context));
        if (projected !== canonical) {
          // PM 编辑态允许段首/段尾空格及空段；Markdown 解析会规范化这些空白。
          // 仅在严格比较不等时校验目标的同一往返语义，仍保留原文补丁及未编辑字节。
          reparsedCanonical ??= serialized(parseMarkdown(canonical, this.context));
          if (projected !== reparsedCanonical) continue;
        }
        return {
          source,
          edits: patches.map((patch) => ({
            fromByte: this.fromByte + utf16ToUtf8(this.source, patch.from),
            toByte: this.fromByte + utf16ToUtf8(this.source, patch.to),
            insertedText: patch.insert,
          })),
        };
      }
    }
    throw new SourceProjectionError();
  }
}
