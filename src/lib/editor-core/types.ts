import type { DiagramType } from './diagramTemplates';
import type { ImageContext } from '../services/render';
import type { ContextMenuOpenEvent, ContextMenuTarget } from './plugins/contextMenu';
import type { EditorThemeOptions } from '../theme/types';
import type { EditorSyncCaret, EditorSyncSnapshot, MarkdownSyncAnchor } from './scrollSyncMapping';

export type { EditorThemeOptions } from '../theme/types';

export type EditorMode = 'semantic' | 'source';
export type InlinePendingMarkName =
  | 'strong'
  | 'em'
  | 'code'
  | 'strikethrough'
  | 'underline'
  | 'highlight';
export type InlinePendingMarks = Record<InlinePendingMarkName, boolean>;

export interface EditorRuntimeOptions {
  readonly: boolean;
  mode: EditorMode;
  inlineCodeRenderingEnabled?: boolean;
  copyMarkdownSyntaxEnabled: boolean;
}

export interface SetMarkdownOptions {
  preserveHistory?: boolean;
  reason?:
    | 'open-file'
    | 'save-file'
    | 'switch-tab'
    | 'restore-snapshot'
    | 'programmatic-update'
    | 'source-input';
  dirty?: boolean;
  savedMarkdown?: string;
  sourceInput?: boolean;
}

export interface EditorSelectionSnapshot {
  anchor: number;
  head: number;
}

export interface EditorLinkSnapshot {
  href: string;
  title: string | null;
  text: string;
  from: number;
  to: number;
  active: boolean;
}

export interface EditorAnchorRect {
  x: number;
  y: number;
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
  toJSON(): Record<string, number>;
}

export interface EditorSearchOptions {
  caseSensitive: boolean;
  wholeWord?: boolean;
}

export interface EditorSearchMatch {
  id: string;
  index: number;
  from: number;
  to: number;
  text: string;
}

export interface EditorSnapshot {
  markdown: string;
  version: number;
  selection?: EditorSelectionSnapshot;
  meta?: Record<string, unknown>;
}

export interface EditorChangeEvent {
  contentRevision?: number;
  markdown: string;
  version: number;
  dirty: boolean;
  mode: EditorMode;
  readonly: boolean;
  reason: 'content-pending' | 'content-sync' | string;
  pendingInlineMarks: InlinePendingMarks;
}

export interface EditorImageDeletionEvent {
  srcs: string[];
}

export interface EditorSelectionEvent {
  selection: EditorSelectionSnapshot | null;
  selectedMarkdown: string;
  caret?: EditorSyncCaret;
}

export interface EditorSelectionSnapshotEvent {
  selection: EditorSelectionSnapshot | null;
  contentRevision: number;
}

export interface EditorError {
  code: string;
  message: string;
  cause?: unknown;
}

export interface EditorClipboardPayload {
  text: string;
  html: string;
}

export type EditorPasteMode = 'auto' | 'plain';

export interface EditorPasteInput {
  text?: string;
  html?: string;
}

export type EditorPasteResult =
  | { status: 'inserted'; format: 'markdown' | 'html' | 'plain' }
  | { status: 'rejected'; reason: 'readonly' | 'no-text' };

export type EditorCommand =
  | { type: 'toggleBold' }
  | { type: 'toggleItalic' }
  | { type: 'toggleCode' }
  | { type: 'toggleStrikethrough' }
  | { type: 'toggleUnderline' }
  | { type: 'toggleHighlight' }
  | { type: 'clearInlineStyles' }
  | { type: 'setHeading'; level: 1 | 2 | 3 | 4 | 5 | 6 }
  | { type: 'setParagraph' }
  | { type: 'toggleBlockquote' }
  | { type: 'insertCallout'; calloutType?: 'note' | 'tip' | 'important' | 'warning' | 'caution' }
  | {
      type: 'toggleCalloutType';
      calloutType?: 'note' | 'tip' | 'important' | 'warning' | 'caution';
    }
  | { type: 'unwrapCallout' }
  | { type: 'toggleBulletList' }
  | { type: 'toggleOrderedList' }
  | { type: 'toggleTaskList' }
  | { type: 'insertLink'; href: string; title?: string; text?: string }
  | { type: 'removeLink' }
  | {
      type: 'insertImage';
      src: string;
      alt?: string;
      title?: string;
      width?: string | null;
      align?: 'left' | 'center' | 'right' | null;
    }
  | { type: 'insertFootnote' }
  | { type: 'insertCommentInline'; content?: string }
  | { type: 'insertCommentBlock'; content?: string }
  | { type: 'insertCodeBlock'; language?: string; code?: string }
  | { type: 'insertMathBlock'; tex?: string }
  | { type: 'insertMermaidBlock'; code?: string }
  | { type: 'insertDiagramBlock'; diagramType: DiagramType }
  | { type: 'insertToc' }
  | { type: 'insertFrontMatter' }
  | { type: 'insertTable'; rows?: number; columns?: number }
  | { type: 'resizeTable'; rows: number; columns: number }
  | { type: 'addTableRowBefore' }
  | { type: 'addTableRowAfter' }
  | { type: 'addTableColumnBefore' }
  | { type: 'addTableColumnAfter' }
  | { type: 'deleteTableRow' }
  | { type: 'deleteTableColumn' }
  | { type: 'deleteTable' }
  | { type: 'toggleTableHeader' }
  | { type: 'setTableColumnAlignment'; align: 'left' | 'center' | 'right' }
  | { type: 'insertParagraphAfter' }
  | { type: 'insertParagraphBefore' }
  | { type: 'increaseHeadingLevel' }
  | { type: 'decreaseHeadingLevel' }
  | { type: 'undo' }
  | { type: 'redo' }
  | { type: 'formatDocument' }
  | { type: 'insertHorizontalRule' }
  | {
      type: 'moveOutlineSection';
      sourceIndex: number;
      targetIndex: number;
      placement: 'before' | 'inside' | 'after';
    }
  | { type: 'scrollToHeading'; headingIndex: number; text: string; level: number };

