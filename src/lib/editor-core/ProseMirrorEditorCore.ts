import { serializeClipboardText } from './markdownSerialization';
import { MarkdownSourceProjection } from '../markdown-document/sourceProjection';
import type { DocumentSelection } from '../markdown-document/protocol';
import { typographyPlugin, isTypographyTransaction } from './plugins/typography';
import { typographyBreaksPlugin } from './nodeViews/TypographyBreakNodeView';
import katex from 'katex';
import {
  chainCommands,
  createParagraphNear,
  deleteSelection,
  joinBackward,
  joinForward,
  liftEmptyBlock,
  newlineInCode,
  selectAll as selectAllCommand,
  selectNodeBackward,
  selectNodeForward,
} from 'prosemirror-commands';
import { history, redo, undo } from 'prosemirror-history';
import { inputRules } from 'prosemirror-inputrules';
import { dismissQuickInsert, quickInsertPlugin } from './plugins/quickInsert';
import { emojiSuggestionsPlugin } from './plugins/emojiSuggestions';
import { DEFAULT_QUICK_INSERT_TRIGGER, isQuickInsertTrigger } from './quickInsert';
import { keymap } from 'prosemirror-keymap';
import { EditorState, Selection, NodeSelection, TextSelection, type Transaction } from 'prosemirror-state';
import { Slice, Fragment, DOMSerializer, DOMParser, type Node as ProseMirrorNode, type ResolvedPos } from 'prosemirror-model';
import { EditorView } from 'prosemirror-view';
import { liftListItem, splitListItem } from 'prosemirror-schema-list';
import {
  convertMarkdownListShortcut,
  createPastedListShortcut,
  indentListItems,
  outdentListItems,
  toggleList,
  toggleTaskListAtCursor,
} from './listCommands';
import { goToNextCell, tableEditing, handlePaste as handleTablePaste, isInTable, TableMap } from 'prosemirror-tables';
import { CodeBlockNodeView } from './nodeViews/CodeBlockNodeView';
import { CommentBlockNodeView } from './nodeViews/CommentBlockNodeView';
import { CommentInlineNodeView } from './nodeViews/CommentInlineNodeView';
import { FootnoteDefNodeView } from './nodeViews/FootnoteDefNodeView';
import { FootnoteRefNodeView } from './nodeViews/FootnoteRefNodeView';
import { HtmlBlockNodeView } from './nodeViews/HtmlBlockNodeView';
import { ImageNodeView } from './nodeViews/ImageNodeView';
import { MathBlockNodeView } from './nodeViews/MathBlockNodeView';
import { MathInlineNodeView } from './nodeViews/MathInlineNodeView';
import { MermaidBlockNodeView } from './nodeViews/MermaidBlockNodeView';
import { CalloutNodeView } from './nodeViews/CalloutNodeView';
import { HorizontalRuleNodeView } from './nodeViews/HorizontalRuleNodeView';
import { TocBlockNodeView } from './nodeViews/TocBlockNodeView';
import {
  awaitActiveEditCompositionEnd,
  cancelActiveEdit,
  commitActiveEdit,
  isActiveEditComposing,
} from './nodeViews/activeEditRegistry';
import {
  executeEditorCommand,
  insertParagraphLineBreak,
  splitBlockExitHeading,
} from './editorCommands';
import { findActiveLinkRange } from './editorCommands';
import { blockquoteInputPlugin } from './plugins/blockquoteInput';
import { codeHighlightPlugin } from './plugins/codeHighlight';
import { codeHighlightDecorationPlugin } from './plugins/codeHighlightDecorationPlugin';
import { inlineCodeSelectionBridgePlugin } from './plugins/inlineCodeSelectionBridge';
import { headingLevelIndicatorPlugin } from './plugins/headingLevelIndicator';
import { headingAnchorsKey, headingAnchorsPlugin } from './plugins/headingAnchors';
import { paragraphSelectionGutterPlugin } from './plugins/paragraphSelectionGutter';
import { codeBlockNavigationPlugin } from './plugins/codeBlockNavigation';
import { displayMathInputPlugin } from './plugins/displayMathInput';
import { mathInlineInputPlugin } from './plugins/mathInlineInput';
import { mathInlineDraftPlugin, mathInlineDraftKey, canInsertMathInline, cancelMathInlineDraft } from './plugins/mathInlineDraft';
import { linkInteractionPlugin } from './plugins/linkInteraction';
import {
  inlineSourceEditingPlugin,
  clearEmptyInlineTemplates,
  isInlineSourceComposing,
  getEmptyInlineTemplateFormats,
} from './plugins/inlineSourceEditing';
import { toggleInlineSourceFormat, getActiveInlineSourceFormats, replaceInlineSourceCodeText } from './inlineSourceCommands';
import {
  materializeInlineSource,
  materializeInlineSourceWithMapping,
  readTaskListMarker,
  projectInlineSource,
  projectInlineSourceWithMapping,
} from './InlineSourceCodec';
import { tableControlsPlugin } from './plugins/tableControls';
import { TableNodeView, tableLayoutPlugin, snapshotEditorTableLayout, restoreEditorTableLayout, prepareEditorTableViewport } from './nodeViews/TableNodeView';
import type { TableLayoutSnapshot } from '../table-layout/dom';
import { tableHtmlBlockPlugin } from './plugins/tableHtml';
import { taskListPlugin } from './plugins/taskList';
import { tocSyncPlugin } from './plugins/tocSync';
import { createCalloutPlugin } from './callout/calloutPlugin';
import { removeEmptyCalloutOnBackspace } from './callout/calloutCommands';
import { DetailsSession, detailsPlugin, detailsKey } from './details/detailsState';
import { preserveUnsupportedDetailsHtml } from './details/detailsClipboard';
import { enterDetailsBody, exitDetails, exitDetailsEmptyParagraph, detailsBackspace, detailsDeleteBoundary } from './details/detailsCommands';
import { DetailsNodeView, DetailsSummaryNodeView } from './nodeViews/DetailsNodeView';
import { deleteCodeBlockBeforeCursor } from './codeBlockCommands';
import {
  ensureTrailingParagraph,
  removeEmptyTrailingParagraph,
  trailingParagraphPlugin,
  type TrailingParagraphNormalization,
} from './plugins/trailingParagraph';
import { contextMenuPlugin, type ContextMenuTarget } from './plugins/contextMenu';
import { searchHighlightPlugin } from './plugins/searchHighlight';
import { windowsImePunctuationFallbackPlugin } from './plugins/windowsImePunctuationFallback';
import {
  blockAlignmentPlugin,
  isSemanticBlockAlignmentTransaction,
  setSemanticBlockAlignmentGaps,
  type SemanticBlockAlignmentGap,
} from './plugins/blockAlignment';
import {
  collectMarkdownImageSrcs,
  normalizeImageSrc,
  tryParseMarkdown,
  createMarkdownInputRules,
  getMarkdownBlockLineMap,
  parseMarkdown,
  parseMarkdownWithSyncAnchors,
  serializeMarkdown,
  serializeMarkdownSelection,
  splitFrontMatter,
} from './markdown';
import {
  classifyClipboardMarkdown,
  createMarkdownClipboardSlice,
  createPlainTextSlice,
  plainTextPasteMeta,
  type ClipboardMarkdownClassification,
} from './clipboardMarkdown';
import { schema } from './schema';
import { addTableRowAfter, addTableRowBefore, findTableContext } from './tableCommands';
import { handleTripleClickByDetail } from './tripleClickSelection';
import { updateTocBlocks } from '../toc/tocService';
import type {
  EditorAnchorRect,
  EditorClipboardPayload,
  EditorChangeEvent,
  EditorCommand,
  EditorCore,
  EditorCoreOptions,
  EditorPasteInput,
  EditorPasteMode,
  EditorPasteResult,
  EditorLinkSnapshot,
  EditorListener,
  EditorSearchMatch,
  EditorSearchOptions,
  InlinePendingMarkName,
  InlinePendingMarks,
  EditorRuntimeOptions,
  EditorImageDeletionEvent,
  EditorSelectionEvent,
  EditorSnapshot,
  EditorThemeOptions,
  SetMarkdownOptions,
} from './types';
import { isWholeWordRange } from '../search/textSearch';
import type { EditorSyncCaret, EditorSyncSnapshot, MarkdownSyncAnchor } from './scrollSyncMapping';

const LARGE_DOCUMENT_SEMANTIC_LIMIT = 300_000;
const MARKDOWN_SYNC_DEBOUNCE_MS = 120;

function editorThemesEqual(
  current: EditorThemeOptions | undefined,
  next: EditorThemeOptions,
): boolean {
  if (!current) {
    return false;
  }
  const currentVariables = current.mermaid.themeVariables ?? {};
  const nextVariables = next.mermaid.themeVariables ?? {};
  const currentKeys = Object.keys(currentVariables);
  const nextKeys = Object.keys(nextVariables);
  return (
    current.name === next.name &&
    current.colorThemeId === next.colorThemeId &&
    current.shikiTheme === next.shikiTheme &&
    current.mermaid.theme === next.mermaid.theme &&
    currentKeys.length === nextKeys.length &&
    currentKeys.every((key) => currentVariables[key] === nextVariables[key])
  );
}

export class ProseMirrorEditorCore implements EditorCore {
  private target: HTMLElement | null;
  private view: EditorView | null = null;
  private suspendedState: EditorState | null = null;
  private detailsSession = new DetailsSession();
  private markdown: string;
  private originalMarkdown: string;
  private semanticViewDirty = false;
  private originalDoc: ProseMirrorNode;
  private pendingMarkdownDoc: ProseMirrorNode | null = null;
  private markdownSyncTimer: ReturnType<typeof setTimeout> | null = null;
  private version = 0;
  private dirty = false;
  private destroyed = false;
  private plainTextPasteRequested = false;
  private plainTextPasteResetTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly htmlPasteContexts = new WeakMap<
    ClipboardEvent,
    { text: string; result?: EditorPasteResult }
  >();
  private runtime: EditorRuntimeOptions;
  private listeners = new Set<EditorListener>();
  private blockAlignmentGaps = new Map<string, number>();
  private contentRevision = 0;
  private syncRenderRevision = 0;
  private syncSnapshot: EditorSyncSnapshot | null = null;
  private compositionWaiters = new Set<() => void>();
  private sourceProjection: MarkdownSourceProjection | null = null;
  private initialEditableFrame: number | null = null;
  private sourceOperationDepth = 0;
  private projectionCompositionTimer: ReturnType<typeof setTimeout> | null = null;
  private parsedDocument: { markdown: string; references: unknown; value: TrailingParagraphNormalization;
    source?: ReturnType<typeof parseMarkdownWithSyncAnchors> } | null = null;

  private withProjectionOperation<T>(operation: () => T): T {
    if (!this.options.documentProjection) return operation();
    const outer = this.sourceOperationDepth === 0;
    if (outer) { this.flushPendingMarkdownSync(); this.options.documentProjection.historyBoundary?.('operation'); }
    this.sourceOperationDepth++;
    try { return operation(); }
    finally {
      this.sourceOperationDepth--;
      if (outer) { this.flushPendingMarkdownSync(); this.options.documentProjection.historyBoundary?.('operation'); }
    }
  }

  getSourceSelection(flush = true): DocumentSelection | undefined {
    if (flush) this.flushPendingMarkdownSync();
    const selection = (this.view?.state ?? this.suspendedState)?.selection;
    return this.sourceProjection && selection ? this.projectSelection(this.sourceProjection, selection) : undefined;
  }

  restoreSourceSelection(selection: DocumentSelection): boolean {
    if (!this.sourceProjection) return false;
    const anchor = this.sourceProjection.byteToPosition(selection.anchorByte);
    const head = selection.anchorByte === selection.headByte ? anchor : this.sourceProjection.byteToPosition(selection.headByte);
    if (anchor === null || head === null) return false;
    const state = this.view?.state ?? this.suspendedState;
    if (!state) return false;
    // 原文字节边界可映射到文档/块边界，光标需要落到最近的可编辑文本位置。
    const safeAnchor = Selection.near(state.doc.resolve(anchor), anchor === state.doc.content.size ? -1 : 1).anchor;
    const safeHead = Selection.near(state.doc.resolve(head), head === state.doc.content.size ? -1 : 1).head;
    // 新窗口通常已在同一初始光标；不发空事务，避免接入 DOM 后再次更新插件和浏览器选区。
    // 显式待输入格式仍须由 setSelection 清除，不能把它当成完全相同的状态。
    if (state.selection instanceof TextSelection && state.selection.anchor === safeAnchor &&
      state.selection.head === safeHead && state.storedMarks === null) return true;
    this.restoreSelectionSnapshot({ anchor: safeAnchor, head: safeHead }); return true;
  }

  hasUnsettledProjection(): boolean {
    return !!this.view?.composing || !!this.projectionCompositionTimer || !!this.pendingMarkdownDoc || this.hasPendingMathInlineDraft();
  }
  atSourceWindowBoundary(direction: -1 | 1): boolean {
    const state = this.view?.state ?? this.suspendedState;
    if (!state || !this.sourceProjection) return false;
    return state.selection.head === (direction < 0 ? Selection.atStart(state.doc).head : Selection.atEnd(state.doc).head);
  }

  sourceByteAtCoords(left: number, top: number): number | undefined {
    const hit = this.view?.posAtCoords({ left, top });
    return hit && this.sourceProjection ? this.sourceProjection.positionToByte(hit.pos) ?? undefined : undefined;
  }

  snapshotTableLayout() { return this.view ? snapshotEditorTableLayout(this.view) : undefined; }
  prepareTableViewport(root: Element, margin: number) {
    if (this.view) prepareEditorTableViewport(this.view, root, margin);
  }
  /** 阅读锚点只使用已缓存的块级映射，不走逐字符序列化验证的编辑坐标转换。 */
  captureBlockReadingAnchor(left: number, top: number) {
    if (!this.view) return undefined;
    const hit = this.view.posAtCoords({ left, top });
    if (!hit) return undefined;
    const anchor = this.getScrollSyncSnapshot().anchors.find(anchor => anchor.depth === 0 &&
      anchor.edge === 'start' && anchor.pos <= hit.pos && anchor.endPos > hit.pos);
    if (!anchor) return undefined;
    return { line: anchor.fromLine, offset: hit.pos - anchor.pos, top: this.view.coordsAtPos(hit.pos).top };
  }
  blockReadingAnchorRect(line: number, offset: number) {
    if (!this.view) return undefined;
    const anchor = this.getScrollSyncSnapshot().anchors.find(anchor => anchor.depth === 0 &&
      anchor.edge === 'start' && anchor.fromLine === line);
    if (!anchor) return undefined;
    try { return this.view.coordsAtPos(Math.min(anchor.endPos - 1, anchor.pos + offset)); }
    catch { return undefined; }
  }
  measureStructureBlocks() {
    return this.getScrollSyncSnapshot().anchors.filter(anchor => anchor.depth === 0 && anchor.edge === 'start')
      .flatMap(anchor => { const rect = this.getScrollSyncAnchorRect(anchor);
        return rect ? [{ line: anchor.fromLine, top: rect.top, bottom: rect.bottom }] : []; });
  }
  restoreTableLayout(snapshots: readonly TableLayoutSnapshot[]) {
    return this.view ? restoreEditorTableLayout(this.view, snapshots) : false;
  }

