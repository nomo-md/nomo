import { lift, setBlockType, splitBlockAs, wrapIn } from 'prosemirror-commands';
import { appendClearInlineSourceStyles, toggleInlineSourceFormat } from './inlineSourceCommands';
import { redo, undo } from 'prosemirror-history';
import { liftListItem, wrapInList } from 'prosemirror-schema-list';
import {
  Fragment,
  type Mark,
  type MarkType,
  type Node as PmNode,
  type NodeType,
  type ResolvedPos,
} from 'prosemirror-model';
import {
  EditorState,
  NodeSelection,
  Selection,
  TextSelection,
  type Transaction,
} from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { schema } from './schema';
import { getDiagramTemplate } from './diagramTemplates';
import { CommentBlockNodeView } from './nodeViews/CommentBlockNodeView';
import { CommentInlineNodeView } from './nodeViews/CommentInlineNodeView';
import { MermaidBlockNodeView } from './nodeViews/MermaidBlockNodeView';
import { MathBlockNodeView } from './nodeViews/MathBlockNodeView';
import { createLinkAttrs } from './link';
import { t } from '../../app/i18n';

/**
 * 在当前块的下方插入一个新的空段落，并将光标移入。
 * 逻辑：定位当前顶层块的末尾位置，在该位置插入空段落节点。
 */
function insertParagraphAfterCurrentBlock(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  if (!dispatch) return false;
  const { $from } = state.selection;
  const topDepth = 1;
  const afterPos = $from.after(topDepth);
  const emptyParagraph = schema.nodes.paragraph.create();
  const tr = state.tr.insert(afterPos, emptyParagraph);
  // 光标移到新段落的起始位置
  const newPos = afterPos + 1;
  tr.setSelection(TextSelection.create(tr.doc, newPos));
  dispatch(tr.scrollIntoView());
  return true;
}

/**
 * 在当前块的上方插入一个新的空段落，并将光标移入。
 * 逻辑：定位当前顶层块的起始位置，在该位置之前插入空段落节点。
 */
function insertParagraphBeforeCurrentBlock(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  if (!dispatch) return false;
  const { $from } = state.selection;
  const topDepth = 1;
  const beforePos = $from.before(topDepth);
  const emptyParagraph = schema.nodes.paragraph.create();
  const tr = state.tr.insert(beforePos, emptyParagraph);
  // 光标移到新段落的起始位置
  tr.setSelection(TextSelection.create(tr.doc, beforePos + 1));
  dispatch(tr.scrollIntoView());
  return true;
}
import {
  addTableColumnAfter,
  addTableColumnBefore,
  addTableRowAfter,
  addTableRowBefore,
  createTableNode,
  deleteCurrentTable,
  deleteCurrentTableColumn,
  deleteCurrentTableRow,
  resizeCurrentTable,
  setTableColumnAlignment,
  toggleFirstTableRowHeader,
} from './tableCommands';
import { insertCallout, toggleCalloutType, unwrapCallout } from './callout/calloutCommands';
import type { EditorCommand, SetMarkdownOptions } from './types';
import { createTocList } from '../toc/tocService';
import { ensureFrontMatter } from '../markdown/frontMatter';
import { planOutlineSectionMove } from '../outline/outlineReorder';

type MarkdownSetter = (markdown: string, options?: SetMarkdownOptions) => void;

const CLEAR_INLINE_MARK_NAMES = [
  'strong',
  'em',
  'underline',
  'highlight',
  'strikethrough',
  'link',
  'code',
];
const CLEAR_INLINE_NODE_NAMES = ['math_inline'];

type InlineAtomReplacement = {
  from: number;
  to: number;
  text: string;
};

export type ActiveLinkRange = {
  from: number;
  to: number;
  href: string;
  title: string | null;
  text: string;
  active: boolean;
};

/** 如果当前已是同级标题，则取消为正文；否则设为对应级别 */
function toggleHeading(view: EditorView, level: number): boolean {
  const { $from } = view.state.selection;
  for (let d = $from.depth; d >= 0; d--) {
    const node = $from.node(d);
    if (node.type === schema.nodes.heading) {
      if (node.attrs.level === level) {
        return setBlockType(schema.nodes.paragraph)(view.state, view.dispatch);
      }
      break;
    }
  }
  return setBlockType(schema.nodes.heading, { level })(view.state, view.dispatch);
}

/**
 * 提升标题重要性：+ 键
 * 段落 → H1
 * H6 → H5 → ... → H2 → H1（H1 保持不变）
 */
function increaseHeadingLevel(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  if (!dispatch) return false;
  const { $from } = state.selection;
  const currentNode = $from.parent;

  // 段落转为 H1
  if (currentNode.type === schema.nodes.paragraph) {
    return setBlockType(schema.nodes.heading, { level: 1 })(state, dispatch);
  }

  // 标题提升重要性（数字减小）
  if (currentNode.type === schema.nodes.heading) {
    const currentLevel = currentNode.attrs.level as number;
    // H1 已是最高重要性，保持不变
    if (currentLevel <= 1) return false;
    return setBlockType(schema.nodes.heading, { level: currentLevel - 1 })(state, dispatch);
  }

  return false;
}

/**
 * 降低标题重要性：- 键
 * H1 → H2 → H3 → ... → H6（H6 保持不变）
 * 段落不处理
 */
function decreaseHeadingLevel(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  if (!dispatch) return false;
  const { $from } = state.selection;
  const currentNode = $from.parent;

  // 段落不处理
  if (currentNode.type === schema.nodes.paragraph) {
    return false;
  }

  // 标题降低重要性（数字增大）
  if (currentNode.type === schema.nodes.heading) {
    const currentLevel = currentNode.attrs.level as number;
    // H6 已是最低重要性，保持不变
    if (currentLevel >= 6) return false;
    return setBlockType(schema.nodes.heading, { level: currentLevel + 1 })(state, dispatch);
  }

  return false;
}

/**
 * 自定义 split 命令：在标题内按 Enter 时，拆分出的新块退化为普通段落，
 * 而不是继续继承 heading 的 level 属性。
 * 其他场景（正文、列表项等）沿用 splitBlock 默认行为。
 */
export function splitBlockExitHeading(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  const { $from, empty } = state.selection;
  const isHeadingStart =
    empty && $from.parent.type === schema.nodes.heading && $from.parentOffset === 0;
  const splitCommand = splitBlockAs((node) => {
    // 标题行首回车：让默认拆分逻辑把空白左侧转为正文，并保留右侧原标题。
    if (isHeadingStart && node.type === schema.nodes.heading) {
      return null;
    }

    // 标题中间或末尾回车：强制新块为普通段落。
    if (node.type === schema.nodes.heading) {
      return { type: schema.nodes.paragraph };
    }
    // 其他场景沿用默认行为
    return null;
  });

  if (!isHeadingStart || !dispatch) {
    return splitCommand(state, dispatch);
  }

  // 默认拆分会把光标留在右侧标题开头；派发同一事务前将其移入左侧空段落。
  return splitCommand(state, (tr) => {
    dispatch(tr.setSelection(TextSelection.create(tr.doc, $from.pos)));
  });
}

