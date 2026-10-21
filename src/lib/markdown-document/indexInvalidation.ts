import type { SourceEdit } from './protocol';
import { utf8Length } from './utf8Text';

export interface MarkdownIndexInvalidation {
  fromByte: number;
  toByte: number;
  byteDelta: number;
}

/** 将连续版本的输入折合为旧索引中的保守范围，无须保存或复制正文。 */
export function invalidateMarkdownIndex(
  previous: MarkdownIndexInvalidation | undefined,
  edits: readonly SourceEdit[],
): MarkdownIndexInvalidation | undefined {
  if (!edits.length) return previous;
  const toOriginal = (position: number, end: boolean) => {
    if (!previous || position <= previous.fromByte) return position;
    if (position >= previous.toByte + previous.byteDelta) return position - previous.byteDelta;
    return end ? previous.toByte : previous.fromByte;
  };
  return {
    fromByte: edits.reduce(
      (from, edit) => Math.min(from, toOriginal(edit.fromByte, false)),
      previous?.fromByte ?? Infinity,
    ),
    toByte: edits.reduce(
      (to, edit) => Math.max(to, toOriginal(edit.toByte, true)),
      previous?.toByte ?? 0,
    ),
    byteDelta:
      (previous?.byteDelta ?? 0) +
      edits.reduce(
        (sum, edit) => sum + utf8Length(edit.insertedText) - (edit.toByte - edit.fromByte),
        0,
      ),
  };
}