  sourceRangeRects(fromByte: number, toByte: number): DOMRect[] {
    if (!this.view || !this.sourceProjection) return [];
    const from = this.sourceProjection.byteToPosition(fromByte);
    const to = this.sourceProjection.byteToPosition(toByte);
    if (from === null || to === null) return [];
    const start = this.view.domAtPos(from); const end = this.view.domAtPos(to);
    const range = document.createRange(); range.setStart(start.node, start.offset); range.setEnd(end.node, end.offset);
    return Array.from(range.getClientRects());
  }

  rebaseSourceProjection(fromByte: number): void {
    if (!this.options.documentProjection || !this.sourceProjection) return;
    const delta = fromByte - this.options.documentProjection.fromByte;
    this.options.documentProjection.fromByte = fromByte;
    this.sourceProjection.fromByte = fromByte;
    if (this.sourceSelectionBefore) this.sourceSelectionBefore = {
      anchorByte: this.sourceSelectionBefore.anchorByte + delta,
      headByte: this.sourceSelectionBefore.headByte + delta,
    };
  }
  /** 宿主已确认现有节点依赖未变化；仅同步以后输入所需的解析上下文。 */
  updateSourceProjectionReferences(references: NonNullable<EditorCoreOptions['documentProjection']>['references']): void {
    if (this.options.documentProjection) this.options.documentProjection.references = references;
  }
  /** 会话已提交的自动目录只更新 TOC 属性，不能反馈成新输入或替换其他节点。 */
  applyDerivedTocSource(source: string): boolean {
    const view = this.view, projection = this.options.documentProjection;
    if (!view || !projection || !this.sourceProjection || this.hasUnsettledProjection()) return false;
    const before = this.parseSemanticDocument(this.markdown).doc;
    const next = this.parseSemanticDocument(source).doc;
    if (next.childCount !== before.childCount) return false;
    const replacements: ProseMirrorNode[] = [];
    let valid = true;
    before.forEach((node, _pos, i) => {
      const replacement = next.child(i);
      if (node.type.name === 'toc_block' && replacement.type === node.type) replacements.push(replacement);
      else if (!node.eq(replacement)) valid = false;
    });
    if (!valid) return false;
    const transaction = view.state.tr;
    let tocIndex = 0;
    view.state.doc.forEach((node, pos) => {
      if (node.type.name !== 'toc_block') return;
      const replacement = replacements[tocIndex++];
      if (!replacement) { valid = false; return; }
      if (!node.eq(replacement)) transaction.setNodeMarkup(pos, undefined, replacement.attrs, replacement.marks);
    });
    if (!valid || tocIndex !== replacements.length) return false;
    const nextProjection = new MarkdownSourceProjection(source, transaction.doc, projection.fromByte, projection);
    // 不经过 dispatchTransaction：这是已经进入统一历史的派生补丁，不能再提交一次。
    view.updateState(view.state.apply(transaction.setMeta('addToHistory', false).setStoredMarks(view.state.storedMarks)));
    this.sourceProjection = nextProjection;
    this.markdown = source;
    this.dirty = source !== this.originalMarkdown;
    this.semanticViewDirty = false;
    this.contentRevision++; this.syncRenderRevision++; this.version++;
    this.syncSnapshot = null;
    this.emit('content-sync');
    return true;
  }
  private sourceSelectionBefore?: DocumentSelection;

  constructor(private readonly options: EditorCoreOptions) {
    const initialTheme = options.theme ?? {
      name: 'light',
      colorThemeId: 'nomo-default',
      shikiTheme: 'github-light',
      mermaid: { theme: 'default' as const },
    };
    this.options.theme = initialTheme;
    CodeBlockNodeView.updateTheme(initialTheme);
    MermaidBlockNodeView.updateTheme(initialTheme.mermaid);
    this.target = options.target ?? null;
    this.markdown = options.documentProjection ? options.markdown : updateTocBlocks(options.markdown);
    this.originalMarkdown = options.savedMarkdown === undefined ? this.markdown
      : options.documentProjection ? options.savedMarkdown : updateTocBlocks(options.savedMarkdown);
    this.originalDoc = this.parseSemanticDocument(this.originalMarkdown).doc;
    this.dirty = options.dirty ?? this.markdown !== this.originalMarkdown;
    if (options.documentProjection) this.sourceProjection = new MarkdownSourceProjection(this.markdown, this.parseSemanticDocument(this.markdown).doc, options.documentProjection.fromByte, options.documentProjection);
    this.runtime = {
      readonly: options.readonly ?? false,
      mode: options.mode ?? 'semantic',
      copyMarkdownSyntaxEnabled: options.copyMarkdownSyntaxEnabled ?? true,
      quickInsertEnabled: options.quickInsertEnabled ?? true,
      quickInsertTrigger: isQuickInsertTrigger(options.quickInsertTrigger)
        ? options.quickInsertTrigger
        : DEFAULT_QUICK_INSERT_TRIGGER,
    };

    if (this.target) {
      this.mount(this.target);
    }
  }

  mount(target: HTMLElement): void {
    this.assertActive();
    if (this.view && this.target === target) return;
    this.unmount();
    this.target = target;
    if (this.suspendedState) this.suspendedState = this.normalizeLegacyInlineState(this.suspendedState);
    this.view = new EditorView(target, {
      state: this.suspendedState ?? this.createState(this.markdown),
      dispatchTransaction: (transaction) => this.dispatchTransaction(transaction),
      editable: () => this.isEditable(),
      clipboardTextSerializer: (slice) => this.serializeClipboardText(slice),
      clipboardTextParser: (text, $context) => createPlainTextSlice(text, $context),
      transformPastedHTML: preserveUnsupportedDetailsHtml,
      transformPasted: (slice, view) => this.transformInlineSourceSlice(
        slice,
        view.state.selection.$from.parent.type === schema.nodes.html_block &&
          view.state.selection.$from.sameParent(view.state.selection.$to)
          ? projectInlineSource
          : materializeInlineSource,
      ),
      transformCopied: (slice, view) => {
        if (!(view.state.selection instanceof TextSelection)) {
          return this.transformInlineSourceSlice(slice, projectInlineSource);
        }
        const projected = projectInlineSourceWithMapping(view.state.doc);
        const from = projected.mapPosition(view.state.selection.from, 1);
        const to = projected.mapPosition(view.state.selection.to, -1);
        const copied = projected.doc.slice(from, to, true);
        const $from = projected.doc.resolve(from);
        // 只修正复制片段左边界被裁剪的列表；原文和完整包含的嵌套列表保持原始序号。
        const adjustListStart = (content: Fragment, depth: number): Fragment => {
          const first = content.firstChild;
          if (!first || depth > $from.depth) return content;
          let adjusted = first.copy(adjustListStart(first.content, depth + 1));
          if (first.type === schema.nodes.ordered_list && $from.index(depth) > 0) {
            adjusted = first.type.create(
              { ...first.attrs, order: Number(first.attrs.order ?? 1) + $from.index(depth) },
              adjusted.content,
              first.marks,
            );
          }
          return content.replaceChild(0, adjusted);
        };
        return new Slice(adjustListStart(copied.content, 1), copied.openStart, copied.openEnd);
      },
      handlePaste: (_view, event, slice) => this.handleNativePaste(event, slice),
      handleDOMEvents: {
        compositionstart: () => {
          if (this.options.documentProjection) {
            if (this.projectionCompositionTimer) clearTimeout(this.projectionCompositionTimer);
            this.projectionCompositionTimer = null;
            this.flushPendingMarkdownSync(); this.options.documentProjection.historyBoundary?.('composition-start');
          }
          return false;
        },
        compositionend: () => {
          if (this.options.documentProjection) this.projectionCompositionTimer = setTimeout(() => {
            this.projectionCompositionTimer = null;
            if (this.destroyed) return;
            try { this.flushPendingMarkdownSync(); this.options.documentProjection?.historyBoundary?.('composition-end'); }
            catch (cause) { this.options.onError?.({ code: 'source-projection-failed', message: '组字输入仍保留，原文提交需要重试。', cause }); }
          }, 0);
          return false;
        },
        keydown: (_view, event) => {
          if (
            event.shiftKey &&
            (event.ctrlKey || event.metaKey) &&
            event.key.toLowerCase() === 'v'
          ) {
            this.requestPlainTextPaste();
          }
          return false;
        },
        // 视图级属性先于插件运行：三击由此接管后，不再触发插件的按下/拖拽处理。
        mousedown: (view, event) => handleTripleClickByDetail(view, event),
      },
      nodeViews: {
        table: (node, view) => new TableNodeView(node, view),
        code_block: (node, view, getPos) =>
          new CodeBlockNodeView(node, view, getPos as () => number, this.isRestoringNodeSelection(getPos)),
        image: (node, view) =>
          new ImageNodeView(node, view, () => this.options.getImageContext?.() ?? {}),
        html_block: (node, view, getPos) =>
          new HtmlBlockNodeView(node, view, getPos as () => number),
        comment_block: (node, view, getPos) =>
          new CommentBlockNodeView(node, view, getPos as () => number),
        comment_inline: (node, view, getPos) =>
          new CommentInlineNodeView(node, view, getPos as () => number, this.isRestoringNodeSelection(getPos)),
        footnote_ref: (node, view) => new FootnoteRefNodeView(node, view),
        footnote_def: (node, view) => new FootnoteDefNodeView(node, view),
        math_inline: (node, view, getPos) =>
          new MathInlineNodeView(node, view, getPos as () => number, this.isRestoringNodeSelection(getPos)),
        math_block: (node, view, getPos) =>
          new MathBlockNodeView(node, view, getPos as () => number, this.isRestoringNodeSelection(getPos)),
        mermaid_block: (node, view, getPos) =>
          new MermaidBlockNodeView(node, view, getPos as () => number),
        callout: (node, view, getPos) => new CalloutNodeView(node, view, getPos as () => number),
        details: (node, view, getPos) => new DetailsNodeView(node, view, getPos),
        details_summary: (node, view, getPos) => new DetailsSummaryNodeView(node, view, getPos, {
          awaitCompositionEnd: () => this.awaitCompositionEnd(),
          commitPendingEdits: () => this.commitPendingEdits(),
        }),
        horizontal_rule: (node, view, getPos) =>
          new HorizontalRuleNodeView(node, view, getPos as () => number),
        toc_block: (node, view, getPos) => new TocBlockNodeView(node, view, getPos as () => number),
      },
    });
    this.suspendedState = null;
    this.semanticViewDirty = false;
    this.refreshInitialEditableState();
  }

  private isRestoringNodeSelection(getPos: () => number | undefined): boolean {
    const selection = this.suspendedState?.selection;
    return selection instanceof NodeSelection && selection.from === getPos();
  }

  commitPendingEdits(): void {
    this.assertActive();
    if (this.view && !isActiveEditComposing(this.view)) {
      commitActiveEdit(this.view);
      if (this.isEditable()) clearEmptyInlineTemplates(this.view);
    }
    this.flushPendingMarkdownSync();
  }

  hasPendingMathInlineDraft(): boolean {
    return !!this.view && !!mathInlineDraftKey.getState(this.view.state);
  }

  private normalizeLegacyInlineState(state: EditorState): EditorState {
    const converted = materializeInlineSourceWithMapping(state.doc);
    if (converted.doc === state.doc) return state;
    const json = state.selection.toJSON();
    const forward = state.selection.anchor <= state.selection.head;
    if (typeof json.anchor === 'number') json.anchor = converted.mapPosition(json.anchor, forward ? 1 : -1);
    if (typeof json.head === 'number') json.head = converted.mapPosition(json.head, state.selection.empty || !forward ? 1 : -1);
    let selection: Selection;
    try { selection = Selection.fromJSON(converted.doc, json); }
    catch { selection = TextSelection.near(converted.doc.resolve(converted.mapPosition(state.selection.from))); }
    // Old mark-based undo steps have different coordinates. This one-time import
    // starts the current plugins; ordinary source-state remount keeps history.
    return EditorState.create({ doc: converted.doc, selection, plugins: this.createState(this.markdown).plugins });
  }

  awaitCompositionEnd(): Promise<void> {
    const view = this.view;
    if (!view || this.runtime.readonly) return Promise.resolve();
    const nodeComposition = awaitActiveEditCompositionEnd(view);
    // 收到 compositionend 后插件状态可能仍待收尾，不能再等待下一次结束事件。
    const documentComposition = !isInlineSourceComposing(view) || this.projectionCompositionTimer !== null
      ? Promise.resolve()
      : new Promise<void>((resolve) => {
          const finish = () => {
            view.dom.removeEventListener('compositionend', onEnd);
            this.compositionWaiters.delete(finish);
            resolve();
          };
          const onEnd = () => window.setTimeout(finish, 0);
          this.compositionWaiters.add(finish);
          view.dom.addEventListener('compositionend', onEnd, { once: true });
        });
    return Promise.all([documentComposition, nodeComposition]).then(async () => {
      // compositionend 的最终 DOM 输入与投影提交在下一任务收尾；关闭/保存也必须等待它。
      if (this.projectionCompositionTimer !== null)
        await new Promise<void>(resolve => setTimeout(resolve, 0));
    });
  }

  unmount(prepared = false): void {
    this.assertActive();
    if (this.initialEditableFrame !== null) cancelAnimationFrame(this.initialEditableFrame);
    this.initialEditableFrame = null;
    if (this.view) cancelMathInlineDraft(this.view);
    if (!prepared) this.commitPendingEdits();
    this.clearPlainTextPasteRequest();
    if (!this.view) return;
    this.clearBlockAlignmentGaps();
    this.suspendedState = this.semanticViewDirty ? null : this.view.state;
    this.view.destroy();
    for (const finish of [...this.compositionWaiters]) finish();
    this.view = null;
    this.target = null;
    this.syncSnapshot = null;
  }