/**
 * 在当前段落内插入软换行。
 * 软换行用于语义模式的 Shift+Enter：视觉上换行，保存为源码普通单换行。
 */
export function insertSoftLineBreak(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  const { selection } = state;
  const { $from, $to } = selection;

  // 步骤1：只处理同一个 paragraph 内的选择，标题等块交给 Enter 回退逻辑。
  if ($from.parent !== $to.parent || $from.parent.type !== schema.nodes.paragraph) {
    return false;
  }

  const softBreak = schema.nodes.hard_break.create({ soft: true });
  if (dispatch) {
    // 步骤2：选区存在时直接用软换行替换选区，保持“段内换行”语义。
    dispatch(state.tr.replaceSelectionWith(softBreak).scrollIntoView());
  }
  return true;
}

// 查找光标所在位置的父列表（bullet_list 或 ordered_list）
function findParentList($pos: ResolvedPos): { listPos: number; listType: NodeType } | null {
  for (let d = $pos.depth; d >= 0; d--) {
    const node = $pos.node(d);
    if (node.type === schema.nodes.bullet_list || node.type === schema.nodes.ordered_list) {
      return { listPos: $pos.before(d), listType: node.type };
    }
  }
  return null;
}

// 查找包含 $pos 的 list_item 及父列表信息
function findListItem($pos: ResolvedPos): { itemPos: number; itemNode: PmNode } | null {
  for (let d = $pos.depth; d >= 0; d--) {
    const node = $pos.node(d);
    if (node.type === schema.nodes.list_item) {
      return { itemPos: $pos.before(d), itemNode: node };
    }
  }
  return null;
}

// 查找 list_item 中首个文本节点的内容偏移（相对于 list_item 内容起始）
function findFirstTextInItem(item: PmNode): { offset: number } | null {
  let result: { offset: number } | null = null;
  item.descendants((node, pos) => {
    if (node.isText) {
      result = { offset: pos };
      return false;
    }
  });
  return result;
}

// 判断 list_item 首段文本是否以 [ ] 或 [x] 开头
function findTaskMarkerInItem(item: PmNode): { offset: number; length: number } | null {
  let result: { offset: number; length: number } | null = null;
  item.descendants((node) => {
    if (node.isText) {
      const match = /^\[[ x]\]\s?/.exec(node.text ?? '');
      if (match) result = { offset: 0, length: match[0].length };
      return false;
    }
  });
  return result;
}

function getTaskMarkerRange(listItem: {
  itemPos: number;
  itemNode: PmNode;
}): { from: number; to: number } | null {
  const textInfo = findFirstTextInItem(listItem.itemNode);
  const taskMarker = findTaskMarkerInItem(listItem.itemNode);
  if (!textInfo || !taskMarker) return null;
  const from = listItem.itemPos + 1 + textInfo.offset + taskMarker.offset;
  return { from, to: from + taskMarker.length };
}

function liftCurrentListItem(
  state: EditorState,
  dispatch: (tr: Transaction) => void,
  tr?: Transaction,
): boolean {
  if (!tr) return liftListItem(schema.nodes.list_item)(state, dispatch);

  const nextState = state.apply(tr);
  let liftTr: Transaction | null = null;
  const lifted = liftListItem(schema.nodes.list_item)(nextState, (capturedTr) => {
    liftTr = capturedTr;
  });
  if (!lifted || !liftTr) return false;

  for (const step of (liftTr as Transaction).steps) tr.step(step);
  dispatch(tr);
  return true;
}

function getListAttrsForType(listType: NodeType): Record<string, unknown> | null {
  if (listType === schema.nodes.ordered_list) return { order: 1 };
  return null;
}

function appendCommandSteps(
  state: EditorState,
  tr: Transaction,
  command: (state: EditorState, dispatch?: (tr: Transaction) => void) => boolean,
): boolean {
  let capturedTr: Transaction | null = null;
  const handled = command(state.apply(tr), (nextTr) => {
    capturedTr = nextTr;
  });
  if (!handled || !capturedTr) return false;
  for (const step of (capturedTr as Transaction).steps) tr.step(step);
  return true;
}

// 列表切换：同类列表取消为普通段落；跨有序/无序转换时保留任务标记。
// 适配 state/dispatch 签名，可直接用于 ProseMirror keymap
export function toggleList(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
  listType?: NodeType,
): boolean {
  if (!dispatch || !listType) return false;
  const { $from } = state.selection;
  const currentList = findParentList($from);

  if (currentList?.listType === listType) {
    const listItem = findListItem($from);
    const taskRange = listItem ? getTaskMarkerRange(listItem) : null;
    const tr = taskRange ? state.tr.delete(taskRange.from, taskRange.to) : undefined;
    return liftCurrentListItem(state, dispatch, tr);
  }

  if (currentList) {
    dispatch(state.tr.setNodeMarkup(currentList.listPos, listType, getListAttrsForType(listType)));
    return true;
  }

  return wrapInList(listType, getListAttrsForType(listType))(state, dispatch);
}

// 切换任务列表：任务项 → 去掉 [ ]/[x] 并保留列表；普通列表项 → 加上 [ ]；正文 → 创建无序任务列表
// 适配 state/dispatch 签名，可直接用于 ProseMirror keymap
export function toggleTaskListAtCursor(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  if (!dispatch) return false;
  const { $from } = state.selection;
  const listItem = findListItem($from);

  if (listItem) {
    const textInfo = findFirstTextInItem(listItem.itemNode);
    if (!textInfo) return false;
    const textPos = listItem.itemPos + 1 + textInfo.offset;
    const taskRange = getTaskMarkerRange(listItem);

    if (taskRange) {
      dispatch(state.tr.delete(taskRange.from, taskRange.to));
      return true;
    }

    dispatch(state.tr.insertText('[ ] ', textPos));
    return true;
  }

  const textStart = $from.start();
  const tr = state.tr;
  if ($from.parent.type !== schema.nodes.paragraph) {
    if (!appendCommandSteps(state, tr, setBlockType(schema.nodes.paragraph))) return false;
  }
  if (
    !appendCommandSteps(
      state,
      tr,
      wrapInList(schema.nodes.bullet_list, getListAttrsForType(schema.nodes.bullet_list)),
    )
  )
    return false;

  tr.insertText('[ ] ', tr.mapping.map(textStart));
  dispatch(tr);
  return true;
}

/** 切换引用：已在引用内则取消（lift），否则包裹 */
function toggleBlockquote(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  const { $from } = state.selection;
  // 向上查找是否已在 blockquote 内
  for (let d = $from.depth; d >= 0; d--) {
    if ($from.node(d).type === schema.nodes.blockquote) {
      return lift(state, dispatch);
    }
  }
  return wrapIn(schema.nodes.blockquote)(state, dispatch);
}

/**
 * 清除行内样式：只处理文本 mark 和行内原子节点，不改变标题、列表、引用等块级语义。
 */
