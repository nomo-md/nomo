import type {
  BlockAlignmentAnchor,
  BlockNaturalGeometry,
} from '../services/markdownBlockAlignment';
import { history } from '@codemirror/commands';
import { EditorState, StateEffect, Transaction } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

let sourceEditorTheme: ReturnType<typeof EditorView.theme> | undefined;

/** style-mod 在文档级保留已挂载规则；同一静态主题只创建一次，首次使用源码时再初始化。 */
export function getSourceEditorTheme() {
  return sourceEditorTheme ??= EditorView.theme({
    '&': { height: '100%', backgroundColor: 'transparent' },
    '&.cm-focused': { outline: 'none' },
    '.cm-scroller': {
      overflow: 'auto', fontFamily: 'var(--md-editor-font-mono)', fontSize: '14px', lineHeight: '1.75',
    },
    '.cm-content': {
      minHeight: 'var(--md-editor-content-min-height, 0px)', padding: '0', caretColor: 'var(--md-editor-fg)',
    },
    '.cm-line': { padding: '0' },
    '.cm-cursor': { borderLeftColor: 'var(--md-editor-fg)' },
    '.cm-selectionBackground': { backgroundColor: 'color-mix(in srgb, var(--md-editor-accent) 22%, transparent)' },
  });
}

/** 只保留文档、选区与撤销字段，不能把组件回调、布局 widget 或视图插件带入标签缓存。 */
export function detachSourceEditorState(state: EditorState): EditorState {
  return state.update({
    effects: StateEffect.reconfigure.of(history()),
    annotations: Transaction.addToHistory.of(false),
  }).state;
}

/** 仅在本窗口按文档缓存，不序列化到工作区或跨窗口移交。 */
export interface MarkdownSourceRuntimeState {
  readonly documentId: string;
  readonly state: EditorState;
  readonly scrollTop: number;
  readonly scrollLeft: number;
  readonly contentRevision: number;
}

export interface MarkdownSourceSelection {
  from: number;
  to: number;
  anchor?: number;
  head?: number;
}

export interface MarkdownSourceSelectionSnapshot extends MarkdownSourceSelection {
  documentId: string;
  contentRevision: number;
}

export { getSourceTextChanges } from '../../lib/markdown-document/textChanges';

export interface MarkdownSourceEditorHandle {
  getRuntimeState?(): MarkdownSourceRuntimeState;
  restoreRuntimeState?(state: MarkdownSourceRuntimeState): void;
  getMarkdown(): string;
  setMarkdown(markdown: string, options?: { addToHistory?: boolean }): void;
  getSelection(): MarkdownSourceSelection;
  replaceRange?(from: number, to: number, text: string, expectedRevision?: number): boolean;
  getSelectionStatsSnapshot?(): MarkdownSourceSelectionSnapshot;
  setSelection(from: number, to?: number): void;
  /** 按视口 CSS 坐标定位，不创建视图或写入撤销历史。 */
  placeCaretAtCoordinates(coordinates: { left: number; top: number }): boolean;
  getSelectedMarkdown(): string;
  focus(options?: FocusOptions): void;
  revealRange(from: number, to?: number): void;
  undo(): boolean;
  redo(): boolean;
  lineAtOffset(offset: number): number;
  offsetAtLine(lineNumber: number): number;
  getLineCount(): number;
  /** 与本编辑器 scrollDOM.scrollTop 相同的未缩放坐标，不能直接与屏幕坐标混用。 */
  getLineTop(lineNumber: number): number;
  lineAtHeight(height: number): number;
  getLineHeight(): number;
  getSyncCaretTop?(): number;
  isComposing?(): boolean;
  /** 当前内容的 CodeMirror 测量和视口锚点调整已完成。 */
  isLayoutReady?(): boolean;
  getScrollElement(): HTMLElement;
  getContentElement(): HTMLElement;
  getContentHeight(): number;
  getBlockGeometry(anchors: BlockAlignmentAnchor[]): BlockNaturalGeometry[];
  applyBlockGaps(
    anchors: BlockAlignmentAnchor[],
    gaps: ReadonlyMap<string, number>,
    leadingGap?: number,
  ): void;
  clearBlockGaps(): void;
  requestMeasure(): Promise<void> | void;
}