  destroy(options?: { prepared?: boolean }): void {
    if (this.projectionCompositionTimer !== null) clearTimeout(this.projectionCompositionTimer);
    this.projectionCompositionTimer = null;
    this.parsedDocument = null;
    if (this.destroyed) return;
    this.unmount(options?.prepared);
    this.clearMarkdownSyncTimer();
    this.clearPlainTextPasteRequest();
    this.listeners.clear();
    this.view?.destroy();
    this.view = null;
    this.target = null;
    this.suspendedState = null;
    this.destroyed = true;
    this.detailsSession.clear();
    this.parsedDocument = null;
  }

  getMarkdown(): string {
    this.assertActive();
    this.flushPendingMarkdownSync();
    return this.markdown;
  }

  revealMarkdownLine(lineNumber: number): boolean {
    if (!this.view) return false;
    const mappings = getMarkdownBlockLineMap(this.markdown);
    if (mappings.length === 0) return false;

    const containing = mappings.find(
      (mapping) => lineNumber >= mapping.fromLine && lineNumber <= mapping.toLine,
    );
    const target =
      containing ??
      mappings.reduce((nearest, mapping) => {
        const nearestDistance = Math.min(
          Math.abs(lineNumber - nearest.fromLine),
          Math.abs(lineNumber - nearest.toLine),
        );
        const distance = Math.min(
          Math.abs(lineNumber - mapping.fromLine),
          Math.abs(lineNumber - mapping.toLine),
        );
        return distance < nearestDistance ? mapping : nearest;
      });
    if (target.nodeIndex >= this.view.state.doc.childCount) return false;

    let blockPosition = 0;
    for (let index = 0; index < target.nodeIndex; index += 1) {
      blockPosition += this.view.state.doc.child(index).nodeSize;
    }
    const selectionPosition = Math.min(blockPosition + 1, this.view.state.doc.content.size);
    this.view.dispatch(
      this.view.state.tr
        .setSelection(TextSelection.near(this.view.state.doc.resolve(selectionPosition)))
        .scrollIntoView(),
    );
    const blockDom = this.view.nodeDOM(blockPosition);
    if (blockDom instanceof HTMLElement) {
      blockDom.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
    return true;
  }

  getBlockAlignmentBlockCount(): number {
    return this.view?.state.doc.childCount ?? 0;
  }

  getExportHtml(): string {
    this.flushPendingMarkdownSync();
    const doc = this.semanticViewDirty
      ? parseMarkdown(this.markdown, this.options.documentProjection)
      : this.view?.state.doc ?? this.suspendedState?.doc;
    if (!doc) return '';
    const nodes = DOMSerializer.nodesFromSchema(schema);
    const snapshots = new Map<ProseMirrorNode, HTMLElement>();
    const renderedTypes = new Set([
      'image', 'math_inline', 'math_block', 'mermaid_block', 'code_block', 'toc_block',
    ]);
    doc.descendants((node, pos) => {
      if (!renderedTypes.has(node.type.name)) return;
      if (this.view?.state.doc !== doc) return;
      const dom = this.view?.nodeDOM(pos);
      if (dom instanceof HTMLElement) snapshots.set(node, dom);
    });
    for (const name of renderedTypes) {
      const fallback = nodes[name];
      nodes[name] = (node) => {
        const snapshot = snapshots.get(node);
        if (name === 'image' && snapshot) return ImageNodeView.getExportSnapshot(snapshot) ?? fallback(node);
        return (snapshot?.cloneNode(true) as HTMLElement | undefined) ?? fallback(node);
      };
    }
    for (const name of ['math_inline', 'math_block']) {
      nodes[name] = (node) => {
        const block = name === 'math_block';
        const element = document.createElement(block ? 'div' : 'span');
        element.className = block ? 'math-block' : 'math-inline';
        element.dataset.tex = String(node.attrs.tex ?? '');
        element.innerHTML = katex.renderToString(element.dataset.tex, {
          displayMode: block, throwOnError: false, trust: false,
        });
        return element;
      };
    }
    const serializer = new DOMSerializer(nodes, DOMSerializer.marksFromSchema(schema));
    const container = document.createElement('div');
    container.append(serializer.serializeFragment(projectInlineSource(removeEmptyTrailingParagraph(doc)).content));
    return container.innerHTML;
  }

  getScrollSyncSnapshot(): EditorSyncSnapshot {
    if (!this.view || this.semanticViewDirty || this.pendingMarkdownDoc) {
      return { revision: this.contentRevision, renderRevision: this.syncRenderRevision,
        ready: false, markdown: this.markdown, anchors: [] };
    }
    if (this.syncSnapshot?.revision === this.contentRevision) {
      if (this.syncSnapshot.renderRevision !== this.syncRenderRevision)
        this.syncSnapshot = { ...this.syncSnapshot, renderRevision: this.syncRenderRevision };
      return this.syncSnapshot;
    }
    this.parseSemanticDocument(this.markdown, true);
    const parsed = this.parsedDocument!.source!;
    const currentDoc = this.view.state.doc;
    // 复杂块的序列化可能多出软换行。用结构差异的已确认前后缀换算偏移，
    // 不能因局部节点长度不同而丢弃后续整篇锚点，也不按块数或文字搜索猜位置。
    const diffStart = parsed.doc.content.findDiffStart(currentDoc.content);
    const diffEnd = diffStart == null ? null : parsed.doc.content.findDiffEnd(currentDoc.content);
    const anchors = parsed.anchors.flatMap((anchor) => {
      if (diffEnd && anchor.pos >= (diffStart ?? 0) && anchor.pos < diffEnd.a) return [];
      const shift =
        diffEnd && anchor.pos >= Math.max(diffStart ?? 0, diffEnd.a)
          ? diffEnd.b - diffEnd.a
          : 0;
      const pos = anchor.pos + shift;
      if (pos < 0 || pos >= currentDoc.content.size) return [];
      const expected = parsed.doc.nodeAt(anchor.pos);
      const actual = currentDoc.nodeAt(pos);
      return expected != null && actual != null && expected.eq(actual)
        ? [{ ...anchor, pos, endPos: anchor.endPos + shift }]
        : [];
    });
    const lines = this.markdown.split('\n');
    const topLevel = anchors.filter((anchor) => anchor.depth === 0 && anchor.edge === 'start');
    for (let index = 1; index < topLevel.length; index += 1) {
      const previous = topLevel[index - 1];
      const next = topLevel[index];
      if (next.fromLine > previous.toLine + 1 &&
          lines.slice(previous.toLine, next.fromLine - 1).every((line) => !line.trim())) {
        anchors.push({ ...previous, key: `${previous.pos}:blank`, kind: 'blank', edge: 'end',
          fromLine: previous.toLine + 1, toLine: next.fromLine - 1 });
      }
    }
    const eofLine = lines.length + 1;
    if (currentDoc.childCount > 0) {
      const lastPos = currentDoc.content.size - currentDoc.lastChild!.nodeSize;
      anchors.push({ key: 'eof', fromLine: eofLine, toLine: eofLine, pos: lastPos,
        endPos: currentDoc.content.size, kind: 'eof', edge: 'end', depth: 0 });
    }
    this.syncSnapshot = {
      revision: this.contentRevision,
      renderRevision: this.syncRenderRevision,
      ready: anchors.some((anchor) => anchor.kind !== 'eof') || Boolean(currentDoc.attrs.frontMatterPrefix),
      markdown: this.markdown, anchors,
    };
    return this.syncSnapshot;
  }

  /** 返回浏览器视口坐标；调用方负责换算到所属滚动容器，不改文档或选区。 */
  getScrollSyncAnchorRect(anchor: MarkdownSyncAnchor): { top: number; bottom: number } | null {
    if (!this.view || anchor.pos < 0 || anchor.pos >= this.view.state.doc.content.size) return null;
    const element = this.view.nodeDOM(anchor.pos);
    if (!(element instanceof HTMLElement) || element.getClientRects().length === 0) return null;
    const rect = element.getBoundingClientRect();
    if (rect.height <= 0) return null;
    if (anchor.edge === 'line') {
      const content = element.querySelector<HTMLElement>('.code-content');
      const code = content?.querySelector<HTMLElement>('code');
      if (!content || !code || element.classList.contains('is-editing') ||
          content.scrollHeight > content.clientHeight + 1 || content.scrollTop > 0) return null;
      const style = getComputedStyle(code);
      const scale = code.offsetHeight > 0 ? code.getBoundingClientRect().height / code.offsetHeight : 1;
      const lineHeight = Number.parseFloat(style.lineHeight);
      if (!Number.isFinite(lineHeight)) return null;
      const top = code.getBoundingClientRect().top +
        ((Number.parseFloat(style.paddingTop) || 0) + (anchor.lineOffset ?? 0) * lineHeight) * scale;
      return { top, bottom: top + lineHeight * scale };
    }
    return rect;
  }

  getScrollSyncCaret(): EditorSyncCaret | null {
    if (!this.view) return null;
    const active = this.view.dom.ownerDocument.activeElement;
    if (active instanceof HTMLTextAreaElement && this.view.dom.contains(active) &&
        active.classList.contains('code-input')) return null;
    if (active instanceof HTMLTextAreaElement && this.view.dom.contains(active)) {
      try {
        return { head: this.view.posAtDOM(active, 0), blockOnly: true };
      } catch {
        return { head: this.view.state.selection.head, blockOnly: true };
      }
    }
    const head = this.view.state.selection.head;
    try {
      return { head, viewportTop: this.view.coordsAtPos(head).top };
    } catch {
      return { head, blockOnly: true };
    }
  }

  getBlockAlignmentGeometry(anchors: Array<{ key: string; nodeIndex: number }>) {
    if (!this.view) return [];
    const view = this.view;
    return anchors
      .filter((anchor) => anchor.nodeIndex >= 0 && anchor.nodeIndex < view.state.doc.childCount)
      .map((anchor, index, validAnchors) => {
        const top = this.getBlockTop(anchor.nodeIndex);
        const nextAnchor = validAnchors[index + 1];
        const nextTop = nextAnchor
          ? this.getBlockTop(nextAnchor.nodeIndex)
          : this.getBlockBottom(anchor.nodeIndex, anchor.key);
        return {
          key: anchor.key,
          top,
          nextTop: Math.max(top, nextTop),
          existingGap: this.blockAlignmentGaps.get(anchor.key) ?? 0,
        };
      });
  }

  applyBlockAlignmentGaps(gaps: SemanticBlockAlignmentGap[]): void {
    if (!this.view) return;
    const view = this.view;
    const scale = this.getBlockAlignmentScale();
    const effectivePhysicalGaps = gaps.filter(
      (gap) =>
        gap.height > 0.05 && gap.nodeIndex >= 0 && gap.nodeIndex < view.state.doc.childCount - 1,
    );
    const effectiveGaps = effectivePhysicalGaps.map((gap) => ({
      ...gap,
      height: gap.height / scale,
      marginCompensation: this.getBlockMarginCompensation(gap.nodeIndex),
    }));
    this.blockAlignmentGaps = new Map(effectivePhysicalGaps.map((gap) => [gap.key, gap.height]));
    view.dispatch(setSemanticBlockAlignmentGaps(view.state.tr, effectiveGaps));
  }

  clearBlockAlignmentGaps(): void {
    if (!this.view || this.blockAlignmentGaps.size === 0) return;
    this.blockAlignmentGaps.clear();
    this.view.dispatch(setSemanticBlockAlignmentGaps(this.view.state.tr, []));
  }

  private getBlockTop(nodeIndex: number) {
    if (!this.view) return 0;
    const dom = this.view.nodeDOM(this.getBlockPosition(nodeIndex));
    return dom instanceof HTMLElement ? dom.getBoundingClientRect().top : 0;
  }

  private getBlockBottom(nodeIndex: number, key: string) {
    if (!this.view) return 0;
    const dom = this.view.nodeDOM(this.getBlockPosition(nodeIndex));
    if (!(dom instanceof HTMLElement)) return this.getBlockTop(nodeIndex);
    return dom.getBoundingClientRect().bottom + (this.blockAlignmentGaps.get(key) ?? 0);
  }

  private getBlockMarginCompensation(nodeIndex: number) {
    if (!this.view || nodeIndex < 0 || nodeIndex >= this.view.state.doc.childCount - 1) return 0;
    const currentDom = this.view.nodeDOM(this.getBlockPosition(nodeIndex));
    const nextDom = this.view.nodeDOM(this.getBlockPosition(nodeIndex + 1));
    if (!(currentDom instanceof HTMLElement) || !(nextDom instanceof HTMLElement)) return 0;

    const currentMarginBottom = Math.max(
      0,
      Number.parseFloat(getComputedStyle(currentDom).marginBottom) || 0,
    );
    const nextMarginTop = Math.max(0, Number.parseFloat(getComputedStyle(nextDom).marginTop) || 0);
    if (currentMarginBottom === 0 || nextMarginTop === 0) return 0;

    return Math.min(currentMarginBottom, nextMarginTop);
  }

  private getBlockAlignmentScale() {
    if (!this.view) return 1;
    const host = this.view.dom.closest<HTMLElement>('.prosemirror-host') ?? this.view.dom;
    const zoom = Number.parseFloat(getComputedStyle(host).zoom);
    if (Number.isFinite(zoom) && zoom > 0) return zoom;
    const rect = host.getBoundingClientRect();
    return host.offsetWidth > 0 && rect.width > 0 ? rect.width / host.offsetWidth : 1;
  }

  private getBlockPosition(nodeIndex: number) {
    if (!this.view) return 0;
    let position = 0;
    for (let index = 0; index < nodeIndex; index += 1) {
      position += this.view.state.doc.child(index).nodeSize;
    }
    return position;
  }

  flushMarkdown(): string {
    this.assertActive();
    this.commitPendingEdits();
    return this.markdown;
  }

  refreshSemanticView(): void {
    this.assertActive();
    this.flushPendingMarkdownSync();
    if (!this.semanticViewDirty) {
      return;
    }
    this.replaceViewState(this.markdown, undefined, true);
  }

  setDirty(dirty: boolean): void {
    this.assertActive();
    this.flushPendingMarkdownSync();
    this.dirty = dirty;
    if (!dirty) {
      this.originalMarkdown = this.markdown;
      this.originalDoc = this.view?.state.doc ?? this.parseSemanticDocument(this.markdown).doc;
    }
  }

  setMarkdown(markdown: string, options?: SetMarkdownOptions): void {
    this.assertActive();
    this.flushPendingMarkdownSync();
    const previousMarkdown = this.markdown;
    if (markdown !== previousMarkdown) this.detailsSession.beforeSourceChange(previousMarkdown, markdown);
    const delaySemanticSync = options?.sourceInput === true;
    this.markdown = this.options.documentProjection ? markdown : updateTocBlocks(markdown);
    this.contentRevision += 1;
    this.syncSnapshot = null;
    this.version += 1;

    const savedMarkdown =
      options?.savedMarkdown === undefined ? undefined : this.options.documentProjection ? options.savedMarkdown : updateTocBlocks(options.savedMarkdown);
    if (savedMarkdown !== undefined) {
      this.originalMarkdown = savedMarkdown;
      this.originalDoc = this.parseSemanticDocument(savedMarkdown).doc;
    } else if (options?.reason === 'open-file' || options?.reason === 'save-file') {
      this.originalMarkdown = this.markdown;
      this.originalDoc = this.parseSemanticDocument(this.markdown).doc;
    } else if (options?.reason === 'switch-tab' && options?.dirty !== true) {
      this.originalMarkdown = this.markdown;
      this.originalDoc = this.parseSemanticDocument(this.markdown).doc;
    }

    this.dirty = options?.dirty ?? this.markdown !== this.originalMarkdown;
    if (this.options.documentProjection) this.sourceProjection = new MarkdownSourceProjection(this.markdown, this.parseSemanticDocument(this.markdown).doc, this.options.documentProjection.fromByte, this.options.documentProjection);
    if (delaySemanticSync) {
      this.semanticViewDirty = true;
      this.suspendedState = null;
      if (this.options.onImagesDeleted && shouldReportImageDeletion(options)) {
        this.notifyDeletedImageSrcs(
          findFullyRemovedMarkdownImageSrcs(previousMarkdown, this.markdown),
        );
      }
      this.emit(options?.reason ?? 'programmatic-update');
      return;
    }

    this.replaceViewState(this.markdown);
    if (this.options.onImagesDeleted && shouldReportImageDeletion(options)) {
      this.notifyDeletedImageSrcs(
        findFullyRemovedMarkdownImageSrcs(previousMarkdown, this.markdown),
      );
    }
    this.emit(options?.reason ?? 'programmatic-update');
  }

  getSnapshot(): EditorSnapshot {
    this.assertActive();
    this.commitPendingEdits();
    const selection = (this.view?.state ?? this.suspendedState)?.selection;
    return {
      markdown: this.markdown,
      version: this.version,
      selection: selection ? { anchor: selection.anchor, head: selection.head } : undefined,
      contentRevision: this.contentRevision,
      meta: {
        mode: this.runtime.mode,
        inlineSource: true,
      },
    };
  }

  restoreSnapshot(snapshot: EditorSnapshot): void {
    this.assertActive();
    this.clearPendingMarkdownSync();
    // 快照不携带视图状态；不同正文不能按旧文档的数值位置继承临时展开。
    if (snapshot.markdown !== this.markdown) this.detailsSession.clear();
    this.markdown = this.options.documentProjection ? snapshot.markdown : updateTocBlocks(snapshot.markdown);
    this.originalMarkdown = this.markdown;
    this.originalDoc = this.parseSemanticDocument(this.markdown).doc;
    if (this.options.documentProjection) this.sourceProjection = new MarkdownSourceProjection(this.markdown, this.originalDoc, this.options.documentProjection.fromByte, this.options.documentProjection);
    this.version = snapshot.version;
    this.dirty = true;
    const selection = snapshot.selection && snapshot.meta?.inlineSource !== true
      ? this.restoreLegacyInlineSelection(this.originalDoc, snapshot.selection)
      : snapshot.selection;
    this.replaceViewState(this.markdown, selection);
    this.emit('restore-snapshot');
  }

  restoreSelectionSnapshot(selection: { anchor: number; head: number }): void {
    this.assertActive();
    const state = this.view?.state ?? this.suspendedState ?? this.createState(this.markdown);
    const transaction = state.tr.setSelection(this.createRestoredSelection(state, selection));
    if (this.view) this.view.dispatch(transaction);
    else this.suspendedState = state.apply(transaction);
  }

  placeCaretAtCoordinates({ left, top }: { left: number; top: number }): boolean {
    const view = this.view;
    if (!view || !view.dom.isConnected || !this.isEditable() || view.composing ||
      isInlineSourceComposing(view) || isActiveEditComposing(view) ||
      !Number.isFinite(left) || !Number.isFinite(top)) return false;

    // 节点输入的提交和 blur 都可能改变文档或恢复旧光标，必须先完成再定位。
    commitActiveEdit(view, true);
    view.focus();
    const bounds = view.dom.getBoundingClientRect();
    // 编辑面有 min-height 和文末滚动留白，边界应取实际内容而非宿主高度。
    const contentTop = view.dom.firstElementChild?.getBoundingClientRect().top ?? bounds.top;
    const contentBottom = view.dom.lastElementChild?.getBoundingClientRect().bottom ?? bounds.bottom;
    const doc = view.state.doc;
    const bias = top >= contentBottom || left >= bounds.right ? -1 : 1;
    let position = top < contentTop ? 0 : top >= contentBottom ? doc.content.size : null;
    if (position === null) {
      position = view.posAtCoords({
        left: Math.max(bounds.left + 1, Math.min(bounds.right - 1, left)),
        top,
      })?.pos ?? null;
    }
    if (position === null) {
      // 块间空白或不可编辑对象可能无法命中；只二分查找邻近块，不扫描全文。
      const children = view.dom.children;
      let lo = 0, hi = children.length;
      while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (children[mid].getBoundingClientRect().bottom < top) lo = mid + 1;
        else hi = mid;
      }
      const block = children[Math.min(lo, children.length - 1)];
      position = block ? view.posAtDOM(block, bias < 0 ? block.childNodes.length : 0, bias) : 0;
    }
    const resolved = doc.resolve(Math.max(0, Math.min(doc.content.size, position)));
    const selection = Selection.findFrom(resolved, bias, true) ?? Selection.findFrom(resolved, -bias, true);
    if (!selection) return false;
    view.dispatch(view.state.tr.setSelection(selection).setMeta('addToHistory', false));
    return true;
  }