function clearInlineStyles(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  const { selection } = state;

  if (selection instanceof NodeSelection && isClearableInlineAtom(selection.node)) {
    if (dispatch) {
      const replacementText = getInlineAtomPlainText(selection.node);
      const tr = state.tr;
      replaceRangeWithPlainText(tr, selection.from, selection.to, replacementText);
      const cursor = selection.from + replacementText.length;
      dispatch(
        exitInlinePendingState(tr)
          .setSelection(TextSelection.create(tr.doc, cursor))
          .setStoredMarks([])
          .scrollIntoView(),
      );
    }
    return true;
  }

  if (!selection.empty) {
    return clearInlineStylesInRange(state, dispatch, selection.from, selection.to);
  }

  const sourceTransaction = state.tr;
  if (appendClearInlineSourceStyles(state, sourceTransaction)) {
    const markRange = findClearableMarkRangeAtCursor(state);
    if (markRange) {
      sourceTransaction.removeMark(
        sourceTransaction.mapping.map(markRange.from, -1),
        sourceTransaction.mapping.map(markRange.to, 1),
        markRange.mark.type,
      );
    }
    if (dispatch) {
      dispatch(
        exitInlinePendingState(sourceTransaction)
          .setSelection(createMappedSelection(sourceTransaction, selection.from, selection.to))
          .setStoredMarks([])
          .scrollIntoView(),
      );
    }
    return true;
  }

  const markRange = findClearableMarkRangeAtCursor(state);
  if (markRange) {
    if (dispatch) {
      const tr = state.tr.removeMark(markRange.from, markRange.to, markRange.mark.type);
      dispatch(
        exitInlinePendingState(tr)
          .setSelection(TextSelection.create(tr.doc, clampDocPosition(markRange.cursor, tr.doc)))
          .setStoredMarks([])
          .scrollIntoView(),
      );
    }
    return true;
  }

  const atomReplacement = findAdjacentClearableInlineAtom(state);
  if (atomReplacement) {
    if (dispatch) {
      const tr = state.tr;
      replaceRangeWithPlainText(tr, atomReplacement.from, atomReplacement.to, atomReplacement.text);
      dispatch(
        exitInlinePendingState(tr)
          .setSelection(
            TextSelection.create(tr.doc, atomReplacement.from + atomReplacement.text.length),
          )
          .setStoredMarks([])
          .scrollIntoView(),
      );
    }
    return true;
  }

  if (hasClearableStoredMark(state)) {
    if (dispatch) {
      dispatch(exitInlinePendingState(state.tr).setStoredMarks([]));
    }
    return true;
  }

  return false;
}

function clearInlineStylesInRange(
  state: EditorState,
  dispatch: ((tr: Transaction) => void) | undefined,
  from: number,
  to: number,
): boolean {
  const clearableMarks = findClearableMarksInRange(state, from, to);
  const atomReplacements = findClearableInlineAtomsInRange(state, from, to);
  const tr = state.tr;
  const hasSourceStyles = appendClearInlineSourceStyles(state, tr);
  if (clearableMarks.length === 0 && atomReplacements.length === 0 && !hasSourceStyles) return false;

  if (dispatch) {
    for (const markType of getClearableMarkTypes(state)) {
      tr.removeMark(tr.mapping.map(from, -1), tr.mapping.map(to, 1), markType);
    }

    for (const replacement of atomReplacements.reverse()) {
      replaceRangeWithPlainText(
        tr,
        tr.mapping.map(replacement.from, 1),
        tr.mapping.map(replacement.to, -1),
        replacement.text,
      );
    }

    dispatch(
      exitInlinePendingState(tr)
        .setSelection(createMappedSelection(tr, from, to))
        .setStoredMarks([])
        .scrollIntoView(),
    );
  }
  return true;
}

function findClearableMarksInRange(state: EditorState, from: number, to: number): Mark[] {
  const markTypes = new Set(getClearableMarkTypes(state));
  const marks: Mark[] = [];
  state.doc.nodesBetween(from, to, (node) => {
    if (!node.isText) return true;
    for (const mark of node.marks) {
      if (markTypes.has(mark.type)) marks.push(mark);
    }
    return true;
  });
  return marks;
}

function findClearableInlineAtomsInRange(
  state: EditorState,
  from: number,
  to: number,
): InlineAtomReplacement[] {
  const replacements: InlineAtomReplacement[] = [];
  state.doc.nodesBetween(from, to, (node, pos) => {
    if (!isClearableInlineAtom(node)) return true;
    const nodeTo = pos + node.nodeSize;
    if (pos >= from && nodeTo <= to) {
      replacements.push({ from: pos, to: nodeTo, text: getInlineAtomPlainText(node) });
    }
    return false;
  });
  return replacements;
}

function findClearableMarkRangeAtCursor(
  state: EditorState,
): { from: number; to: number; mark: Mark; cursor: number } | null {
  const pos = state.selection.from;
  const directMarks = state.storedMarks ?? state.selection.$from.marks();
  const directMark = getFirstClearableMark(directMarks);
  if (directMark) {
    const range = findMarkRangeNearPosition(state, pos, directMark);
    if (range) return { ...range, mark: directMark, cursor: pos };
  }

  const rightMark = getFirstClearableMark(state.selection.$from.nodeAfter?.marks ?? []);
  if (rightMark) {
    const range = findMarkRangeNearPosition(state, pos, rightMark);
    if (range) return { ...range, mark: rightMark, cursor: pos };
  }

  const leftMark = getFirstClearableMark(state.selection.$from.nodeBefore?.marks ?? []);
  if (leftMark) {
    const range = findMarkRangeNearPosition(state, Math.max(pos - 1, 0), leftMark);
    if (range) return { ...range, mark: leftMark, cursor: pos };
  }

  return null;
}

function findMarkRangeNearPosition(
  state: EditorState,
  pos: number,
  mark: Mark,
): { from: number; to: number } | null {
  let found: { from: number; to: number } | null = null;
  state.doc.nodesBetween(0, state.doc.content.size, (node, nodePos) => {
    if (found || !node.isText || !mark.isInSet(node.marks)) return true;
    const from = nodePos;
    const to = nodePos + node.nodeSize;
    if (pos >= from && pos <= to) {
      found = expandMarkRange(state.doc, from, to, mark);
      return false;
    }
    return true;
  });
  return found;
}

function expandMarkRange(
  doc: PmNode,
  initialFrom: number,
  initialTo: number,
  mark: Mark,
): { from: number; to: number } {
  let from = initialFrom;
  let to = initialTo;

  doc.nodesBetween(0, doc.content.size, (node, pos) => {
    if (!node.isText || !mark.isInSet(node.marks)) return true;
    const nodeTo = pos + node.nodeSize;
    if (nodeTo === from) {
      from = pos;
      return true;
    }
    if (pos === to) {
      to = nodeTo;
      return true;
    }
    return true;
  });

  return { from, to };
}

