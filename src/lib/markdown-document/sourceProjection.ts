import { Fragment, Slice, type Node as ProseMirrorNode } from 'prosemirror-model';
import {
  parseMarkdown,
  parseMarkdownWithSyncAnchors,
  serializeMarkdown,
  type MarkdownParseContext,
} from '../editor-core/markdown';
import { removeEmptyTrailingParagraph } from '../editor-core/plugins/trailingParagraph';
import { getSourceTextChanges } from './textChanges';
import { utf16ToUtf8 } from './utf8Text';
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
    let found: number | null = null;
    this.doc.descendants((node, start) => {
      if (found !== null || !node.isText) return found === null;
      let low = start;
      let high = start + node.nodeSize;
      while (low <= high) {
        let middle = Math.floor((low + high) / 2);
        const textOffset = middle - start;
        if (
          textOffset > 0 &&
          /[\uD800-\uDBFF]/.test(node.text![textOffset - 1]) &&
          /[\uDC00-\uDFFF]/.test(node.text![textOffset] ?? '')
        )
          middle--;
        const mapped = this.positionToByte(middle);
        if (mapped === byte) {
          found = middle;
          break;
        }
        if (mapped === null) break;
        if (mapped < byte)
          low = Math.max(
            low + 1,
            middle + (node.text!.codePointAt(middle - start)! > 0xffff ? 2 : 1),
          );
        else high = middle - 1;
      }
      return false;
    });
    return found;
  }

  positionToByte(position: number): number | null {
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
    // 端点可能与被序列化器省略的空行/引用定义相邻；只尝试有限的精确边界组合。
    for (const toSource of [this.toSource, getSourceTextChanges(this.canonical, this.source)]) {
      for (const [startSide, endSide] of [
        [1, -1],
        [-1, -1],
        [1, 1],
        [-1, 1],
      ] as const) {
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
        if (serialized(parseMarkdown(source, this.context)) !== canonical) continue;
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