  focus(): void {
    this.view?.focus();
  }

  blur(): void {
    if (this.view?.dom instanceof HTMLElement) {
      this.view.dom.blur();
    }
  }

  getActiveLink(): EditorLinkSnapshot | null {
    this.assertActive();
    if (!this.view) return null;
    return findActiveLinkRange(this.view.state);
  }

  getSelectionAnchorRect(): EditorAnchorRect | null {
    this.assertActive();
    if (!this.view) return null;

    const { selection } = this.view.state;
    const from = selection.from;
    const to = selection.empty ? selection.from : selection.to;

    try {
      const fromRect = this.view.coordsAtPos(from);
      const toRect = this.view.coordsAtPos(to);
      const left = Math.min(fromRect.left, toRect.left);
      const top = Math.min(fromRect.top, toRect.top);
      const right = Math.max(fromRect.right, toRect.right);
      const bottom = Math.max(fromRect.bottom, toRect.bottom);

      return {
        x: left,
        y: top,
        left,
        top,
        right,
        bottom,
        width: Math.max(1, right - left),
        height: Math.max(1, bottom - top),
        toJSON: () => ({ x: left, y: top, left, top, right, bottom }),
      };
    } catch {
      const rect = this.view.dom.getBoundingClientRect();
      return {
        x: rect.x,
        y: rect.y,
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        width: rect.width,
        height: rect.height,
        toJSON: () => rect.toJSON(),
      };
    }
  }

  getClipboardPayload(): EditorClipboardPayload | null {
    this.assertActive();
    if (!this.view || this.view.state.selection.empty) return null;
    const serialized = this.view.serializeForClipboard(this.view.state.selection.content());
    return {
      text: serialized.text,
      html: serialized.dom.innerHTML,
    };
  }

  private serializeClipboardText(slice: Slice): string {
    const plainText = serializeClipboardText(slice);
    if (!this.runtime.copyMarkdownSyntaxEnabled || !this.view) {
      return plainText;
    }

    try {
      const { doc, selection } = this.view.state;
      const clipboardDoc = removeEmptyTrailingParagraph(doc);
      const selectionTo = Math.min(selection.to, clipboardDoc.content.size);
      return serializeMarkdownSelection(clipboardDoc, selection.from, selectionTo) ??
        serializeClipboardText(selection.content(), true);
    } catch {
      return plainText;
    }
  }

  /** 旧快照按去掉六类定界符后的 PM 坐标存储；只在未标识的新旧边界换算一次。 */
  private restoreLegacyInlineSelection(doc: ProseMirrorNode, selection: { anchor: number; head: number }) {
    const projected = projectInlineSourceWithMapping(doc);
    const restore = (position: number, bias: number) => {
      const target = Math.max(0, Math.min(projected.doc.content.size, position));
      let low = 0;
      let high = doc.content.size;
      while (low < high) {
        const mid = Math.floor((low + high) / 2);
        const mapped = projected.mapPosition(mid);
        if (mapped < target || (bias > 0 && mapped === target)) low = mid + 1;
        else high = mid;
      }
      return bias > 0 && projected.mapPosition(low) > target ? Math.max(0, low - 1) : low;
    };
    const forward = selection.anchor <= selection.head;
    return {
      anchor: restore(selection.anchor, forward ? 1 : -1),
      head: restore(selection.head, selection.anchor === selection.head || !forward ? 1 : -1),
    };
  }

  /** 导入/复制只转换片段，保持原有 Slice 开放深度和复杂节点。 */
  private transformInlineSourceSlice(slice: Slice, transform: (doc: ProseMirrorNode) => ProseMirrorNode): Slice {
    const inline = slice.content.firstChild?.isInline === true;
    const content = inline ? Fragment.from(schema.nodes.paragraph.create(null, slice.content)) : slice.content;
    const converted = transform(schema.nodes.doc.create(null, content));
    return new Slice(inline ? converted.firstChild!.content : converted.content, slice.openStart, slice.openEnd);
  }

  pasteClipboardText(text: string): boolean {
    return this.pasteClipboard({ text }).status === 'inserted';
  }

  pasteClipboardHtml(html: string): boolean {
    return this.pasteClipboard({ html }).status === 'inserted';
  }

  pasteClipboard(
    input: EditorPasteInput,
    options: { mode?: EditorPasteMode } = {},
  ): EditorPasteResult {
    return this.withProjectionOperation(() => this.pasteClipboardContent(input, options));
  }

  private pasteClipboardContent(input: EditorPasteInput, options: { mode?: EditorPasteMode }): EditorPasteResult {
    this.assertActive();
    if (!this.view || !this.isEditable()) {
      return { status: 'rejected', reason: 'readonly' };
    }

    const text = input.text ?? '';
    if (text && replaceInlineSourceCodeText(this.view.state, text)) {
      return this.insertPlainClipboardText(text);
    }
    if (options.mode === 'plain') {
      return text ? this.insertPlainClipboardText(text) : { status: 'rejected', reason: 'no-text' };
    }

    if (input.html && isInTable(this.view.state)) {
      const template = document.createElement('template');
      template.innerHTML = input.html;
      const parser = this.view.someProp('clipboardParser') ?? this.view.someProp('domParser') ?? DOMParser.fromSchema(schema);
      const slice = parser.parseSlice(template.content, { preserveWhitespace: true, context: this.view.state.selection.$from });
      if (this.isTableClipboardSlice(slice)) return this.insertHtmlClipboard(input.html, text);
    }

    const listShortcut = text ? this.insertListClipboardShortcut(text) : null;
    if (listShortcut) return listShortcut;

    const classification = text ? classifyClipboardMarkdown(text) : null;
    if (classification?.kind === 'front-matter') {
      if (this.isEmptySemanticDocument()) {
        this.insertWholeMarkdownDocument(classification.doc);
        return { status: 'inserted', format: 'markdown' };
      }
      return this.insertPlainClipboardText(text);
    }

    if (classification?.kind === 'markdown') {
      if (this.insertMarkdownClipboard(classification)) {
        return { status: 'inserted', format: 'markdown' };
      }
      return this.insertPlainClipboardText(text);
    }

    if (input.html) return this.insertHtmlClipboard(input.html, text);

    return text ? this.insertPlainClipboardText(text) : { status: 'rejected', reason: 'no-text' };
  }

  private insertHtmlClipboard(html: string, text: string): EditorPasteResult {
    const event = typeof ClipboardEvent === 'undefined' ? new Event('paste') as ClipboardEvent : new ClipboardEvent('paste');
    const context: { text: string; result?: EditorPasteResult } = { text };
    this.htmlPasteContexts.set(event, context);
    const inserted = this.view!.pasteHTML(html, event);
    this.htmlPasteContexts.delete(event);
    return context.result ?? (inserted ? { status: 'inserted', format: 'html' } : { status: 'rejected', reason: 'no-text' });
  }

  /** 只允许 schema 合法的 table/row/cell 片段进入网格路径；其它块仍受 B057 guard 约束。 */
  private isTableClipboardSlice(slice: Slice): boolean {
    let content = slice.content;
    const table = content.childCount === 1 && content.firstChild?.type === schema.nodes.table ? content.firstChild : null;
    if (table) content = table.content;
    if (!content.childCount) return false;
    const rows = content.firstChild!.type === schema.nodes.table_row;
    try {
      for (let index = 0; index < content.childCount; index += 1) {
        const node = content.child(index);
        if (rows ? node.type !== schema.nodes.table_row : node.type !== schema.nodes.table_cell && node.type !== schema.nodes.table_header) return false;
        node.check();
        const cells = rows ? node.content : Fragment.from(node);
        for (let column = 0; column < cells.childCount; column += 1) {
          const cell = cells.child(column);
          if (!Number.isSafeInteger(cell.attrs.colspan) || cell.attrs.colspan < 1 ||
              !Number.isSafeInteger(cell.attrs.rowspan) || cell.attrs.rowspan < 1) return false;
        }
      }
      return !table || !TableMap.get(table).problems;
    } catch {
      return false;
    }
  }