function findAdjacentClearableInlineAtom(state: EditorState): InlineAtomReplacement | null {
  const { $from } = state.selection;
  const nodeAfter = $from.nodeAfter;
  if (nodeAfter && isClearableInlineAtom(nodeAfter)) {
    return {
      from: $from.pos,
      to: $from.pos + nodeAfter.nodeSize,
      text: getInlineAtomPlainText(nodeAfter),
    };
  }

  const nodeBefore = $from.nodeBefore;
  if (nodeBefore && isClearableInlineAtom(nodeBefore)) {
    return {
      from: $from.pos - nodeBefore.nodeSize,
      to: $from.pos,
      text: getInlineAtomPlainText(nodeBefore),
    };
  }

  return null;
}

function getClearableMarkTypes(state: EditorState): MarkType[] {
  return CLEAR_INLINE_MARK_NAMES.map((name) => state.schema.marks[name]).filter(
    (markType): markType is MarkType => Boolean(markType),
  );
}

function getFirstClearableMark(marks: readonly Mark[]): Mark | null {
  return marks.find((mark) => CLEAR_INLINE_MARK_NAMES.includes(mark.type.name)) ?? null;
}

function hasClearableStoredMark(state: EditorState): boolean {
  return Boolean(
    state.storedMarks?.some((mark) => CLEAR_INLINE_MARK_NAMES.includes(mark.type.name)),
  );
}

function exitInlinePendingState(tr: Transaction): Transaction {
  return tr.setMeta('inlineSourceTemplate', { action: 'clear' });
}

function isClearableInlineAtom(node: PmNode): boolean {
  return node.isInline && CLEAR_INLINE_NODE_NAMES.includes(node.type.name);
}

function getInlineAtomPlainText(node: PmNode): string {
  if (node.type.name === 'math_inline') return String(node.attrs.tex ?? '');
  return node.textContent;
}

function replaceRangeWithPlainText(tr: Transaction, from: number, to: number, text: string): void {
  if (text) {
    tr.replaceWith(from, to, schema.text(text));
  } else {
    tr.delete(from, to);
  }
}

function createMappedSelection(tr: Transaction, from: number, to: number): Selection {
  const mappedFrom = clampDocPosition(tr.mapping.map(from, -1), tr.doc);
  const mappedTo = clampDocPosition(tr.mapping.map(to, 1), tr.doc);
  try {
    return TextSelection.create(tr.doc, mappedFrom, mappedTo);
  } catch {
    return TextSelection.near(tr.doc.resolve(mappedFrom));
  }
}

function clampDocPosition(position: number, doc: PmNode): number {
  return Math.max(0, Math.min(position, doc.content.size));
}