export type EditorListener = (event: EditorChangeEvent) => void;

export interface EditorCoreOptions {
  target?: HTMLElement;
  markdown: string;
  readonly?: boolean;
  mode?: EditorMode;
  inlineCodeRenderingEnabled?: boolean;
  copyMarkdownSyntaxEnabled?: boolean;
  theme?: EditorThemeOptions;
  onChange?: (event: EditorChangeEvent) => void;
  onSelectionChange?: (event: EditorSelectionEvent) => void;
  onSelectionSnapshotChange?: (event: EditorSelectionSnapshotEvent) => void;
  onError?: (error: EditorError) => void;
  onLinkShortcut?: () => void;
  onOpenLink?: (href: string) => void;
  getImageContext?: () => ImageContext;
  onImagesDeleted?: (event: EditorImageDeletionEvent) => void;
  /** 上下文菜单打开回调（NodeView 右键） */
  onContextMenuOpen?: (event: ContextMenuOpenEvent) => void;
}

export interface EditorCore {
  getDocumentStatsSnapshot(): Record<string, unknown> | null;
  getSelectionStatsSnapshot(): EditorSelectionSnapshot | null;
  mount(target: HTMLElement): void;
  /** 提交 NodeView 临时输入，刷新正文，但保留文档编辑历史。 */
  commitPendingEdits(): void;
  /** 挂起视图以便同一文档稍后重新挂载；最终关闭才调用 destroy。 */
  unmount(): void;
  destroy(): void;
  getMarkdown(): string;
  flushMarkdown(): string;
  refreshSemanticView(): void;
  getScrollSyncSnapshot(): EditorSyncSnapshot;
  getScrollSyncAnchorRect(anchor: MarkdownSyncAnchor): { top: number; bottom: number } | null;
  getScrollSyncCaret(): EditorSyncCaret | null;
  setMarkdown(markdown: string, options?: SetMarkdownOptions): void;
  setDirty(dirty: boolean): void;
  /** 保存完成时只更新该次写入的基线，不覆盖保存期间的新输入。 */
  setSavedMarkdownBaseline(markdown: string): void;
  getSnapshot(): EditorSnapshot;
  restoreSnapshot(snapshot: EditorSnapshot): void;
  /** 只恢复光标/选区，保持正文、撤销历史与保存基线。 */
  restoreSelectionSnapshot(selection: EditorSelectionSnapshot): void;
  focus(): void;
  blur(): void;
  getActiveLink(): EditorLinkSnapshot | null;
  getSelectionAnchorRect(): EditorAnchorRect | null;
  getClipboardPayload(): EditorClipboardPayload | null;
  pasteClipboard(input: EditorPasteInput, options?: { mode?: EditorPasteMode }): EditorPasteResult;
  pasteClipboardText(text: string): boolean;
  pasteClipboardHtml(html: string): boolean;
  deleteSelection(): boolean;
  selectAll(): boolean;
  selectContextTarget(target: ContextMenuTarget): boolean;
  editContextTarget(target: ContextMenuTarget): boolean;
  chooseContextTargetLanguage(target: ContextMenuTarget): boolean;
  deleteContextTarget(target: ContextMenuTarget): boolean;
  findSearchMatches(query: string, options: EditorSearchOptions): EditorSearchMatch[];
  setSearchHighlights(matches: EditorSearchMatch[], activeIndex: number): void;
  clearSearchState?(activeMatch?: EditorSearchMatch): void;
  selectSearchMatch(match: EditorSearchMatch, focus?: boolean): boolean;
  revealMarkdownLine(lineNumber: number): boolean;
  /** 返回当前语义文档的顶层块数量，不读取或暴露 ProseMirror 内部状态。 */
  getBlockAlignmentBlockCount(): number;
  /** 对齐几何统一使用浏览器视口 CSS 像素。 */
  getBlockAlignmentGeometry(
    anchors: Array<{ key: string; nodeIndex: number }>,
  ): Array<{ key: string; top: number; nextTop: number; existingGap: number }>;
  /** 接收浏览器视口 CSS 像素，由实现内部换算为编辑器局部高度。 */
  applyBlockAlignmentGaps(gaps: Array<{ key: string; nodeIndex: number; height: number }>): void;
  clearBlockAlignmentGaps(): void;
  replaceSearchMatch(match: EditorSearchMatch, replacement: string): boolean;
  replaceAllSearchMatches(query: string, replacement: string, options: EditorSearchOptions): number;
  execute(command: EditorCommand): boolean;
  canExecute(command: EditorCommand): boolean;
  updateTheme(theme: EditorThemeOptions): void;
  updateOptions(options: Partial<EditorRuntimeOptions>): void;
  subscribe(listener: EditorListener): () => void;
  /** 判断指定行内格式是否处于 pending 状态 */
  isPendingMarkActive?(markName: InlinePendingMarkName): boolean;
}