  private handleNativePaste(event: ClipboardEvent, slice: Slice): boolean {
    if (!this.view || !this.isEditable()) {
      const context = this.htmlPasteContexts.get(event);
      if (context) context.result = { status: 'rejected', reason: 'readonly' };
      event.preventDefault();
      return true;
    }
    const clipboard = event.clipboardData;
    const plain = clipboard ? this.consumePlainTextPasteRequest() : false;
    if (!plain && clipboard && clipboard.files.length > 0) {
      return false;
    }

    const text = clipboard?.getData('text/plain') ?? '';
    const html = clipboard?.getData('text/html') ?? '';
    const codeText = text || this.htmlPasteContexts.get(event)?.text ||
      serializeClipboardText(this.transformInlineSourceSlice(slice, projectInlineSource));
    if (this.view.state.selection.$from.parent.type.name === 'details_summary' &&
      (plain || !this.canInsertClipboardContent(slice.content))) {
      const result = codeText ? this.insertPlainClipboardText(codeText) : { status: 'rejected', reason: 'no-text' } as EditorPasteResult;
      const context = this.htmlPasteContexts.get(event);
      if (context) context.result = result;
      event.preventDefault();
      return true;
    }
    if (codeText && replaceInlineSourceCodeText(this.view.state, codeText)) {
      const result = this.insertPlainClipboardText(codeText);
      const context = this.htmlPasteContexts.get(event);
      if (context) context.result = result;
      event.preventDefault();
      return true;
    }
    if (!plain && isInTable(this.view.state) && this.isTableClipboardSlice(slice) &&
        handleTablePaste(this.view, event, slice)) {
      const context = this.htmlPasteContexts.get(event);
      if (context) context.result = { status: 'inserted', format: 'html' };
      event.preventDefault();
      return true;
    }
    if (!plain && text && this.insertListClipboardShortcut(text)) {
      event.preventDefault();
      return true;
    }
    if (!plain && !this.canInsertClipboardContent(slice.content)) {
      const context = this.htmlPasteContexts.get(event);
      const fallbackText = text || context?.text || serializeClipboardText(slice);
      const result: EditorPasteResult = !this.isEditable()
        ? { status: 'rejected', reason: 'readonly' }
        : fallbackText
          ? this.insertPlainClipboardText(fallbackText)
          : { status: 'rejected', reason: 'no-text' };
      if (context) context.result = result;
      // 空 HR 没有可回退文字，必须吞掉粘贴事件但保留原选区内容。
      event.preventDefault();
      return true;
    }
    if (!clipboard) return false;

    if (!plain) {
      if (!text || classifyClipboardMarkdown(text).kind === 'plain') {
        return false;
      }
    }

    const result = this.pasteClipboard({ text, html }, { mode: plain ? 'plain' : 'auto' });
    event.preventDefault();
    return result.status === 'inserted' || result.reason === 'no-text';
  }

  private insertListClipboardShortcut(text: string): EditorPasteResult | null {
    if (!this.view) return null;
    const shortcut = createPastedListShortcut(this.view.state, text);
    if (!shortcut) return null;
    this.view.dispatch(shortcut.transaction.setMeta('uiEvent', 'paste'));
    return { status: 'inserted', format: shortcut.converted ? 'markdown' : 'plain' };
  }

  private insertPlainClipboardText(text: string): EditorPasteResult {
    if (!this.view) return { status: 'rejected', reason: 'readonly' };
    const { state } = this.view;
    if (state.selection.$from.parent.type.name === 'details_summary' && state.selection.$from.sameParent(state.selection.$to)) {
      const normalized = text.replace(/[\r\n\t]+/g, ' ');
      const tr = replaceInlineSourceCodeText(state, normalized) ?? state.tr.insertText(normalized);
      this.view.dispatch(tr.setMeta(plainTextPasteMeta, true).scrollIntoView());
      return { status: 'inserted', format: 'plain' };
    }
    const codeTransaction = replaceInlineSourceCodeText(state, text);
    const transaction = (codeTransaction ?? state.tr.replaceSelection(createPlainTextSlice(text, state.selection.$from)))
      .setMeta(plainTextPasteMeta, true)
      .scrollIntoView();
    this.view.dispatch(transaction);
    return { status: 'inserted', format: 'plain' };
  }

  private insertMarkdownClipboard(classification: ClipboardMarkdownClassification): boolean {
    if (!this.view || classification.kind !== 'markdown') return false;
    const { state } = this.view;
    if (!this.canInsertClipboardContent(classification.doc.content)) return false;

    try {
      const clipboardDoc = state.selection.$from.parent.type === schema.nodes.html_block &&
        state.selection.$from.sameParent(state.selection.$to)
        ? projectInlineSource(classification.doc)
        : classification.doc;
      const transaction = state.tr
        .replaceSelection(createMarkdownClipboardSlice(clipboardDoc))
        .scrollIntoView();
      if (!transaction.docChanged) return false;
      this.view.dispatch(transaction);
      return true;
    } catch {
      return false;
    }
  }

  private canInsertClipboardContent(content: Fragment): boolean {
    if (!this.view) return false;
    const { selection } = this.view.state;
    if (selection.$from.parent.type.name === 'details_summary' || selection.$to.parent.type.name === 'details_summary') {
      let allowed = true;
      content.forEach(node => { if (!node.isText && (node.type.name !== 'paragraph' || Array.from({ length: node.childCount }, (_, i) => node.child(i)).some(child => !child.isText))) allowed = false; });
      return allowed && content.childCount <= 1;
    }
    if (
      selection.$from.parent.type === schema.nodes.code_block ||
      selection.$to.parent.type === schema.nodes.code_block
    ) {
      return false;
    }

    const insideTableCell =
      isSelectionInsideNodeType(selection.$from, 'table_cell', 'table_header') ||
      isSelectionInsideNodeType(selection.$to, 'table_cell', 'table_header');
    if (insideTableCell) {
      for (let index = 0; index < content.childCount; index += 1) {
        if (content.child(index).type !== schema.nodes.paragraph) return false;
      }
    }
    return true;
  }

  private insertWholeMarkdownDocument(doc: ProseMirrorNode): void {
    if (!this.view) return;
    const frontMatterPrefix = String(doc.attrs.frontMatterPrefix ?? '');
    let transaction = this.view.state.tr
      .replaceWith(0, this.view.state.doc.content.size, doc.content)
      .setDocAttribute('frontMatterPrefix', frontMatterPrefix);
    transaction = transaction.setSelection(TextSelection.atEnd(transaction.doc)).scrollIntoView();
    this.view.dispatch(transaction);
  }

  private isEmptySemanticDocument(): boolean {
    if (!this.view) return false;
    const { doc } = this.view.state;
    return (
      !String(doc.attrs.frontMatterPrefix ?? '') &&
      doc.childCount === 1 &&
      doc.firstChild?.type === schema.nodes.paragraph &&
      doc.firstChild.content.size === 0
    );
  }

  private requestPlainTextPaste(): void {
    this.clearPlainTextPasteRequest();
    this.plainTextPasteRequested = true;
    this.plainTextPasteResetTimer = setTimeout(() => this.clearPlainTextPasteRequest(), 0);
  }

  private consumePlainTextPasteRequest(): boolean {
    const requested = this.plainTextPasteRequested;
    this.clearPlainTextPasteRequest();
    return requested;
  }

  private clearPlainTextPasteRequest(): void {
    this.plainTextPasteRequested = false;
    if (this.plainTextPasteResetTimer !== null) {
      clearTimeout(this.plainTextPasteResetTimer);
      this.plainTextPasteResetTimer = null;
    }
  }

  deleteSelection(): boolean {
    this.assertActive();
    if (!this.view || !this.isEditable()) return false;
    return deleteSelection(this.view.state, this.view.dispatch);
  }

  selectAll(): boolean {
    this.assertActive();
    if (!this.view) return false;
    return selectAllCommand(this.view.state, this.view.dispatch);
  }

  selectContextTarget(target: ContextMenuTarget): boolean {
    this.assertActive();
    if (!this.view) return false;
    if (target.kind === 'table' || target.kind === 'heading' || target.kind === 'link') return true;
    const pos = this.findContextNodePosition(target);
    if (pos === null) return false;
    const node = this.view.state.doc.nodeAt(pos);
    if (!node) return false;
    try {
      this.view.dispatch(
        this.view.state.tr.setSelection(NodeSelection.create(this.view.state.doc, pos)),
      );
      return true;
    } catch {
      return false;
    }
  }

  editContextTarget(target: ContextMenuTarget): boolean {
    this.assertActive();
    if (!this.view || !this.isEditable()) return false;
    const pos = this.findContextNodePosition(target);
    if (pos === null) return false;
    if (target.kind === 'code-block')
      return CodeBlockNodeView.enterEditAt(this.view, pos, 0, 'start');
    if (target.kind === 'math-block') return MathBlockNodeView.enterEditAt(this.view, pos, 'start');
    if (target.kind === 'mermaid-block')
      return MermaidBlockNodeView.enterEditAt(this.view, pos, 'start');
    return false;
  }

  chooseContextTargetLanguage(target: ContextMenuTarget): boolean {
    this.assertActive();
    if (!this.view || !this.isEditable() || target.kind !== 'code-block') return false;
    const pos = this.findContextNodePosition(target);
    return pos === null ? false : CodeBlockNodeView.showLanguageSelectorAt(this.view, pos);
  }

  deleteContextTarget(target: ContextMenuTarget): boolean {
    this.assertActive();
    if (!this.view || !this.isEditable()) return false;
    const pos = this.findContextNodePosition(target);
    if (pos === null) return false;
    const node = this.view.state.doc.nodeAt(pos);
    if (!node) return false;
    this.view.dispatch(this.view.state.tr.delete(pos, pos + node.nodeSize).scrollIntoView());
    return true;
  }

  private findContextNodePosition(target: ContextMenuTarget): number | null {
    if (!this.view) return null;
    const expectedType =
      target.kind === 'code-block'
        ? 'code_block'
        : target.kind === 'math-block'
          ? 'math_block'
          : target.kind === 'mermaid-block'
            ? 'mermaid_block'
            : target.kind === 'image'
              ? 'image'
              : target.kind === 'heading'
                ? 'heading'
                : target.kind === 'table'
                  ? 'table'
                  : target.nodeType;
    const doc = this.view.state.doc;
    const safePos = Math.max(0, Math.min(target.pos, doc.content.size));
    const direct = doc.nodeAt(safePos);
    if (direct?.type.name === expectedType) return safePos;
    const $pos = doc.resolve(safePos);
    for (let depth = $pos.depth; depth > 0; depth -= 1) {
      if ($pos.node(depth).type.name === expectedType) return $pos.before(depth);
    }
    if ($pos.nodeAfter?.type.name === expectedType) return $pos.pos;
    if ($pos.nodeBefore?.type.name === expectedType) return $pos.pos - $pos.nodeBefore.nodeSize;
    return null;
  }

  findSearchMatches(query: string, options: EditorSearchOptions): EditorSearchMatch[] {
    this.assertActive();
    if (!this.view || !query) {
      return [];
    }

    return findEditorTextMatches(this.view.state.doc, query, options);
  }

  setSearchHighlights(matches: EditorSearchMatch[], activeIndex: number): void {
    this.assertActive();
    if (!this.view) {
      return;
    }

    const tr = this.view.state.tr.setMeta('searchHighlight', { matches, activeIndex });
    this.view.dispatch(tr);
  }

  clearSearchState(activeMatch?: EditorSearchMatch): void {
    this.assertActive();
    if (!this.view) return;

    const { selection } = this.view.state;
    let tr = this.view.state.tr.setMeta('searchHighlight', { matches: [], activeIndex: 0 });
    if (activeMatch && selection.from === activeMatch.from && selection.to === activeMatch.to) {
      tr = tr.setSelection(TextSelection.create(this.view.state.doc, activeMatch.to));
    }
    this.view.dispatch(tr);
  }

  selectSearchMatch(match: EditorSearchMatch, focus = true): boolean {
    this.assertActive();
    if (!this.view) {
      return false;
    }

    return selectEditorTextRange(this.view, match.from, match.to, focus);
  }

  replaceSearchMatch(match: EditorSearchMatch, replacement: string): boolean {
    this.assertActive();
    if (!this.view || this.runtime.readonly) {
      return false;
    }

    const selected = selectEditorTextRange(this.view, match.from, match.to);
    if (!selected) {
      return false;
    }

    const tr = this.view.state.tr.insertText(replacement, match.from, match.to).scrollIntoView();
    this.view.dispatch(tr);
    return true;
  }

  replaceAllSearchMatches(
    query: string,
    replacement: string,
    options: EditorSearchOptions,
  ): number {
    this.assertActive();
    if (!this.view || this.runtime.readonly || !query) {
      return 0;
    }

    const matches = findEditorTextMatches(this.view.state.doc, query, options);
    if (matches.length === 0) {
      return 0;
    }

    let tr = this.view.state.tr;
    for (const match of [...matches].reverse()) {
      tr = tr.insertText(replacement, match.from, match.to);
    }
    this.view.dispatch(tr.scrollIntoView());
    return matches.length;
  }

  execute(command: EditorCommand): boolean {
    if (!['insertText', 'undo', 'redo'].includes(command.type)) return this.withProjectionOperation(() => this.executeCommand(command));
    return this.executeCommand(command);
  }

  private executeCommand(command: EditorCommand): boolean {
    this.assertActive();
    this.flushPendingMarkdownSync();

    if (!this.canExecute(command)) {
      return false;
    }
    if (this.options.documentProjection && (command.type === 'undo' || command.type === 'redo')) {
      this.commitPendingEdits();
      return this.options.documentProjection.history(command.type === 'redo');
    }

    if (!this.view) {
      // 异步图片导入可能在标签挂起后完成，仍在来源文档的历史中提交。
      if (command.type === 'insertImage' && this.runtime.mode === 'semantic') {
        const state = this.suspendedState ?? this.createState(this.markdown);
        this.suspendedState = state;
        const image = schema.nodes.image.create({
          src: command.src, alt: command.alt ?? null, title: command.title ?? null,
          width: command.width ?? null, align: command.align ?? null,
        });
        this.dispatchTransaction(state.tr.replaceSelectionWith(image, false));
        return true;
      }
      return false;
    }

    // 挂载首帧会暂时关闭 DOM 编辑；合法命令开始时恢复真实运行状态。
    if (this.isEditable() && !this.view.editable) {
      this.view.setProps({ editable: () => this.isEditable() });
    }
    if (command.type === 'undo' || command.type === 'redo' || command.type === 'setDetailsDefaultOpen' || command.type === 'unwrapDetails') {
      if (isActiveEditComposing(this.view)) return false;
      // 先结束旧输入缓冲，再应用文档历史，避免失焦把撤销前的值写回。
      commitActiveEdit(this.view, true);
    }
    return this.runProseMirrorCommand(command);
  }