export function executeEditorCommand(
  command: EditorCommand,
  view: EditorView,
  markdown: string,
  setMarkdown: MarkdownSetter,
): boolean {
  const { state, dispatch } = view;
  const run = (fn: (state: EditorState, dispatch?: (tr: Transaction) => void) => boolean) =>
    fn(state, dispatch);

  switch (command.type) {
    case 'toggleBold':
      return run(toggleInlineSourceFormat('strong'));
    case 'toggleItalic':
      return run(toggleInlineSourceFormat('em'));
    case 'toggleCode':
      return run(toggleInlineSourceFormat('code'));
    case 'toggleStrikethrough':
      return run(toggleInlineSourceFormat('strikethrough'));
    case 'toggleUnderline':
      return run(toggleInlineSourceFormat('underline'));
    case 'toggleHighlight':
      return run(toggleInlineSourceFormat('highlight'));
    case 'clearInlineStyles':
      return run(clearInlineStyles);
    case 'setHeading':
      return toggleHeading(view, command.level);
    case 'setParagraph':
      return run(setBlockType(schema.nodes.paragraph));
    case 'toggleBlockquote':
      return run(toggleBlockquote);
    case 'insertCallout':
      return insertCallout(state, dispatch, command.calloutType ?? 'note');
    case 'toggleCalloutType':
      return toggleCalloutType(state, dispatch, command.calloutType);
    case 'unwrapCallout':
      return unwrapCallout(state, dispatch);
    case 'toggleBulletList':
      return toggleList(state, dispatch, schema.nodes.bullet_list);
    case 'toggleOrderedList':
      return toggleList(state, dispatch, schema.nodes.ordered_list);
    case 'insertLink':
      return applyLinkMark(view, command.href, command.title, command.text);
    case 'removeLink':
      return removeLinkMark(state, dispatch);
    case 'insertImage':
      return insertInlineNode(view, schema.nodes.image, {
        src: command.src,
        alt: command.alt ?? null,
        title: command.title ?? null,
        width: command.width ?? null,
        align: command.align ?? null,
      });
    case 'insertFootnote':
      return insertFootnote(view);
    case 'insertCommentInline':
      return insertInlineComment(view, command.content);
    case 'insertCommentBlock':
      return insertBlockComment(view, command.content);
    case 'insertCodeBlock': {
      // 开关逻辑：已在代码块内则取消（恢复为段落），否则插入新代码块
      const { selection } = state;

      // 检查1：NodeSelection 直接选中了代码块
      if (selection instanceof NodeSelection && selection.node.type === schema.nodes.code_block) {
        const cbPos = selection.from;
        const cbNode = selection.node;
        const cbEnd = cbPos + cbNode.nodeSize;
        const text = cbNode.textContent;
        const lines = text.split('\n');
        const paragraphs: PmNode[] = [];
        const paraType = schema.nodes.paragraph;
        for (const line of lines) {
          paragraphs.push(line ? paraType.create({}, schema.text(line)) : paraType.create());
        }
        const tr = state.tr.replaceWith(cbPos, cbEnd, paragraphs);
        if (text) {
          const fragSize = paragraphs.reduce((s, p) => s + p.nodeSize, 0);
          tr.setSelection(TextSelection.create(tr.doc, cbPos, cbPos + fragSize));
        } else {
          tr.setSelection(TextSelection.near(tr.doc.resolve(cbPos), 1));
        }
        dispatch(tr.scrollIntoView());
        return true;
      }

      // 检查2：光标在代码块内部（通过 depth walk）
      const { $from: cbFrom } = state.selection;
      for (let d = cbFrom.depth; d >= 0; d--) {
        if (cbFrom.node(d).type === schema.nodes.code_block) {
          const cbPos = cbFrom.before(d + 1);
          const cbNode = cbFrom.node(d);
          const cbEnd = cbPos + cbNode.nodeSize;
          const text = cbNode.textContent;
          const lines = text.split('\n');
          const paragraphs: PmNode[] = [];
          const paraType = schema.nodes.paragraph;
          for (const line of lines) {
            paragraphs.push(line ? paraType.create({}, schema.text(line)) : paraType.create());
          }
          const tr = state.tr.replaceWith(cbPos, cbEnd, paragraphs);
          if (text) {
            const fragSize = paragraphs.reduce((s, p) => s + p.nodeSize, 0);
            tr.setSelection(TextSelection.create(tr.doc, cbPos, cbPos + fragSize));
          } else {
            tr.setSelection(TextSelection.near(tr.doc.resolve(cbPos), 1));
          }
          dispatch(tr.scrollIntoView());
          return true;
        }
      }
      // 不在代码块内 → 正常插入（有选中文本时放入代码块）
      const selectedText = state.selection.empty
        ? (command.code ?? '')
        : state.doc.textBetween(state.selection.from, state.selection.to, '\n');
      return insertBlock(view, schema.nodes.code_block, selectedText, {
        params: command.language ?? '',
      });
    }
    case 'toggleTaskList':
      return toggleTaskListAtCursor(state, dispatch);
    case 'insertMathBlock': {
      // 开关逻辑：已在公式块内则取消（恢复为段落），否则插入新公式块
      const { selection } = state;

      // 检查1：NodeSelection 直接选中了公式块
      if (selection instanceof NodeSelection && selection.node.type === schema.nodes.math_block) {
        const mbPos = selection.from;
        const mbNode = selection.node;
        const mbEnd = mbPos + mbNode.nodeSize;
        const tex = mbNode.attrs.tex || '';
        const lines = tex.split('\n');
        const paragraphs: PmNode[] = [];
        const paraType = schema.nodes.paragraph;
        for (const line of lines) {
          paragraphs.push(line ? paraType.create({}, schema.text(line)) : paraType.create());
        }
        const tr = state.tr.replaceWith(mbPos, mbEnd, paragraphs);
        if (tex) {
          const fragSize = paragraphs.reduce((s, p) => s + p.nodeSize, 0);
          tr.setSelection(TextSelection.create(tr.doc, mbPos, mbPos + fragSize));
        } else {
          tr.setSelection(TextSelection.near(tr.doc.resolve(mbPos), 1));
        }
        dispatch(tr.scrollIntoView());
        return true;
      }

      // 检查2：光标在公式块内部（通过 depth walk）
      const { $from: mbFrom } = state.selection;
      for (let d = mbFrom.depth; d >= 0; d--) {
        if (mbFrom.node(d).type === schema.nodes.math_block) {
          const mbPos = mbFrom.before(d + 1);
          const mbNode = mbFrom.node(d);
          const mbEnd = mbPos + mbNode.nodeSize;
          const tex = mbNode.attrs.tex || '';
          const lines = tex.split('\n');
          const paragraphs: PmNode[] = [];
          const paraType = schema.nodes.paragraph;
          for (const line of lines) {
            paragraphs.push(line ? paraType.create({}, schema.text(line)) : paraType.create());
          }
          const tr = state.tr.replaceWith(mbPos, mbEnd, paragraphs);
          if (tex) {
            const fragSize = paragraphs.reduce((s, p) => s + p.nodeSize, 0);
            tr.setSelection(TextSelection.create(tr.doc, mbPos, mbPos + fragSize));
          } else {
            tr.setSelection(TextSelection.near(tr.doc.resolve(mbPos), 1));
          }
          dispatch(tr.scrollIntoView());
          return true;
        }
      }

      // 不在公式块内 → 正常插入（有选中文本时作为 tex 内容）
      const { $from, $to, empty } = state.selection;
      const selectedTex = empty
        ? (command.tex ?? 'E = mc^2')
        : state.doc.textBetween(state.selection.from, state.selection.to, '\n');
      const node = schema.nodes.math_block.create({ tex: selectedTex });
      if ($from.depth <= 1) {
        if (empty && $from.parent.isTextblock && $from.parent.content.size > 0) {
          const insertPos = $from.after(1);
          const tr = state.tr.insert(insertPos, node);
          tr.setSelection(NodeSelection.create(tr.doc, insertPos));
          view.dispatch(tr.scrollIntoView());
          MathBlockNodeView.scheduleEnterEditAt(view, insertPos, 'start');
          return true;
        }

        const blockStart = $from.before(1);
        const blockEnd = $to.after(1);
        const tr = state.tr.replaceWith(blockStart, blockEnd, node);
        tr.setSelection(NodeSelection.create(tr.doc, blockStart));
        view.dispatch(tr.scrollIntoView());
        MathBlockNodeView.scheduleEnterEditAt(view, blockStart, 'start');
      } else {
        const endPos = state.doc.content.size;
        const tr = state.tr.insert(endPos, node);
        tr.setSelection(NodeSelection.create(tr.doc, endPos));
        view.dispatch(tr.scrollIntoView());
        MathBlockNodeView.scheduleEnterEditAt(view, endPos, 'start');
      }
      return true;
    }
    case 'insertMermaidBlock':
      return insertMermaidBlock(view, command.code ?? '', { enterEdit: !command.code });
    case 'insertDiagramBlock':
      return insertMermaidBlock(view, getDiagramTemplate(command.diagramType).code);
    case 'insertToc':
      return insertTocBlock(view, markdown);
    case 'insertFrontMatter':
      return insertFrontMatterBlock(markdown, setMarkdown);
    case 'insertTable': {
      const tableNode = createTableNode(command.rows ?? 3, command.columns ?? 3);
      return insertTable(view, tableNode);
    }
    case 'resizeTable':
      return run(resizeCurrentTable(command.rows, command.columns));
    case 'addTableRowBefore':
      return run(addTableRowBefore());
    case 'addTableRowAfter':
      return run(addTableRowAfter());
    case 'addTableColumnBefore':
      return run(addTableColumnBefore());
    case 'addTableColumnAfter':
      return run(addTableColumnAfter());
    case 'deleteTableRow':
      return run(deleteCurrentTableRow());
    case 'deleteTableColumn':
      return run(deleteCurrentTableColumn());
    case 'deleteTable':
      return run(deleteCurrentTable());
    case 'toggleTableHeader':
      return run(toggleFirstTableRowHeader());
    case 'setTableColumnAlignment':
      return run(setTableColumnAlignment(command.align));
    case 'insertParagraphAfter':
      return insertParagraphAfterCurrentBlock(state, dispatch);
    case 'insertParagraphBefore':
      return insertParagraphBeforeCurrentBlock(state, dispatch);
    case 'increaseHeadingLevel':
      return increaseHeadingLevel(state, dispatch);
    case 'decreaseHeadingLevel':
      return decreaseHeadingLevel(state, dispatch);
    case 'insertHorizontalRule':
      return insertHorizontalRule(view);
    case 'undo':
      return run(undo);
    case 'redo':
      return run(redo);
    case 'moveOutlineSection':
      return moveOutlineSection(view, command);
    case 'scrollToHeading':
      return scrollToHeading(view, command.headingIndex);
    default:
      return false;
  }
}

