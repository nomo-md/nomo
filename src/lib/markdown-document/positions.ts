import type { DocumentSelection, SourceEdit, SourceRange } from './protocol';
import { utf8Length } from './utf8Text';

/** 补丁中的位置属于同一个旧版本；边界吸附规则必须由调用方明确指定。 */
export function mapMarkdownByte(
  position: number,
  edits: readonly SourceEdit[],
  association: -1 | 1 = 1,
) {
  let delta = 0;
  for (const edit of edits) {
    if (position < edit.fromByte) break;
    const inserted = utf8Length(edit.insertedText);
    if (position > edit.toByte || (position === edit.toByte && edit.fromByte !== edit.toByte)) {
      delta += inserted - (edit.toByte - edit.fromByte);
      continue;
    }
    return edit.fromByte + delta + (association > 0 ? inserted : 0);
  }
  return position + delta;
}

export function mapMarkdownSelection(
  selection: DocumentSelection,
  edits: readonly SourceEdit[],
): DocumentSelection {
  if (selection.anchorByte === selection.headByte) {
    const byte = mapMarkdownByte(selection.headByte, edits, 1);
    return { anchorByte: byte, headByte: byte };
  }
  const forward = selection.anchorByte <= selection.headByte;
  return {
    anchorByte: mapMarkdownByte(selection.anchorByte, edits, forward ? -1 : 1),
    headByte: mapMarkdownByte(selection.headByte, edits, forward ? 1 : -1),
  };
}

/** 边界上的插入可能改变段落、列表或围栏归属，也必须由结构重扫确认。 */
export function editsTouchRange(edits: readonly SourceEdit[], range: SourceRange) {
  return edits.some((edit) => edit.fromByte <= range.toByte && edit.toByte >= range.fromByte);
}