  canExecute(command: EditorCommand): boolean {
    if (this.destroyed || this.runtime.readonly) {
      return false;
    }
    if (this.options.documentProjection && (command.type === 'undo' || command.type === 'redo')) return this.options.documentProjection.canHistory(command.type === 'redo');
    if (this.view?.state.selection.$from.parent.type.name === 'details_summary' &&
      !['insertText', 'undo', 'redo', 'toggleBold', 'toggleItalic', 'toggleCode', 'toggleStrikethrough',
        'toggleUnderline', 'toggleHighlight', 'toggleSuperscript', 'toggleSubscript', 'clearInlineStyles',
        'insertLink', 'removeLink', 'setDetailsDefaultOpen', 'unwrapDetails'].includes(command.type)) return false;
    if (command.type === 'insertMathInline') {
      return this.runtime.mode === 'semantic' && !!this.view && this.isEditable() &&
        !isActiveEditComposing(this.view) && canInsertMathInline(this.view.state);
    }
    if (command.type === 'insertText') {
      return !!this.view && this.isEditable() && !isActiveEditComposing(this.view) &&
        this.view.state.selection instanceof TextSelection && this.view.state.selection.$from.parent.inlineContent;
    }

    if (this.runtime.mode !== 'semantic' && isPendingInlineMarkCommand(command)) {
      return false;
    }
    if (isPendingInlineMarkCommand(command)) {
      if (!this.view) return false;
      const formats = {
        toggleBold: 'strong', toggleItalic: 'em', toggleCode: 'code',
        toggleStrikethrough: 'strikethrough', toggleUnderline: 'underline', toggleHighlight: 'highlight',
        toggleSuperscript: 'superscript', toggleSubscript: 'subscript',
      } as const;
      return toggleInlineSourceFormat(formats[command.type as keyof typeof formats])(this.view.state);
    }

    return true;
  }

  /** 保留工具栏状态接口；值来自真实语法范围及尚未输入的成对模板。 */
  isPendingMarkActive(markName: InlinePendingMarkName): boolean {
    if (!this.view) return false;
    return getActiveInlineSourceFormats(this.view.state).has(markName) ||
      getEmptyInlineTemplateFormats(this.view.state).has(markName);
  }

  /**
   * 把语义编辑器的代码高亮和 Mermaid 主题切到新外观。
   *
   * 主题未变化时直接返回，避免设置窗连点配色时整页重跑 Shiki / Mermaid。
   *
   * @param theme 已解析的编辑器主题，包含 Shiki 主题名和 Mermaid 配置。
   */
  updateTheme(theme: EditorThemeOptions): void {
    this.assertActive();
    if (editorThemesEqual(this.options.theme, theme)) {
      return;
    }
    this.options.theme = theme;
    CodeBlockNodeView.updateTheme(theme);
    MermaidBlockNodeView.updateTheme(theme.mermaid);
    this.emit(`theme:${theme.name}`);
  }

  /** 派生锚点仅改变视图，不写入正文、历史或对外编辑通知。 */
  updateHeadingAnchors(ids: readonly string[]): void {
    const state = this.view?.state ?? this.suspendedState;
    if (!state) return;
    const anchors = headingAnchorsKey.getState(state);
    if (!anchors || (anchors.ids?.length === ids.length && anchors.ids.every((id, i) => id === ids[i]))) return;
    this.dispatchTransaction(state.tr.setMeta(headingAnchorsKey, [...ids]).setMeta('addToHistory', false));
  }

  headingAnchorTop(id: string): number | undefined {
    const view = this.view;
    if (!view) return undefined;
    const anchor = headingAnchorsKey.getState(view.state)?.decorations.find()
      .find(decoration => decoration.spec.headingId === id);
    const node = anchor && view.nodeDOM(anchor.from);
    return node instanceof HTMLElement ? node.getBoundingClientRect().top : undefined;
  }

  revealHeadingAnchor(id: string, scroll = true): boolean {
    const view = this.view;
    if (!view) return false;
    const anchor = headingAnchorsKey.getState(view.state)?.decorations.find()
      .find(decoration => decoration.spec.headingId === id);
    if (!anchor) return false;
    const transaction = view.state.tr.setSelection(TextSelection.near(view.state.doc.resolve(anchor.from + 1)));
    view.dispatch(scroll ? transaction.scrollIntoView() : transaction);
    // 默认沿用搜索居中；大纲由窗口宿主在挂载/布局后按独立参考线定位。
    if (scroll) scrollSearchSelectionIntoView(view, view.state.selection.head);
    return true;
  }

  updateOptions(options: Partial<EditorRuntimeOptions>): void {
    this.assertActive();
    const optionsChanged = (Object.keys(options) as Array<keyof EditorRuntimeOptions>)
      .some(key => options[key] !== this.runtime[key]);
    if (this.view) dismissQuickInsert(this.view);
    if (options.mode && options.mode !== this.runtime.mode && options.readonly !== true) {
      if (this.view) commitActiveEdit(this.view, true);
      this.commitPendingEdits();
    }
    this.runtime = {
      ...this.runtime,
      ...options,
    };
    if (this.semanticViewDirty && this.runtime.mode === 'semantic' && options.mode === 'semantic') {
      this.replaceViewState(this.markdown, undefined, true);
    }
    // NodeView 与插件不需要为重复的权限通知重新更新；编辑收束和事件仍按原顺序执行。
    if (this.view && (optionsChanged || this.view.editable !== this.isEditable())) {
      this.view.setProps({ editable: () => this.isEditable() });
    }
    // A forced hide during composition must not leave a stale draft to revive later.
    if (this.view && (this.runtime.readonly || this.runtime.mode !== 'semantic')) {
      cancelMathInlineDraft(this.view);
    }
    if (this.view && this.runtime.readonly) {
      cancelActiveEdit(this.view);
      for (const finish of [...this.compositionWaiters]) finish();
    }
    this.emit('runtime-options');
  }