function moveOutlineSection(
  view: EditorView,
  command: Extract<EditorCommand, { type: 'moveOutlineSection' }>,
): boolean {
  const { state } = view;
  const topLevelNodes: Array<{ node: PmNode; pos: number }> = [];
  const headings: Array<{ childIndex: number; level: number }> = [];
  let pos = 0;
  for (let childIndex = 0; childIndex < state.doc.childCount; childIndex += 1) {
    const node = state.doc.child(childIndex);
    topLevelNodes.push({ node, pos });
    if (node.type === state.schema.nodes.heading) {
      headings.push({ childIndex, level: Number(node.attrs.level) });
    }
    pos += node.nodeSize;
  }

  const planResult = planOutlineSectionMove(headings, command);
  if (!planResult.ok) return false;
  const { plan } = planResult;
  const sourceStartChild = headings[plan.sourceStartIndex].childIndex;
  const sourceEndChild = headings[plan.sourceEndIndex]?.childIndex ?? state.doc.childCount;
  const insertionHeadingIndex =
    command.placement === 'before' ? command.targetIndex : plan.targetEndIndex;
  const insertionChild = headings[insertionHeadingIndex]?.childIndex ?? state.doc.childCount;
  const from = topLevelNodes[sourceStartChild].pos;
  const to = topLevelNodes[sourceEndChild]?.pos ?? state.doc.content.size;
  const insertionPos = topLevelNodes[insertionChild]?.pos ?? state.doc.content.size;
  const movedHeadingLevels = new Map(
    headings
      .slice(plan.sourceStartIndex, plan.sourceEndIndex)
      .map((heading) => [heading.childIndex, heading.level + plan.levelDelta]),
  );
  const movedNodes = topLevelNodes
    .slice(sourceStartChild, sourceEndChild)
    .map(({ node }, offset) => {
      const nextLevel = movedHeadingLevels.get(sourceStartChild + offset);
      if (!nextLevel) return node;
      return node.type.create(
        { ...node.attrs, level: nextLevel },
        node.content,
        node.marks,
      );
    });
  const mappedInsertionPos = insertionPos > from ? insertionPos - (to - from) : insertionPos;
  const tr = state.tr.delete(from, to).insert(mappedInsertionPos, Fragment.fromArray(movedNodes));
  tr.setSelection(TextSelection.near(tr.doc.resolve(mappedInsertionPos + 1)));
  view.dispatch(tr.scrollIntoView());
  return true;
}

function insertFrontMatterBlock(markdown: string, setMarkdown: MarkdownSetter): boolean {
  const nextMarkdown = ensureFrontMatter(markdown);
  if (nextMarkdown !== markdown) {
    setMarkdown(nextMarkdown, { reason: 'programmatic-update' });
  }
  return true;
}