  subscribe(listener: EditorListener): () => void {
    this.assertActive();
    this.listeners.add(listener);
    listener(this.createChangeEvent('subscribe'));

    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(reason: string): void {
    const event = this.createChangeEvent(reason);
    this.options.onChange?.(event);
    for (const listener of this.listeners) {
      listener(event);
    }
  }

  private createChangeEvent(reason: string): EditorChangeEvent {
    return {
      contentRevision: this.contentRevision,
      markdown: this.markdown,
      version: this.version,
      dirty: this.dirty,
      mode: this.runtime.mode,
      readonly: this.runtime.readonly,
      reason,
      pendingInlineMarks: this.createPendingInlineMarks(),
    };
  }

  private createPendingInlineMarks(): InlinePendingMarks {
    return {
      strong: this.isPendingMarkActive('strong'),
      em: this.isPendingMarkActive('em'),
      code: this.isPendingMarkActive('code'),
      strikethrough: this.isPendingMarkActive('strikethrough'),
      underline: this.isPendingMarkActive('underline'),
      highlight: this.isPendingMarkActive('highlight'),
      superscript: this.isPendingMarkActive('superscript'),
      subscript: this.isPendingMarkActive('subscript'),
    };
  }

  private createState(markdown: string): EditorState {
    this.detailsSession.prepare(markdown);
    const { doc, appended } = this.parseSemanticDocument(markdown);
    return EditorState.create({
      doc,
      selection: appended ? TextSelection.create(doc, doc.content.size - 1) : undefined,
      plugins: [
        windowsImePunctuationFallbackPlugin(),
        emojiSuggestionsPlugin(() => this.isEditable()),
        quickInsertPlugin(() => ({
          enabled: this.isEditable() && (this.runtime.quickInsertEnabled ?? true),
          trigger: this.runtime.quickInsertTrigger ?? DEFAULT_QUICK_INSERT_TRIGGER,
          onChoose: this.options.onQuickInsertPicker,
        })),
        blockAlignmentPlugin(),
        inputRules({
          rules: createMarkdownInputRules(),
        }),
        blockquoteInputPlugin(),
        ...(this.options.documentProjection ? [] : [history()]),
        taskListPlugin(),
        mathInlineInputPlugin(() => this.view),
        mathInlineDraftPlugin(),
        inlineSourceEditingPlugin({ readonly: () => !this.isEditable() }),
        linkInteractionPlugin({ openLink: this.options.onOpenLink }),
        codeHighlightPlugin(),
        codeHighlightDecorationPlugin({ enabled: false }),
        inlineCodeSelectionBridgePlugin(),
        headingLevelIndicatorPlugin(),
        ...(this.options.documentProjection ? [headingAnchorsPlugin()] : []),
        paragraphSelectionGutterPlugin(),
        searchHighlightPlugin(),
        // mathBlockPlugin(),  // 已被 math_block 语义节点 + displayMathInputPlugin 取代
        displayMathInputPlugin(),
        trailingParagraphPlugin(),
        ...(this.options.documentProjection ? [] : [tocSyncPlugin()]),
        tableHtmlBlockPlugin(),
        tableControlsPlugin(),
        tableLayoutPlugin(),
        tableEditing({ allowTableNodeSelection: true }),
        createCalloutPlugin(),
        detailsPlugin(this.detailsSession, enterDetailsBody),
        keymap({
          'Mod-z': this.options.documentProjection ? () => this.execute({ type: 'undo' }) : undo,
          'Mod-y': this.options.documentProjection ? () => this.execute({ type: 'redo' }) : redo,
          'Shift-Mod-z': this.options.documentProjection ? () => this.execute({ type: 'redo' }) : redo,
          'Mod-b': this.options.documentProjection ? () => this.execute({ type: 'toggleBold' }) : toggleInlineSourceFormat('strong'),
          'Mod-i': this.options.documentProjection ? () => this.execute({ type: 'toggleItalic' }) : toggleInlineSourceFormat('em'),
          'Ctrl-`': this.options.documentProjection ? () => this.execute({ type: 'toggleCode' }) : toggleInlineSourceFormat('code'),
          'Mod-k': (_state, _dispatch, _view) => {
            this.options.onLinkShortcut?.();
            return Boolean(this.options.onLinkShortcut);
          },
          'Alt-Shift-5': this.options.documentProjection ? () => this.execute({ type: 'toggleStrikethrough' }) : toggleInlineSourceFormat('strikethrough'),
          'Mod-u': this.options.documentProjection ? () => this.execute({ type: 'toggleUnderline' }) : toggleInlineSourceFormat('underline'),
          'Mod-\\': (_state, _dispatch, _view) =>
            this.runProseMirrorCommand({ type: 'clearInlineStyles' }),
          'Ctrl-\\': (_state, _dispatch, _view) =>
            this.runProseMirrorCommand({ type: 'clearInlineStyles' }),
          'Ctrl-Enter': (state, dispatch) => {
            if (exitDetails()(state, dispatch)) return true;
            // 在表格内：下方插入新行
            const context = findTableContext(state);
            if (context) return addTableRowAfter()(state, dispatch);
            // 在 callout 内：跳出 callout，在下方插入段落
            const { $from: $fromCtrl } = state.selection;
            for (let d = $fromCtrl.depth; d >= 0; d--) {
              if ($fromCtrl.node(d).type.name === 'callout') {
                const calloutEnd = $fromCtrl.after(d);
                const emptyParagraph = schema.nodes.paragraph.create();
                const tr = state.tr.insert(calloutEnd, emptyParagraph);
                if (dispatch) {
                  dispatch(
                    tr.setSelection(TextSelection.create(tr.doc, calloutEnd + 1)).scrollIntoView(),
                  );
                }
                return true;
              }
            }
            // 其他块：在下方插入新段落
            const { $from } = state.selection;
            const afterPos = state.selection instanceof NodeSelection && $from.depth === 0
              ? state.selection.to
              : $from.after(1);
            const emptyParagraph = schema.nodes.paragraph.create();
            const tr = state.tr.insert(afterPos, emptyParagraph);
            const newPos = afterPos + 1;
            if (dispatch) {
              dispatch(tr.setSelection(TextSelection.create(tr.doc, newPos)).scrollIntoView());
            }
            return true;
          },
          'Shift-Ctrl-Enter': (state, dispatch) => {
            if (exitDetails(true)(state, dispatch)) return true;
            // 在表格内：上方插入新行
            const context = findTableContext(state);
            if (context) return addTableRowBefore()(state, dispatch);
            // 其他块：在上方插入新段落
            const { $from } = state.selection;
            const beforePos = $from.before(1);
            const emptyParagraph = schema.nodes.paragraph.create();
            const tr = state.tr.insert(beforePos, emptyParagraph);
            if (dispatch) {
              dispatch(
                tr.setSelection(TextSelection.create(tr.doc, beforePos + 1)).scrollIntoView(),
              );
            }
            return true;
          },
          'Shift-Enter': chainCommands(
            enterDetailsBody,
            insertParagraphLineBreak,
            newlineInCode,
            (state, dispatch) =>
              state.selection.$from.parent.type === schema.nodes.footnote_def &&
              splitBlockExitHeading(state, dispatch),
            splitTaskListItem(schema.nodes.list_item),
            splitListItem(schema.nodes.list_item),
            createParagraphNear,
            liftEmptyBlock,
            splitBlockExitHeading,
          ),
          Enter: chainCommands(
            enterDetailsBody,
            exitDetailsEmptyParagraph,
            // $$ 回车自动补全：段落内只有 $$ 时按回车，生成空 math_block 并自动进入编辑态
            (state, dispatch) => {
              const { $from, empty } = state.selection;
              if (!empty || $from.parent.type.name !== 'paragraph') return false;
              if ($from.parent.textContent !== '$$') return false;
              for (let index = 0; index < $from.parent.childCount; index++) {
                if (!$from.parent.child(index).isText) return false;
              }
              const parent = $from.node($from.depth - 1);
              const paragraphIndex = $from.index($from.depth - 1);
              // 表格等容器有内容约束，不能让替换事务越界修复结构。
              if (
                !parent.canReplaceWith(paragraphIndex, paragraphIndex + 1, schema.nodes.math_block)
              ) {
                return false;
              }
              if (dispatch) {
                const blockStart = $from.before($from.depth);
                const blockEnd = $from.after($from.depth);
                const node = schema.nodes.math_block.create({ tex: '' });
                const tr = state.tr.replaceWith(blockStart, blockEnd, node);
                // 选中新创建的 math_block，触发 NodeView.selectNode → 自动进入编辑态
                tr.setSelection(NodeSelection.create(tr.doc, blockStart));
                dispatch(tr);
              }
              return true;
            },
            newlineInCode,
            (state, dispatch) =>
              state.selection.$from.parent.type === schema.nodes.footnote_def &&
              splitBlockExitHeading(state, dispatch),
            splitTaskListItem(schema.nodes.list_item),
            splitListItem(schema.nodes.list_item),
            createParagraphNear,
            liftEmptyBlock,
            splitBlockExitHeading,
          ),
          Backspace: chainCommands(
            deleteSelection,
            detailsBackspace,
            deleteTaskListItemBeforeCursor,
            removeEmptyCalloutOnBackspace,
            deleteCodeBlockBeforeCursor,
            joinTaskListBackward,
            selectNodeBackward,
          ),
          Delete: chainCommands(deleteSelection, detailsDeleteBoundary, joinForward, selectNodeForward),
          Space: convertMarkdownListShortcut,
          Tab: chainCommands(
            convertMarkdownListShortcut,
            goToNextCell(1),
            indentListItems,
            insertTabInTextblock,
          ),
          'Shift-Tab': chainCommands(
            goToNextCell(-1),
            outdentListItems,
            removeTabBeforeCursorInTextblock,
          ),
          'Shift-Ctrl-[': (state, dispatch) =>
            state.selection.$from.parent.type.name !== 'details_summary' && toggleList(state, dispatch, schema.nodes.ordered_list),
          'Shift-Ctrl-]': (state, dispatch) =>
            state.selection.$from.parent.type.name !== 'details_summary' && toggleList(state, dispatch, schema.nodes.bullet_list),
          'Shift-Ctrl-x': (state, dispatch) => state.selection.$from.parent.type.name !== 'details_summary' && toggleTaskListAtCursor(state, dispatch),
          'Shift-Ctrl-q': (_state, _dispatch, view) =>
            this.runProseMirrorCommand({ type: 'toggleBlockquote' }),
          'Shift-Ctrl-a': (_state, _dispatch, view) =>
            this.runProseMirrorCommand({ type: 'insertCallout' }),
          'Shift-Ctrl-m': (_state, _dispatch, view) =>
            this.runProseMirrorCommand({ type: 'insertMathBlock', tex: '' }),
          'Shift-Ctrl-k': (_state, _dispatch, view) =>
            this.runProseMirrorCommand({ type: 'insertCodeBlock' }),
          ArrowRight: (state, dispatch) => {
            const { $from, empty } = state.selection;
            if (!empty) return false;
            const nodeAfter = $from.nodeAfter;
            if (nodeAfter?.type.name === 'math_inline') {
              if (dispatch) {
                MathInlineNodeView.requestKeyboardEntry('start', this.view ?? undefined);
                dispatch(state.tr.setSelection(NodeSelection.create(state.doc, $from.pos)));
              }
              return true;
            }
            if (nodeAfter?.type.name === 'comment_inline') {
              if (dispatch) {
                CommentInlineNodeView.requestKeyboardEntry('start', this.view ?? undefined);
                dispatch(state.tr.setSelection(NodeSelection.create(state.doc, $from.pos)));
              }
              return true;
            }
            return false;
          },
          ArrowLeft: (state, dispatch) => {
            const { $from, empty } = state.selection;
            if (!empty) return false;
            const nodeBefore = $from.nodeBefore;
            if (nodeBefore?.type.name === 'math_inline') {
              if (dispatch) {
                MathInlineNodeView.requestKeyboardEntry('end', this.view ?? undefined);
                dispatch(
                  state.tr.setSelection(
                    NodeSelection.create(state.doc, $from.pos - nodeBefore.nodeSize),
                  ),
                );
              }
              return true;
            }
            if (nodeBefore?.type.name === 'comment_inline') {
              if (dispatch) {
                CommentInlineNodeView.requestKeyboardEntry('end', this.view ?? undefined);
                dispatch(
                  state.tr.setSelection(
                    NodeSelection.create(state.doc, $from.pos - nodeBefore.nodeSize),
                  ),
                );
              }
              return true;
            }
            return false;
          },
        }),
        // 必须在 keymap 之后注册：ProseMirror 的 someProp 取最后一个结果，
        // 若在 keymap 之前，keymap 返回 false 会覆盖本插件的 true。
        codeBlockNavigationPlugin({
          enterEditAt: (view, pos, clickLine, caret) =>
            CodeBlockNodeView.enterEditAt(view, pos, clickLine, caret),
          enterMathEditAt: (view, pos, caret) => MathBlockNodeView.enterEditAt(view, pos, caret),
          enterMermaidEditAt: (view, pos, caret) =>
            MermaidBlockNodeView.enterEditAt(view, pos, caret),
          prepareMathKeyboardEntry: (caret) => MathBlockNodeView.prepareKeyboardEntry(caret, this.view ?? undefined),
        }),
        // Format boundaries and editable atoms handle arrows before KP's text navigation.
        typographyBreaksPlugin(),
        typographyPlugin(),
        contextMenuPlugin({
          onOpen: (event) => this.options.onContextMenuOpen?.(event),
        }),
      ],
    });
  }

  private parseSemanticDocument(markdown: string, withSource = !!this.options.documentProjection): TrailingParagraphNormalization {
    const references = this.options.documentProjection?.references;
    if (this.parsedDocument?.markdown === markdown && this.parsedDocument.references === references) {
      if (withSource && !this.parsedDocument.source)
        this.parsedDocument.source = parseMarkdownWithSyncAnchors(markdown, this.options.documentProjection);
      return this.parsedDocument.value;
    }
    // 会话窗口必需测量结构位置，在同一次解析中记录来源；尾段规范化共享原节点。
    // 非窗口编辑器仍按需生成映射，不为未使用的同步功能增加初始化工作。
    const source = withSource ? parseMarkdownWithSyncAnchors(markdown, this.options.documentProjection) : undefined;
    const value = ensureTrailingParagraph(source?.doc ?? parseMarkdown(markdown, this.options.documentProjection));
    // 单个不可变解析结果，与当前模型共享节点；不保留正文历史、DOM 或回调。
    this.parsedDocument = { markdown, references, value, source };
    return value;
  }

  private dispatchTransaction(transaction: Transaction): void {
    const currentState = this.view?.state ?? this.suspendedState;
    if (!currentState || (this.runtime.readonly && transaction.docChanged)) return;
    const previousDoc = currentState.doc;
    const previousSelection = currentState.selection;
    const nextState = currentState.apply(transaction);
    // 空模板清理和原文标记可能由 appendTransaction 完成，不能只看首个事务。
    const docChanged = !nextState.doc.eq(previousDoc);
    if (this.runtime.readonly && docChanged) return;
    if (this.view) this.view.updateState(nextState);
    else this.suspendedState = nextState;

    if (isTypographyTransaction(transaction) && !docChanged) {
      this.syncRenderRevision += 1;
      return;
    }
    if (isSemanticBlockAlignmentTransaction(transaction) && !docChanged) {
      return;
    }
    if (transaction.getMeta(headingAnchorsKey) !== undefined && !docChanged) return;
    const visibilityOnly = transaction.getMeta(detailsKey) && !docChanged;
    if (!docChanged && detailsKey.getState(currentState)?.decorations !== detailsKey.getState(nextState)?.decorations) {
      this.syncRenderRevision += 1;
      // 显式切换和搜索/历史引起的自动展开均改变双栏几何，不改变正文修订。
    }

    if (docChanged) {
      if (this.sourceProjection && !this.pendingMarkdownDoc) {
        this.sourceSelectionBefore = this.projectSelection(this.sourceProjection, previousSelection);
      }
      if (this.view) FootnoteRefNodeView.refreshOpenPreviews(this.view);
      this.contentRevision += 1;
      this.syncRenderRevision += 1;
      this.syncSnapshot = null;
      this.pendingMarkdownDoc = nextState.doc;
      this.dirty = !nextState.doc.eq(this.originalDoc);
      this.scheduleMarkdownSync();
      this.notifyDeletedImages(previousDoc, nextState.doc);
    }

    // 每次事务都递增版本并通知（pending mark 状态切换、选区变化等需要及时反映到 UI）
    if (!visibilityOnly) this.version += 1;
    this.emit(visibilityOnly ? 'details-visibility' : docChanged ? 'content-pending' : 'transaction');
    if (!nextState.selection.eq(previousSelection)) {
      this.options.onSelectionSnapshotChange?.({
        selection: nextState.selection.empty ? null : {
          anchor: nextState.selection.anchor, head: nextState.selection.head,
        },
        contentRevision: this.contentRevision,
      });
      this.options.onSelectionChange?.(this.createSelectionEvent());
    }
  }

  private createSelectionEvent(): EditorSelectionEvent {
    if (!this.view || this.view.state.selection.empty) {
      return { selection: null, selectedMarkdown: '', caret: this.getScrollSyncCaret() ?? undefined };
    }

    const { doc, selection } = this.view.state;
    const clipboardDoc = removeEmptyTrailingParagraph(doc);
    const selectionTo = Math.min(selection.to, clipboardDoc.content.size);
    const selectedMarkdown =
      serializeMarkdownSelection(clipboardDoc, selection.from, selectionTo) ??
      serializeClipboardText(selection.content(), true);

    return {
      selection: { anchor: selection.anchor, head: selection.head },
      selectedMarkdown,
      caret: this.getScrollSyncCaret() ?? undefined,
    };
  }

  setSavedMarkdownBaseline(markdown: string): void {
    this.assertActive();
    this.commitPendingEdits();
    this.originalMarkdown = this.options.documentProjection ? markdown : updateTocBlocks(markdown);
    this.originalDoc = this.parseSemanticDocument(this.originalMarkdown).doc;
    this.dirty = this.markdown !== this.originalMarkdown;
    this.emit('saved-baseline');
  }

  getDocumentStatsSnapshot(): Record<string, unknown> | null {
    return this.view?.state.doc.toJSON() ?? null;
  }

  getSelectionStatsSnapshot(): { anchor: number; head: number } | null {
    const selection = this.view?.state.selection;
    return selection && !selection.empty ? { anchor: selection.anchor, head: selection.head } : null;
  }

  private scheduleMarkdownSync(): void {
    this.clearMarkdownSyncTimer();
    this.markdownSyncTimer = setTimeout(() => {
      this.markdownSyncTimer = null;
      try { this.flushPendingMarkdownSync(); }
      catch (cause) { this.options.onError?.({ code: 'source-projection-failed', message: cause instanceof Error ? cause.message : '原文写回失败，编辑仍保留在当前视图。', cause }); }
    }, MARKDOWN_SYNC_DEBOUNCE_MS);
  }

  private clearMarkdownSyncTimer(): void {
    if (this.markdownSyncTimer !== null) {
      clearTimeout(this.markdownSyncTimer);
      this.markdownSyncTimer = null;
    }
  }

  private clearPendingMarkdownSync(): void {
    this.clearMarkdownSyncTimer();
    this.pendingMarkdownDoc = null;
  }

  private flushPendingMarkdownSync(): boolean {
    const pendingDoc = this.pendingMarkdownDoc;
    if (!pendingDoc) {
      return false;
    }

    if (this.sourceProjection && this.options.documentProjection) {
      // 先验证并提交补丁，再清空待提交输入；失败不能把窗口当作已经保存的正文。
      const change = this.sourceProjection.project(pendingDoc);
      const selection = (this.view?.state ?? this.suspendedState)?.selection;
      const nextProjection = new MarkdownSourceProjection(change.source, pendingDoc, this.options.documentProjection.fromByte, this.options.documentProjection);
      this.options.documentProjection.commit({ ...change, selectionBefore: this.sourceSelectionBefore,
        selectionAfter: selection && this.projectSelection(nextProjection, selection) });
      this.sourceProjection = nextProjection; this.sourceSelectionBefore = undefined;
      this.clearPendingMarkdownSync(); this.markdown = change.source;
      this.dirty = this.markdown !== this.originalMarkdown; this.semanticViewDirty = false;
      this.emit('content-sync'); return true;
    }

    this.clearPendingMarkdownSync();

    const serializedMarkdown = pendingDoc.eq(this.originalDoc)
      ? this.originalMarkdown
      : pendingDoc.content.eq(this.originalDoc.content)
        ? `${String(pendingDoc.attrs.frontMatterPrefix ?? '')}${splitFrontMatter(this.originalMarkdown).body}`
        : serializeMarkdown(removeEmptyTrailingParagraph(pendingDoc));
    // 保留 Markdown 字符串层的 TOC 规范化，但不再因此重建 EditorState。
    // 语义视图中的 toc_block 已由 tocSyncPlugin 原位同步，历史栈保持不变。
    this.markdown = updateTocBlocks(serializedMarkdown);
    this.dirty = this.markdown !== this.originalMarkdown;
    this.semanticViewDirty = false;
    this.emit('content-sync');
    return true;
  }

  private projectSelection(projection: MarkdownSourceProjection, selection: { anchor: number; head: number }): DocumentSelection | undefined {
    const anchorByte = projection.positionToByte(selection.anchor);
    const headByte = selection.anchor === selection.head ? anchorByte : projection.positionToByte(selection.head);
    return anchorByte !== null && headByte !== null ? { anchorByte, headByte } : undefined;
  }

  private replaceViewState(markdown: string, selection?: { anchor: number; head: number }, preserveCodeViewport = false): void {
    if (!this.view) {
      this.suspendedState = this.createState(markdown);
      if (selection) this.suspendedState = this.restoreSelection(this.suspendedState, selection);
      this.semanticViewDirty = false;
      this.syncRenderRevision += 1;
      this.syncSnapshot = null;
      return;
    }
    if (this.runtime.mode === 'source' && markdown.length > LARGE_DOCUMENT_SEMANTIC_LIMIT) {
      return;
    }

    const nextState = this.createState(markdown);
    this.syncRenderRevision += 1;
    this.syncSnapshot = null;
    this.blockAlignmentGaps.clear();
    const update = () => this.view!.updateState(selection ? this.restoreSelection(nextState, selection) : nextState);
    if (preserveCodeViewport) CodeBlockNodeView.preserveViewports(this.view, nextState.doc, update);
    else update();
    FootnoteRefNodeView.refreshOpenPreviews(this.view);
    this.semanticViewDirty = false;
  }

  private restoreSelection(
    state: EditorState,
    selection: { anchor: number; head: number },
  ): EditorState {
    return state.apply(state.tr.setSelection(this.createRestoredSelection(state, selection)));
  }

  private createRestoredSelection(state: EditorState, selection: { anchor: number; head: number }): Selection {
    const anchor = clampDocPosition(state.doc, selection.anchor);
    const head = clampDocPosition(state.doc, selection.head);

    try {
      return TextSelection.create(state.doc, anchor, head);
    } catch {
      return TextSelection.near(state.doc.resolve(head));
    }
  }

  private isEditable(): boolean {
    return !this.runtime.readonly && this.runtime.mode === 'semantic';
  }

  private refreshInitialEditableState(): void {
    const view = this.view;
    if (!view || (!this.isEditable() && !this.options.documentProjection?.onViewReady)) {
      return;
    }

    view.setProps({ editable: () => false });
    this.initialEditableFrame = requestAnimationFrame(() => {
      this.initialEditableFrame = null;
      if (this.destroyed || this.view !== view) {
        return;
      }
      view.setProps({ editable: () => this.isEditable() });
      this.options.documentProjection?.onViewReady?.();
    });
  }

  /** 窗口在接入 DOM 前完成同一就绪交接，避免先布局只读树再切换可编辑性。 */
  prepareDetachedProjection(): void {
    const view = this.view;
    if (!view || view.dom.isConnected || this.destroyed || !this.options.documentProjection?.onViewReady ||
      this.initialEditableFrame === null) return;
    cancelAnimationFrame(this.initialEditableFrame);
    this.initialEditableFrame = null;
    view.setProps({ editable: () => this.isEditable() });
    this.options.documentProjection.onViewReady();
  }

  private runProseMirrorCommand(command: EditorCommand): boolean {
    if (!this.view || !this.canExecute(command)) {
      return false;
    }
    return executeEditorCommand(command, this.view, this.markdown, (markdown, options) =>
      this.setMarkdown(markdown, options),
    );
  }

  private assertActive(): void {
    if (this.destroyed) {
      throw new Error('EditorCore has been destroyed.');
    }
  }

  private notifyDeletedImages(previousDoc: ProseMirrorNode, nextDoc: ProseMirrorNode): void {
    if (this.options.documentProjection) return;
    if (!this.options.onImagesDeleted) {
      return;
    }

    const deletedSrcs = findFullyRemovedImageSrcs(previousDoc, nextDoc);
    if (deletedSrcs.length === 0) return;
    // 当前语义事务尚未同步 this.markdown；用 nextDoc 校验真实删除候选。
    const nextMarkdown = serializeMarkdown(removeEmptyTrailingParagraph(nextDoc));
    this.notifyDeletedImageSrcs(filterFullyRemovedImageSrcs(deletedSrcs, nextMarkdown));
  }

  private notifyDeletedImageSrcs(deletedSrcs: string[]): void {
    if (this.options.documentProjection) return;
    if (!this.options.onImagesDeleted) {
      return;
    }

    if (deletedSrcs.length === 0) {
      return;
    }

    const event: EditorImageDeletionEvent = { srcs: deletedSrcs };
    this.options.onImagesDeleted(event);
  }
}

function isPendingInlineMarkCommand(command: EditorCommand): boolean {
  return (
    command.type === 'toggleBold' ||
    command.type === 'toggleItalic' ||
    command.type === 'toggleCode' ||
    command.type === 'toggleStrikethrough' ||
    command.type === 'toggleUnderline' ||
    command.type === 'toggleHighlight' ||
    command.type === 'toggleSuperscript' ||
    command.type === 'toggleSubscript'
  );
}

function insertTabInTextblock(state: EditorState, dispatch?: (tr: Transaction) => void): boolean {
  const { $from } = state.selection;
  if (!$from.parent.isTextblock) {
    return false;
  }
  if (dispatch) {
    dispatch(state.tr.insertText('\t').scrollIntoView());
  }
  return true;
}

function removeTabBeforeCursorInTextblock(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  const { selection } = state;
  if (!selection.empty || !selection.$from.parent.isTextblock) {
    return false;
  }

  const cursor = selection.from;
  if (cursor <= selection.$from.start() || state.doc.textBetween(cursor - 1, cursor) !== '\t') {
    return false;
  }

  if (dispatch) {
    dispatch(state.tr.delete(cursor - 1, cursor).scrollIntoView());
  }
  return true;
}

function splitTaskListItem(itemType: typeof schema.nodes.list_item) {
  return (state: EditorState, dispatch?: (tr: Transaction) => void): boolean => {
    const taskItem = findCurrentTaskListItem(state);
    if (!taskItem || !state.selection.empty) {
      return false;
    }

    const visibleText = taskItem.itemNode.textContent.slice(taskItem.markerLength).trim();
    if (!visibleText && taskItem.itemNode.childCount === 1) {
      const paragraph = taskItem.itemNode.firstChild!;
      let textOnly = true;
      paragraph.forEach((node) => {
        if (!node.isText) textOnly = false;
      });
      if (textOnly) {
        // 隐藏的任务标记仍是正文；先清空，再退出当前列表层级，保持一次撤销。
        const tr = state.tr.delete(
          taskItem.markerRange.from,
          taskItem.markerRange.from + paragraph.content.size,
        );
        if (!appendCommandTransaction(state, tr, liftListItem(itemType))) {
          return false;
        }
        if (dispatch) dispatch(tr.scrollIntoView());
        return true;
      }
    }

    let capturedTr: Transaction | null = null;
    const handled = splitListItem(itemType)(state, (tr) => {
      capturedTr = tr;
    });
    if (!handled || !capturedTr) {
      return false;
    }

    if (dispatch) {
      const tr = capturedTr as Transaction;
      const nextSelection = tr.selection;
      if (isSelectionInsideListItem(nextSelection.$from)) {
        tr.insertText('[ ] ', nextSelection.from);
      }
      dispatch(tr.scrollIntoView());
    }
    return true;
  };
}

function joinTaskListBackward(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
  view?: EditorView,
): boolean {
  const { $from, empty } = state.selection;
  if (!empty || $from.parent.type !== schema.nodes.paragraph || $from.parent.content.size !== 0) {
    return joinBackward(state, dispatch, view);
  }

  const previousList = state.doc.resolve($from.before()).nodeBefore;
  const lastParagraph = previousList?.lastChild?.firstChild;
  if (
    !previousList ||
    (previousList.type !== schema.nodes.bullet_list && previousList.type !== schema.nodes.ordered_list) ||
    !lastParagraph || !readTaskListMarker(lastParagraph)
  ) {
    return joinBackward(state, dispatch, view);
  }

  // 复用原有列表恢复事务，只给新恢复的空列表项补上未勾选任务标记。
  return joinBackward(state, dispatch ? (tr) => {
    const next = tr.selection.$from;
    if (
      next.depth >= 2 && next.parent.type === schema.nodes.paragraph &&
      next.parent.content.size === 0 && next.node(-1).type === schema.nodes.list_item &&
      next.index(-1) === 0
    ) {
      tr.insertText('[ ] ', next.pos);
    }
    dispatch(tr.scrollIntoView());
  } : undefined, view);
}

function deleteTaskListItemBeforeCursor(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  const taskItem = findCurrentTaskListItem(state);
  if (!taskItem || !state.selection.empty) {
    return false;
  }

  const cursor = state.selection.from;
  if (cursor < taskItem.markerRange.from - 1 || cursor > taskItem.markerRange.to) {
    return false;
  }

  if (dispatch) {
    const tr = state.tr;
    if (taskItem.parentListNode.childCount === 1) {
      tr.replaceWith(
        taskItem.parentListPos,
        taskItem.parentListPos + taskItem.parentListNode.nodeSize,
        schema.nodes.paragraph.create(),
      );
      dispatch(
        tr.setSelection(TextSelection.create(tr.doc, taskItem.parentListPos + 1)).scrollIntoView(),
      );
      return true;
    }

    const nextSelectionPos =
      taskItem.itemIndex > 0
        ? taskItem.itemPos - 1
        : taskItem.itemPos + taskItem.itemNode.nodeSize + 1;
    tr.delete(taskItem.itemPos, taskItem.itemPos + taskItem.itemNode.nodeSize);
    const safeSelectionPos = clampDocPosition(tr.doc, tr.mapping.map(nextSelectionPos, -1));
    dispatch(
      tr.setSelection(TextSelection.near(tr.doc.resolve(safeSelectionPos))).scrollIntoView(),
    );
  }
  return true;
}

function findCurrentTaskListItem(state: EditorState): {
  itemNode: ProseMirrorNode;
  itemPos: number;
  itemIndex: number;
  parentListNode: ProseMirrorNode;
  parentListPos: number;
  markerLength: number;
  markerRange: { from: number; to: number };
} | null {
  const { $from } = state.selection;
  for (let depth = $from.depth; depth >= 0; depth -= 1) {
    const itemNode = $from.node(depth);
    if (itemNode.type !== schema.nodes.list_item) {
      continue;
    }

    const parentListNode = $from.node(depth - 1);
    if (
      parentListNode.type !== schema.nodes.bullet_list &&
      parentListNode.type !== schema.nodes.ordered_list
    ) {
      return null;
    }

    const itemPos = $from.before(depth);
    const paragraph = itemNode.firstChild;
    const marker = paragraph ? readTaskListMarker(paragraph) : null;
    if (!marker) return null;

    const markerFrom = itemPos + 2;
    return {
      itemNode,
      itemPos,
      itemIndex: $from.index(depth - 1),
      parentListNode,
      parentListPos: $from.before(depth - 1),
      markerLength: marker.text.length,
      markerRange: {
        from: markerFrom,
        to: markerFrom + marker.text.length,
      },
    };
  }
  return null;
}

function isSelectionInsideListItem($pos: ResolvedPos): boolean {
  for (let depth = $pos.depth; depth >= 0; depth -= 1) {
    if ($pos.node(depth).type === schema.nodes.list_item) {
      return true;
    }
  }
  return false;
}

function isSelectionInsideNodeType($pos: ResolvedPos, ...typeNames: string[]): boolean {
  for (let depth = $pos.depth; depth > 0; depth -= 1) {
    if (typeNames.includes($pos.node(depth).type.name)) return true;
  }
  return false;
}

function appendCommandTransaction(
  state: EditorState,
  tr: Transaction,
  command: (state: EditorState, dispatch?: (tr: Transaction) => void) => boolean,
): boolean {
  let capturedTr: Transaction | null = null;
  const handled = command(state.apply(tr), (nextTr) => {
    capturedTr = nextTr;
  });
  if (!handled || !capturedTr) {
    return false;
  }

  const selectionAnchor = (capturedTr as Transaction).selection.anchor;
  for (const step of (capturedTr as Transaction).steps) {
    tr.step(step);
  }
  const nextAnchor = clampDocPosition(tr.doc, selectionAnchor);
  tr.setSelection(TextSelection.near(tr.doc.resolve(nextAnchor)));
  return true;
}

function clampDocPosition(doc: ProseMirrorNode, position: number): number {
  return Math.max(0, Math.min(position, doc.content.size));
}

function findEditorTextMatches(
  doc: ProseMirrorNode,
  query: string,
  options: EditorSearchOptions,
): EditorSearchMatch[] {
  const needle = options.caseSensitive ? query : query.toLocaleLowerCase();
  const matches: EditorSearchMatch[] = [];

  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) {
      return true;
    }

    const source = options.caseSensitive ? node.text : node.text.toLocaleLowerCase();
    let offset = 0;
    while (offset <= source.length) {
      const found = source.indexOf(needle, offset);
      if (found < 0) {
        break;
      }

      const from = pos + found;
      const to = from + query.length;
      if (!options.wholeWord || isWholeWordRange(node.text, found, found + query.length)) {
        matches.push({
          id: `${from}:${to}:${matches.length}`,
          index: matches.length,
          from,
          to,
          text: node.text.slice(found, found + query.length),
        });
      }
      offset = found + Math.max(needle.length, 1);
    }

    return true;
  });

  return matches;
}

function selectEditorTextRange(view: EditorView, from: number, to: number, focus = true): boolean {
  const safeFrom = clampDocPosition(view.state.doc, from);
  const safeTo = clampDocPosition(view.state.doc, to);

  try {
    view.dispatch(
      view.state.tr
        .setSelection(TextSelection.create(view.state.doc, safeFrom, safeTo))
        .scrollIntoView(),
    );
    if (focus) {
      view.focus();
    }
    scrollSearchSelectionIntoView(view, safeFrom);
    return true;
  } catch {
    return false;
  }
}

function scrollSearchSelectionIntoView(view: EditorView, pos: number) {
  const scrollContainer = view.dom.closest<HTMLElement>('.semantic-pane');
  if (!scrollContainer || scrollContainer.clientHeight <= 0) {
    return;
  }

  try {
    const selectionRect = view.coordsAtPos(pos);
    const containerRect = scrollContainer.getBoundingClientRect();
    const targetTop =
      scrollContainer.scrollTop +
      selectionRect.top -
      containerRect.top -
      scrollContainer.clientHeight / 2 +
      Math.max(1, selectionRect.bottom - selectionRect.top) / 2;
    const maxScrollTop = Math.max(0, scrollContainer.scrollHeight - scrollContainer.clientHeight);
    const top = Math.min(Math.max(0, targetTop), maxScrollTop);

    if (typeof scrollContainer.scrollTo === 'function') {
      scrollContainer.scrollTo({ top, behavior: 'instant' });
    } else {
      scrollContainer.scrollTop = top;
    }
  } catch {
    // 搜索跳转不能因为个别 NodeView 坐标不可取而中断选区更新。
  }
}

function shouldReportImageDeletion(options: SetMarkdownOptions | undefined): boolean {
  return (
    options?.reason !== 'open-file' &&
    options?.reason !== 'save-file' &&
    options?.reason !== 'switch-tab' &&
    options?.reason !== 'restore-snapshot'
  );
}

function findFullyRemovedImageSrcs(
  previousDoc: ProseMirrorNode,
  nextDoc: ProseMirrorNode,
): string[] {
  const previous = countImageSrcs(previousDoc);
  const next = new Set([...countImageSrcs(nextDoc).keys()].map(normalizeImageSrc));
  return [...previous.keys()].filter((src) => !next.has(normalizeImageSrc(src)));
}

function findFullyRemovedMarkdownImageSrcs(
  previousMarkdown: string,
  nextMarkdown: string,
): string[] {
  const previous = collectMarkdownImageSrcs(previousMarkdown);
  if (previous === null) return [];
  return filterFullyRemovedImageSrcs([...previous], nextMarkdown);
}

/** 只有 tokenizer 和实际解析均确认无引用时，才允许发出资源删除通知。 */
function filterFullyRemovedImageSrcs(srcs: string[], nextMarkdown: string): string[] {
  if (srcs.length === 0) return [];
  const next = collectMarkdownImageSrcs(nextMarkdown);
  if (next === null) return [];
  const referenced = new Set([...next].map(normalizeImageSrc));
  const candidates = srcs.filter((src) => !referenced.has(normalizeImageSrc(src)));
  if (candidates.length === 0) return [];

  // 源码输入常规更新只做 token 扫描；潜在磁盘删除才校验完整文档，解析失败保留资源。
  const parsed = tryParseMarkdown(nextMarkdown);
  if (parsed === null) return [];
  for (const src of countImageSrcs(parsed).keys()) referenced.add(normalizeImageSrc(src));
  const seen = new Set<string>();
  return candidates.filter((src) => {
    const normalized = normalizeImageSrc(src);
    if (!normalized || referenced.has(normalized) || seen.has(normalized)) return false;
    seen.add(normalized);
    return true;
  });
}

function countImageSrcs(doc: ProseMirrorNode): Map<string, number> {
  const counts = new Map<string, number>();
  doc.descendants((node) => {
    if (node.type.name !== 'image') {
      return true;
    }

    const src = String(node.attrs.src ?? '').trim();
    if (src) {
      counts.set(src, (counts.get(src) ?? 0) + 1);
    }
    return false;
  });
  return counts;
}