function insertTocBlock(view: EditorView, markdown: string): boolean {
  const node = schema.nodes.toc_block.create({ content: createTocList(markdown) });
  const { state } = view;
  const { $from, empty } = state.selection;

  if (empty && $from.depth === 1 && $from.parent.isTextblock && $from.parent.content.size === 0) {
    const blockStart = $from.before(1);
    const blockEnd = $from.after(1);
    const tr = state.tr.replaceWith(blockStart, blockEnd, node);
    tr.setSelection(NodeSelection.create(tr.doc, blockStart));
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  const tr = state.tr.replaceSelectionWith(node);
  const nodePos = tr.mapping.map(state.selection.from, -1);
  if (tr.doc.nodeAt(nodePos)?.type === schema.nodes.toc_block) {
    tr.setSelection(NodeSelection.create(tr.doc, nodePos));
  }
  view.dispatch(tr.scrollIntoView());
  return true;
}

function scrollToHeading(view: EditorView, headingIndex: number): boolean {
  let foundPos: number | null = null;
  let headingCount = 0;
  view.state.doc.descendants((node, pos) => {
    if (node.type.name === 'heading') {
      headingCount += 1;
      if (headingCount === headingIndex + 1) {
        foundPos = pos;
        return false;
      }
    }
  });

  if (foundPos === null) return false;

  const $pos = view.state.doc.resolve(foundPos);
  view.dispatch(view.state.tr.setSelection(TextSelection.near($pos)).scrollIntoView());
  return true;
}

function insertInlineNode(
  view: EditorView,
  type: NodeType,
  attrs: Record<string, unknown>,
): boolean {
  const node = type.createAndFill(attrs);
  if (!node) return false;

  view.dispatch(view.state.tr.replaceSelectionWith(node).scrollIntoView());
  return true;
}

export function findActiveLinkRange(state: EditorState): ActiveLinkRange | null {
  const markType = state.schema.marks.link;
  if (!markType) return null;

  const { selection } = state;
  if (!selection.empty) {
    const selectedText = state.doc.textBetween(selection.from, selection.to, '\n');
    const firstLink = findFirstLinkMarkInRange(state, selection.from, selection.to);
    if (firstLink) {
      return {
        ...firstLink,
        from: selection.from,
        to: selection.to,
        text: selectedText,
        active: true,
      };
    }
    return {
      from: selection.from,
      to: selection.to,
      href: '',
      title: null,
      text: selectedText,
      active: false,
    };
  }

  const cursor = selection.from;
  const marks = [
    ...(state.storedMarks ?? selection.$from.marks()),
    ...(selection.$from.nodeAfter?.marks ?? []),
    ...(selection.$from.nodeBefore?.marks ?? []),
  ];
  const linkMark = marks.find((mark) => mark.type === markType);
  if (!linkMark) {
    return {
      from: cursor,
      to: cursor,
      href: '',
      title: null,
      text: '',
      active: false,
    };
  }

  const range = findMarkRangeNearPosition(state, cursor, linkMark);
  if (!range) {
    return {
      from: cursor,
      to: cursor,
      href: String(linkMark.attrs.href ?? ''),
      title: linkMark.attrs.title ? String(linkMark.attrs.title) : null,
      text: '',
      active: true,
    };
  }

  return {
    ...range,
    href: String(linkMark.attrs.href ?? ''),
    title: linkMark.attrs.title ? String(linkMark.attrs.title) : null,
    text: state.doc.textBetween(range.from, range.to, '\n'),
    active: true,
  };
}

function findFirstLinkMarkInRange(
  state: EditorState,
  from: number,
  to: number,
): Pick<ActiveLinkRange, 'href' | 'title'> | null {
  let found: Pick<ActiveLinkRange, 'href' | 'title'> | null = null;
  state.doc.nodesBetween(from, to, (node) => {
    if (found || !node.isText) return true;
    const link = node.marks.find((mark) => mark.type === state.schema.marks.link);
    if (link) {
      found = {
        href: String(link.attrs.href ?? ''),
        title: link.attrs.title ? String(link.attrs.title) : null,
      };
      return false;
    }
    return true;
  });
  return found;
}

function applyLinkMark(view: EditorView, href: string, title?: string, text?: string): boolean {
  const attrs = createLinkAttrs(href, title);
  if (!attrs) return false;

  const { state } = view;
  const linkMark = state.schema.marks.link.create(attrs);
  const activeLink = findActiveLinkRange(state);
  const targetText = text?.trim();

  if (activeLink?.active) {
    const replacementText = targetText || activeLink.text || attrs.href;
    const tr = state.tr.removeMark(activeLink.from, activeLink.to, state.schema.marks.link);
    if (replacementText !== activeLink.text) {
      tr.replaceWith(activeLink.from, activeLink.to, schema.text(replacementText, [linkMark]));
      tr.setSelection(
        TextSelection.create(tr.doc, activeLink.from, activeLink.from + replacementText.length),
      );
    } else {
      tr.addMark(activeLink.from, activeLink.to, linkMark);
      tr.setSelection(TextSelection.create(tr.doc, activeLink.from, activeLink.to));
    }
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  if (!state.selection.empty) {
    const selectedText = state.doc.textBetween(state.selection.from, state.selection.to, '\n');
    const replacementText = targetText || selectedText || attrs.href;
    const tr = state.tr.replaceSelectionWith(schema.text(replacementText, [linkMark]), false);
    const from = tr.selection.from - replacementText.length;
    tr.setSelection(TextSelection.create(tr.doc, from, from + replacementText.length));
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  const replacementText = targetText || attrs.href;
  const tr = state.tr.replaceSelectionWith(schema.text(replacementText, [linkMark]), false);
  const from = tr.selection.from - replacementText.length;
  tr.setSelection(TextSelection.create(tr.doc, from, from + replacementText.length));
  view.dispatch(tr.scrollIntoView());
  return true;
}

function removeLinkMark(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  const activeLink = findActiveLinkRange(state);
  if (!activeLink?.active) return false;
  if (dispatch) {
    const tr = state.tr.removeMark(activeLink.from, activeLink.to, state.schema.marks.link);
    dispatch(
      tr
        .setSelection(TextSelection.create(tr.doc, activeLink.from, activeLink.to))
        .scrollIntoView(),
    );
  }
  return true;
}

function insertTable(view: EditorView, tableNode: PmNode): boolean {
  const { state } = view;
  const { $from, empty } = state.selection;

  // 步骤1：空段落中插入表格时，替换整个段落，确保表格起点不会贴到下方旧表格。
  if (empty && $from.depth === 1 && $from.parent.isTextblock && $from.parent.content.size === 0) {
    const tablePos = $from.before(1);
    const blockEnd = $from.after(1);
    const tr = state.tr.replaceWith(tablePos, blockEnd, tableNode);
    setSelectionInFirstTableCell(tr, tablePos);
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  const tr = state.tr.replaceSelectionWith(tableNode);
  const tablePos = tr.mapping.map(state.selection.from, -1);
  if (tr.doc.nodeAt(tablePos)?.type === schema.nodes.table) {
    setSelectionInFirstTableCell(tr, tablePos);
  }
  view.dispatch(tr.scrollIntoView());
  return true;
}

function insertFootnote(view: EditorView): boolean {
  const { state } = view;
  const id = createNextFootnoteId(state.doc);
  const footnoteRef = schema.nodes.footnote_ref.create({ id });
  const footnoteDef = schema.nodes.footnote_def.create({ id });

  let tr = state.tr.replaceSelectionWith(footnoteRef, false);
  const defPos = tr.doc.content.size;
  tr = tr.insert(defPos, footnoteDef);
  tr = tr.setSelection(TextSelection.create(tr.doc, defPos + 1));
  view.dispatch(tr.scrollIntoView());
  return true;
}

function insertInlineComment(view: EditorView, content?: string): boolean {
  const { state } = view;
  const selectedText = state.selection.empty
    ? ''
    : state.doc.textBetween(state.selection.from, state.selection.to, '\n');
  const node = schema.nodes.comment_inline.create({
    content: content ?? (selectedText || t.inlineComment()),
  });
  const tr = state.tr.replaceSelectionWith(node, false);
  const nodePos = tr.mapping.map(state.selection.from, -1);
  if (tr.doc.nodeAt(nodePos)?.type === schema.nodes.comment_inline) {
    tr.setSelection(NodeSelection.create(tr.doc, nodePos));
    CommentInlineNodeView.requestInstantEdit(view);
  }
  view.dispatch(tr.scrollIntoView());
  return true;
}

function insertBlockComment(view: EditorView, content?: string): boolean {
  const { state } = view;
  const { $from, $to, empty } = state.selection;
  const selectedText = empty
    ? ''
    : state.doc.textBetween(state.selection.from, state.selection.to, '\n');
  const node = schema.nodes.comment_block.create({
    content: content ?? (selectedText || t.blockComment()),
  });

  let tr: Transaction;
  let nodePos: number;
  if (empty && $from.depth === 1 && $from.parent.isTextblock && $from.parent.content.size === 0) {
    nodePos = $from.before(1);
    tr = state.tr.replaceWith(nodePos, $from.after(1), node);
  } else if (empty && $from.depth === 1 && $from.parent.isTextblock) {
    nodePos = $from.after(1);
    tr = state.tr.insert(nodePos, node);
  } else if ($from.depth === 1 && $to.depth === 1) {
    nodePos = $from.before(1);
    tr = state.tr.replaceWith(nodePos, $to.after(1), node);
  } else {
    nodePos = state.doc.content.size;
    tr = state.tr.insert(nodePos, node);
  }

  tr.setSelection(NodeSelection.create(tr.doc, nodePos));
  CommentBlockNodeView.requestInstantEdit(view);
  view.dispatch(tr.scrollIntoView());
  return true;
}

function createNextFootnoteId(doc: PmNode): string {
  let maxNumericId = 0;
  doc.descendants((node) => {
    if (node.type.name !== 'footnote_ref' && node.type.name !== 'footnote_def') {
      return true;
    }

    const id = String(node.attrs.id ?? '');
    if (/^\d+$/.test(id)) {
      maxNumericId = Math.max(maxNumericId, Number(id));
    }
    return true;
  });
  return String(maxNumericId + 1);
}

function setSelectionInFirstTableCell(tr: Transaction, tablePos: number): void {
  const table = tr.doc.nodeAt(tablePos);
  if (!table || table.type !== schema.nodes.table) return;

  let textPos: number | null = null;
  table.descendants((node, pos) => {
    if (textPos === null && node.isTextblock) {
      textPos = tablePos + 1 + pos + 1;
      return false;
    }
    return true;
  });

  if (textPos !== null) {
    tr.setSelection(TextSelection.create(tr.doc, textPos));
  }
}

function insertBlock(
  view: EditorView,
  type: NodeType,
  text: string,
  attrs?: Record<string, unknown>,
): boolean {
  const content = text ? schema.text(text) : undefined;
  const node = type.create(attrs, content);
  const { state } = view;
  const { $from, empty } = state.selection;

  // 步骤1：空段落插入块时，直接替换整个段落。
  // 这样能拿到新节点的精确起点，避免相邻代码块之间用近似位置误进入旧块。
  if (empty && $from.depth === 1 && $from.parent.isTextblock && $from.parent.content.size === 0) {
    const blockStart = $from.before(1);
    const blockEnd = $from.after(1);
    const tr = state.tr.replaceWith(blockStart, blockEnd, node);
    if (type.name === 'code_block' || type.name === 'mermaid_block') {
      tr.setSelection(NodeSelection.create(tr.doc, blockStart));
    }
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  // 步骤2：正文段落中空选区插入块时，保留当前正文，把新块插到当前顶层块下方。
  // 这样工具栏插入代码块不会误删用户正在写的段落内容。
  if (empty && $from.depth === 1 && $from.parent.isTextblock) {
    const insertPos = $from.after(1);
    const tr = state.tr.insert(insertPos, node);
    if (type.name === 'code_block' || type.name === 'mermaid_block') {
      tr.setSelection(NodeSelection.create(tr.doc, insertPos));
    }
    view.dispatch(tr.scrollIntoView());
    return true;
  }

  const tr = state.tr.replaceSelectionWith(node);
  const newPos = tr.mapping.map(state.selection.from, -1);
  if (
    (type.name === 'code_block' || type.name === 'mermaid_block') &&
    tr.doc.nodeAt(newPos)?.type === type
  ) {
    tr.setSelection(NodeSelection.create(tr.doc, newPos));
  }
  view.dispatch(tr.scrollIntoView());
  return true;
}

function insertMermaidBlock(
  view: EditorView,
  code: string,
  options: { enterEdit?: boolean } = {},
): boolean {
  const node = schema.nodes.mermaid_block.create({ code });
  const { state } = view;
  const { selection } = state;
  const { $from, empty } = selection;

  // 步骤0：当前选中的是一个块级节点时，把图表插到该块后方，避免误删公式块、表格等内容。
  if (selection instanceof NodeSelection && selection.node.isBlock) {
    const insertPos = selection.to;
    const tr = state.tr.insert(insertPos, node);
    setSelectionAfterMermaidBlock(tr, insertPos, node);
    view.dispatch(tr.scrollIntoView());
    enterMermaidEditWhenReady(view, insertPos, options);
    return true;
  }

  // 步骤1：空段落里插入图表时，直接替换当前段落，并补一个后续输入段落。
  // Mermaid 常态只负责渲染，不能停留在 NodeSelection 选中态。
  if (empty && $from.depth === 1 && $from.parent.isTextblock && $from.parent.content.size === 0) {
    const blockStart = $from.before(1);
    const blockEnd = $from.after(1);
    const tr = state.tr.replaceWith(blockStart, blockEnd, node);
    setSelectionAfterMermaidBlock(tr, blockStart, node);
    view.dispatch(tr.scrollIntoView());
    enterMermaidEditWhenReady(view, blockStart, options);
    return true;
  }

  // 步骤2：正文段落中插入图表时，保留原文，把图表放到当前块下方。
  if (empty && $from.depth === 1 && $from.parent.isTextblock) {
    const insertPos = $from.after(1);
    const tr = state.tr.insert(insertPos, node);
    setSelectionAfterMermaidBlock(tr, insertPos, node);
    view.dispatch(tr.scrollIntoView());
    enterMermaidEditWhenReady(view, insertPos, options);
    return true;
  }

  // 步骤3：有选区时用图表替换选中内容，随后仍把光标放到图表后方。
  const tr = state.tr.replaceSelectionWith(node);
  const insertedPos = tr.mapping.map(state.selection.from, -1);
  if (tr.doc.nodeAt(insertedPos)?.type === schema.nodes.mermaid_block) {
    setSelectionAfterMermaidBlock(tr, insertedPos, node);
  }
  view.dispatch(tr.scrollIntoView());
  enterMermaidEditWhenReady(view, insertedPos, options);
  return true;
}

function enterMermaidEditWhenReady(
  view: EditorView,
  pos: number,
  options: { enterEdit?: boolean },
): void {
  if (!options.enterEdit) return;
  if (MermaidBlockNodeView.enterEditAt(view, pos, 'start')) return;
  if (MermaidBlockNodeView.enterClosestEditAt(view, pos, 'start')) return;
  requestAnimationFrame(() => {
    if (MermaidBlockNodeView.enterEditAt(view, pos, 'start')) return;
    MermaidBlockNodeView.enterClosestEditAt(view, pos, 'start');
  });
}

function setSelectionAfterMermaidBlock(
  tr: Transaction,
  blockPos: number,
  mermaidNode: PmNode,
): void {
  const afterPos = blockPos + mermaidNode.nodeSize;
  const nextNode = tr.doc.nodeAt(afterPos);

  if (nextNode?.type === schema.nodes.paragraph) {
    tr.setSelection(TextSelection.create(tr.doc, afterPos + 1));
    return;
  }

  tr.insert(afterPos, schema.nodes.paragraph.create());
  tr.setSelection(TextSelection.create(tr.doc, afterPos + 1));
}

/**
 * 插入/取消水平分割线。
 *
 * 行为：
 * - 直接选中了水平分割线：删除它。
 * - 光标在空段落且紧邻水平分割线：删除相邻的水平分割线（开关逻辑）。
 * - 否则：在当前块位置插入 HR，并在下方新建空段落聚焦。
 */
function insertHorizontalRule(view: EditorView): boolean {
  const { state, dispatch } = view;
  const { selection } = state;

  // 1. 直接选中水平分割线：删除
  if (selection instanceof NodeSelection && selection.node.type === schema.nodes.horizontal_rule) {
    const tr = state.tr.delete(selection.from, selection.to);
    const $pos = tr.doc.resolve(clampDocPosition(selection.from, tr.doc));
    tr.setSelection(TextSelection.near($pos, 1));
    dispatch(tr.scrollIntoView());
    return true;
  }

  const { $from, empty } = selection;
  const hrNode = schema.nodes.horizontal_rule.create();
  const emptyParagraph = schema.nodes.paragraph.create();

  // 2. 光标在空段落且紧邻水平分割线：取消相邻的分割线
  if (
    empty &&
    $from.depth === 1 &&
    $from.parent.type === schema.nodes.paragraph &&
    $from.parent.content.size === 0
  ) {
    const currentIndex = $from.index(0);
    const docNode = $from.doc;

    // 优先取消前一条水平分割线
    if (currentIndex > 0) {
      const prevNode = docNode.child(currentIndex - 1);
      if (prevNode.type === schema.nodes.horizontal_rule) {
        const hrFrom = $from.before(1) - prevNode.nodeSize;
        const tr = state.tr.delete(hrFrom, $from.before(1));
        tr.setSelection(TextSelection.create(tr.doc, hrFrom + 1));
        dispatch(tr.scrollIntoView());
        return true;
      }
    }

    // 其次取消后一条水平分割线
    if (currentIndex < docNode.childCount - 1) {
      const nextNode = docNode.child(currentIndex + 1);
      if (nextNode.type === schema.nodes.horizontal_rule) {
        const hrFrom = $from.after(1);
        const tr = state.tr.delete(hrFrom, hrFrom + nextNode.nodeSize);
        tr.setSelection(TextSelection.create(tr.doc, $from.start(1)));
        dispatch(tr.scrollIntoView());
        return true;
      }
    }

    // 当前是空段落，但附近没有分割线：替换为 HR + 空段落
    const blockStart = $from.before(1);
    const blockEnd = $from.after(1);
    const tr = state.tr.replaceWith(blockStart, blockEnd, [hrNode, emptyParagraph]);
    tr.setSelection(TextSelection.create(tr.doc, blockStart + hrNode.nodeSize + 1));
    dispatch(tr.scrollIntoView());
    return true;
  }

  // 3. 默认：在当前位置后插入 HR + 空段落
  const insertPos = $from.after(1);
  const tr = state.tr.insert(insertPos, [hrNode, emptyParagraph]);
  tr.setSelection(TextSelection.create(tr.doc, insertPos + hrNode.nodeSize + 1));
  dispatch(tr.scrollIntoView());
  return true;
}
