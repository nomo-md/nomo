import { serializeClipboardText } from './markdownSerialization';
import { typographyPlugin, isTypographyTransaction } from './plugins/typography';
import { TypographyBreakNodeView } from './nodeViews/TypographyBreakNodeView';
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
  toggleMark,
} from 'prosemirror-commands';
import { history, redo, undo } from 'prosemirror-history';
import { inputRules } from 'prosemirror-inputrules';
import { keymap } from 'prosemirror-keymap';
import { EditorState, NodeSelection, TextSelection, type Transaction } from 'prosemirror-state';
import { Slice, DOMSerializer, type Node as ProseMirrorNode, type ResolvedPos } from 'prosemirror-model';
import { EditorView } from 'prosemirror-view';
import { liftListItem, sinkListItem, splitListItem, wrapInList } from 'prosemirror-schema-list';
import { goToNextCell, tableEditing } from 'prosemirror-tables';
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
import { commitActiveEdit } from './nodeViews/activeEditRegistry';
import {
  executeEditorCommand,
  insertSoftLineBreak,
  splitBlockExitHeading,
  toggleList,
  toggleTaskListAtCursor,
} from './editorCommands';
import { findActiveLinkRange } from './editorCommands';
import { blockquoteInputPlugin } from './plugins/blockquoteInput';
import { codeHighlightPlugin } from './plugins/codeHighlight';
import { codeHighlightDecorationPlugin } from './plugins/codeHighlightDecorationPlugin';
import { inlineCodeSelectionBridgePlugin } from './plugins/inlineCodeSelectionBridge';
import { headingLevelIndicatorPlugin } from './plugins/headingLevelIndicator';
import { codeBlockNavigationPlugin } from './plugins/codeBlockNavigation';
import { displayMathInputPlugin } from './plugins/displayMathInput';
import { mathInlineInputPlugin } from './plugins/mathInlineInput';
import { inlineMarkdownMarkInputPlugin } from './plugins/inlineMarkdownMarkInput';
import { linkInteractionPlugin } from './plugins/linkInteraction';
import {
  pendingInlineMarkPlugin,
  toggleMarkPending,
  isPendingMarkActive,
} from './plugins/pendingInlineMark';
import { tableControlsPlugin } from './plugins/tableControls';
import { tableHtmlBlockPlugin } from './plugins/tableHtml';
import { taskListPlugin } from './plugins/taskList';
import { tocSyncPlugin } from './plugins/tocSync';
import { createCalloutPlugin } from './callout/calloutPlugin';
import { removeEmptyCalloutOnBackspace } from './callout/calloutCommands';
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
  private runtime: EditorRuntimeOptions;
  private listeners = new Set<EditorListener>();
  private blockAlignmentGaps = new Map<string, number>();
  private contentRevision = 0;
  private syncRenderRevision = 0;
  private syncSnapshot: EditorSyncSnapshot | null = null;

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
    this.markdown = updateTocBlocks(options.markdown);
    this.originalMarkdown = this.markdown;
    this.originalDoc = this.parseSemanticDocument(this.markdown).doc;
    this.runtime = {
      readonly: options.readonly ?? false,
      mode: options.mode ?? 'semantic',
      copyMarkdownSyntaxEnabled: options.copyMarkdownSyntaxEnabled ?? true,
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
    this.view = new EditorView(target, {
      state: this.suspendedState ?? this.createState(this.markdown),
      dispatchTransaction: (transaction) => this.dispatchTransaction(transaction),
      editable: () => this.isEditable(),
      clipboardTextSerializer: (slice) => this.serializeClipboardText(slice),
      clipboardTextParser: (text, $context) => createPlainTextSlice(text, $context),
      handlePaste: (_view, event) => this.handleNativePaste(event),
      handleDOMEvents: {
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
      },
      nodeViews: {
        hard_break: (node, view, getPos) => new TypographyBreakNodeView(node, view, getPos),
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
    if (this.view) commitActiveEdit(this.view);
    this.flushPendingMarkdownSync();
  }

  unmount(): void {
    this.assertActive();
    this.commitPendingEdits();
    this.clearPlainTextPasteRequest();
    if (!this.view) return;
    this.clearBlockAlignmentGaps();
    this.suspendedState = this.semanticViewDirty ? null : this.view.state;
    this.view.destroy();
    this.view = null;
    this.target = null;
    this.syncSnapshot = null;
  }

  destroy(): void {
    if (this.destroyed) return;
    this.unmount();
    this.clearMarkdownSyncTimer();
    this.clearPlainTextPasteRequest();
    this.listeners.clear();
    this.view?.destroy();
    this.view = null;
    this.target = null;
    this.suspendedState = null;
    this.destroyed = true;
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
      ? parseMarkdown(this.markdown)
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
      nodes[name] = (node) =>
        (snapshots.get(node)?.cloneNode(true) as HTMLElement | undefined) ?? fallback(node);
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
    container.append(serializer.serializeFragment(removeEmptyTrailingParagraph(doc).content));
    return container.innerHTML;
  }

  getScrollSyncSnapshot(): EditorSyncSnapshot {
    if (!this.view || this.semanticViewDirty || this.pendingMarkdownDoc) {
      return { revision: this.contentRevision, renderRevision: this.syncRenderRevision,
        ready: false, markdown: this.markdown, anchors: [] };
    }
    if (this.syncSnapshot?.revision === this.contentRevision) return this.syncSnapshot;
    const parsed = parseMarkdownWithSyncAnchors(this.markdown);
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
    const delaySemanticSync = options?.sourceInput === true;
    const previousDoc = delaySemanticSync
      ? null
      : (this.view?.state.doc ?? this.parseSemanticDocument(this.markdown).doc);
    this.markdown = updateTocBlocks(markdown);
    this.contentRevision += 1;
    this.syncSnapshot = null;
    this.version += 1;

    const savedMarkdown =
      options?.savedMarkdown === undefined ? undefined : updateTocBlocks(options.savedMarkdown);
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
    if (delaySemanticSync) {
      this.semanticViewDirty = true;
      this.suspendedState = null;
      if (shouldReportImageDeletion(options)) {
        this.notifyDeletedImageSrcs(
          findFullyRemovedMarkdownImageSrcs(previousMarkdown, this.markdown),
        );
      }
      this.emit(options?.reason ?? 'programmatic-update');
      return;
    }

    this.replaceViewState(this.markdown);
    if (previousDoc && shouldReportImageDeletion(options)) {
      const nextDoc = this.view?.state.doc ?? this.parseSemanticDocument(this.markdown).doc;
      this.notifyDeletedImages(previousDoc, nextDoc);
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
      meta: {
        mode: this.runtime.mode,
      },
    };
  }

  restoreSnapshot(snapshot: EditorSnapshot): void {
    this.assertActive();
    this.clearPendingMarkdownSync();
    this.markdown = updateTocBlocks(snapshot.markdown);
    this.originalMarkdown = this.markdown;
    this.originalDoc = this.parseSemanticDocument(this.markdown).doc;
    this.version = snapshot.version;
    this.dirty = true;
    this.replaceViewState(this.markdown, snapshot.selection);
    this.emit('restore-snapshot');
  }

  restoreSelectionSnapshot(selection: { anchor: number; head: number }): void {
    this.assertActive();
    const state = this.view?.state ?? this.suspendedState ?? this.createState(this.markdown);
    const nextState = this.restoreSelection(state, selection);
    if (this.view) this.view.dispatch(state.tr.setSelection(nextState.selection));
    else this.suspendedState = nextState;
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
      return serializeMarkdownSelection(clipboardDoc, selection.from, selectionTo) ?? plainText;
    } catch {
      return plainText;
    }
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
    this.assertActive();
    if (!this.view || !this.isEditable()) {
      return { status: 'rejected', reason: 'readonly' };
    }

    const text = input.text ?? '';
    if (options.mode === 'plain') {
      return text ? this.insertPlainClipboardText(text) : { status: 'rejected', reason: 'no-text' };
    }

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

    if (input.html) {
      const event =
        typeof ClipboardEvent === 'undefined'
          ? (new Event('paste') as ClipboardEvent)
          : new ClipboardEvent('paste');
      return this.view.pasteHTML(input.html, event)
        ? { status: 'inserted', format: 'html' }
        : { status: 'rejected', reason: 'no-text' };
    }

    return text ? this.insertPlainClipboardText(text) : { status: 'rejected', reason: 'no-text' };
  }

  private handleNativePaste(event: ClipboardEvent): boolean {
    const clipboard = event.clipboardData;
    if (!clipboard) return false;

    const plain = this.consumePlainTextPasteRequest();
    if (!plain && clipboard.files.length > 0) {
      return false;
    }

    const text = clipboard.getData('text/plain');
    const html = clipboard.getData('text/html');
    if (!plain) {
      if (!text || classifyClipboardMarkdown(text).kind === 'plain') {
        return false;
      }
    }

    const result = this.pasteClipboard({ text, html }, { mode: plain ? 'plain' : 'auto' });
    event.preventDefault();
    return result.status === 'inserted' || result.reason === 'no-text';
  }

  private insertPlainClipboardText(text: string): EditorPasteResult {
    if (!this.view) return { status: 'rejected', reason: 'readonly' };
    const { state } = this.view;
    const slice = createPlainTextSlice(text, state.selection.$from);
    const transaction = state.tr
      .replaceSelection(slice)
      .setMeta(plainTextPasteMeta, true)
      .scrollIntoView();
    this.view.dispatch(transaction);
    return { status: 'inserted', format: 'plain' };
  }

  private insertMarkdownClipboard(classification: ClipboardMarkdownClassification): boolean {
    if (!this.view || classification.kind !== 'markdown') return false;
    const { state } = this.view;
    if (!this.canInsertMarkdownClipboard(classification.doc)) return false;

    try {
      const transaction = state.tr
        .replaceSelection(createMarkdownClipboardSlice(classification.doc))
        .scrollIntoView();
      if (!transaction.docChanged) return false;
      this.view.dispatch(transaction);
      return true;
    } catch {
      return false;
    }
  }

  private canInsertMarkdownClipboard(doc: ProseMirrorNode): boolean {
    if (!this.view) return false;
    const { selection } = this.view.state;
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
      for (let index = 0; index < doc.childCount; index += 1) {
        if (doc.child(index).type !== schema.nodes.paragraph) return false;
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
    this.assertActive();
    this.flushPendingMarkdownSync();

    if (!this.canExecute(command)) {
      return false;
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

    return this.runProseMirrorCommand(command);
  }

  canExecute(command: EditorCommand): boolean {
    if (this.destroyed || this.runtime.readonly) {
      return command.type === 'undo' || command.type === 'redo';
    }

    if (this.runtime.mode !== 'semantic' && isPendingInlineMarkCommand(command)) {
      return false;
    }

    return true;
  }

  /** 判断指定行内格式是否处于 pending 状态（collapsed selection 下的待定标记） */
  isPendingMarkActive(markName: InlinePendingMarkName): boolean {
    if (!this.view) return false;
    const markType = this.view.state.schema.marks[markName];
    if (!markType) return false;
    return isPendingMarkActive(this.view.state, markType);
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

  updateOptions(options: Partial<EditorRuntimeOptions>): void {
    this.assertActive();
    if (options.mode && options.mode !== this.runtime.mode) {
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
    this.view?.setProps({
      editable: () => this.isEditable(),
    });
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
    };
  }

  private createState(markdown: string): EditorState {
    const { doc, appended } = this.parseSemanticDocument(markdown);
    return EditorState.create({
      doc,
      selection: appended ? TextSelection.create(doc, doc.content.size - 1) : undefined,
      plugins: [
        windowsImePunctuationFallbackPlugin(),
        blockAlignmentPlugin(),
        inputRules({
          rules: createMarkdownInputRules(),
        }),
        blockquoteInputPlugin(),
        history(),
        taskListPlugin(),
        mathInlineInputPlugin(() => this.view),
        inlineMarkdownMarkInputPlugin(),
        linkInteractionPlugin({ openLink: this.options.onOpenLink }),
        codeHighlightPlugin(),
        codeHighlightDecorationPlugin({ enabled: false }),
        inlineCodeSelectionBridgePlugin(),
        headingLevelIndicatorPlugin(),
        searchHighlightPlugin(),
        // mathBlockPlugin(),  // 已被 math_block 语义节点 + displayMathInputPlugin 取代
        displayMathInputPlugin(),
        trailingParagraphPlugin(),
        tocSyncPlugin(),
        pendingInlineMarkPlugin(),
        tableHtmlBlockPlugin(),
        tableControlsPlugin(),
        tableEditing({ allowTableNodeSelection: true }),
        createCalloutPlugin(),
        keymap({
          'Mod-z': undo,
          'Mod-y': redo,
          'Shift-Mod-z': redo,
          'Mod-b': toggleMarkPending(schema.marks.strong),
          'Mod-i': toggleMarkPending(schema.marks.em),
          'Ctrl-`': toggleMarkPending(schema.marks.code),
          'Mod-k': (_state, _dispatch, _view) => {
            this.options.onLinkShortcut?.();
            return Boolean(this.options.onLinkShortcut);
          },
          'Alt-Shift-5': toggleMarkPending(schema.marks.strikethrough),
          'Mod-u': toggleMarkPending(schema.marks.underline),
          'Mod-\\': (_state, _dispatch, _view) =>
            this.runProseMirrorCommand({ type: 'clearInlineStyles' }),
          'Ctrl-\\': (_state, _dispatch, _view) =>
            this.runProseMirrorCommand({ type: 'clearInlineStyles' }),
          'Ctrl-Enter': (state, dispatch) => {
            // 在表格内：下方插入新行
            const context = findTableContext(state);
            if (context) return addTableRowAfter()(state, dispatch);
            // 在 callout 内：跳出 callout，在下方插入段落
            const { $from: $fromCtrl } = state.selection;
            for (let d = $fromCtrl.depth; d >= 0; d--) {
              if ($fromCtrl.node(d).type.name === 'callout') {
                const calloutEnd = $fromCtrl.after(d + 1);
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
            const afterPos = $from.after(1);
            const emptyParagraph = schema.nodes.paragraph.create();
            const tr = state.tr.insert(afterPos, emptyParagraph);
            const newPos = afterPos + 1;
            if (dispatch) {
              dispatch(tr.setSelection(TextSelection.create(tr.doc, newPos)).scrollIntoView());
            }
            return true;
          },
          'Shift-Ctrl-Enter': (state, dispatch) => {
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
            insertSoftLineBreak,
            newlineInCode,
            splitTaskListItem(schema.nodes.list_item),
            splitListItem(schema.nodes.list_item),
            createParagraphNear,
            liftEmptyBlock,
            splitBlockExitHeading,
          ),
          Enter: chainCommands(
            // $$ 回车自动补全：段落内只有 $$ 时按回车，生成空 math_block 并自动进入编辑态
            (state, dispatch) => {
              const { $from, empty } = state.selection;
              if (!empty || $from.parent.type.name !== 'paragraph') return false;
              if ($from.parent.textContent !== '$$') return false;
              if (dispatch) {
                const blockStart = $from.before(1);
                const blockEnd = $from.after(1);
                const node = schema.nodes.math_block.create({ tex: '' });
                const tr = state.tr.replaceWith(blockStart, blockEnd, node);
                // 选中新创建的 math_block，触发 NodeView.selectNode → 自动进入编辑态
                tr.setSelection(NodeSelection.create(tr.doc, blockStart));
                dispatch(tr);
              }
              return true;
            },
            newlineInCode,
            splitTaskListItem(schema.nodes.list_item),
            splitListItem(schema.nodes.list_item),
            createParagraphNear,
            liftEmptyBlock,
            splitBlockExitHeading,
          ),
          Backspace: chainCommands(
            deleteSelection,
            deleteTaskListItemBeforeCursor,
            removeEmptyCalloutOnBackspace,
            deleteCodeBlockBeforeCursor,
            joinBackward,
            selectNodeBackward,
          ),
          Delete: chainCommands(deleteSelection, joinForward, selectNodeForward),
          Space: convertMarkdownListShortcut,
          Tab: chainCommands(
            convertMarkdownListShortcut,
            goToNextCell(1),
            sinkListItem(schema.nodes.list_item),
            insertTabInTextblock,
          ),
          'Shift-Tab': chainCommands(
            goToNextCell(-1),
            liftListItem(schema.nodes.list_item),
            removeTabBeforeCursorInTextblock,
          ),
          'Shift-Ctrl-[': (state, dispatch) =>
            toggleList(state, dispatch, schema.nodes.ordered_list),
          'Shift-Ctrl-]': (state, dispatch) =>
            toggleList(state, dispatch, schema.nodes.bullet_list),
          'Shift-Ctrl-x': (state, dispatch) => toggleTaskListAtCursor(state, dispatch),
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
        typographyPlugin(),
        contextMenuPlugin({
          onOpen: (event) => this.options.onContextMenuOpen?.(event),
        }),
      ],
    });
  }

  private parseSemanticDocument(markdown: string): TrailingParagraphNormalization {
    return ensureTrailingParagraph(parseMarkdown(markdown));
  }

  private dispatchTransaction(transaction: Transaction): void {
    const currentState = this.view?.state ?? this.suspendedState;
    if (!currentState) return;
    const previousDoc = currentState.doc;
    const previousSelection = currentState.selection;
    const nextState = currentState.apply(transaction);
    if (this.view) this.view.updateState(nextState);
    else this.suspendedState = nextState;

    if (isTypographyTransaction(transaction)) {
      this.syncRenderRevision += 1;
      this.syncSnapshot = null;
      return;
    }
    if (isSemanticBlockAlignmentTransaction(transaction)) {
      return;
    }

    if (transaction.docChanged) {
      this.contentRevision += 1;
      this.syncRenderRevision += 1;
      this.syncSnapshot = null;
      this.pendingMarkdownDoc = nextState.doc;
      this.dirty = !nextState.doc.eq(this.originalDoc);
      this.scheduleMarkdownSync();
      this.notifyDeletedImages(previousDoc, nextState.doc);
    }

    // 每次事务都递增版本并通知（pending mark 状态切换、选区变化等需要及时反映到 UI）
    this.version += 1;
    this.emit(transaction.docChanged ? 'content-pending' : 'transaction');
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
      serializeClipboardText(selection.content());

    return {
      selection: { anchor: selection.anchor, head: selection.head },
      selectedMarkdown,
      caret: this.getScrollSyncCaret() ?? undefined,
    };
  }

  setSavedMarkdownBaseline(markdown: string): void {
    this.assertActive();
    this.commitPendingEdits();
    this.originalMarkdown = updateTocBlocks(markdown);
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
      this.flushPendingMarkdownSync();
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
    this.semanticViewDirty = false;
  }

  private restoreSelection(
    state: EditorState,
    selection: { anchor: number; head: number },
  ): EditorState {
    const anchor = clampDocPosition(state.doc, selection.anchor);
    const head = clampDocPosition(state.doc, selection.head);

    try {
      return state.apply(state.tr.setSelection(TextSelection.create(state.doc, anchor, head)));
    } catch {
      const fallback = TextSelection.near(state.doc.resolve(head));
      return state.apply(state.tr.setSelection(fallback));
    }
  }

  private isEditable(): boolean {
    return !this.runtime.readonly && this.runtime.mode === 'semantic';
  }

  private refreshInitialEditableState(): void {
    const view = this.view;
    if (!view || !this.isEditable()) {
      return;
    }

    view.setProps({ editable: () => false });
    requestAnimationFrame(() => {
      if (this.destroyed || this.view !== view) {
        return;
      }
      view.setProps({ editable: () => this.isEditable() });
    });
  }

  private runProseMirrorCommand(command: EditorCommand): boolean {
    if (!this.view) {
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
    if (!this.options.onImagesDeleted) {
      return;
    }

    const deletedSrcs = findFullyRemovedImageSrcs(previousDoc, nextDoc);
    this.notifyDeletedImageSrcs(deletedSrcs);
  }

  private notifyDeletedImageSrcs(deletedSrcs: string[]): void {
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
    command.type === 'toggleHighlight'
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

function convertMarkdownListShortcut(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
): boolean {
  const { selection } = state;
  if (!selection.empty || !selection.$from.parent.isTextblock) {
    return false;
  }

  const { $from } = selection;
  if ($from.parentOffset !== $from.parent.content.size) {
    return false;
  }

  const shortcutText = $from.parent.textBetween(0, $from.parentOffset);
  const taskMatch = /^(\d+\.|[-+*])\s+\[([ xX])\]$/.exec(shortcutText);
  const orderedMatch = /^(\d+)\.$/.exec(shortcutText);
  const bulletMatch = /^[-+*]$/.exec(shortcutText);
  if (!taskMatch && !orderedMatch && !bulletMatch) {
    return false;
  }

  const listType =
    orderedMatch || taskMatch?.[1].endsWith('.')
      ? schema.nodes.ordered_list
      : schema.nodes.bullet_list;
  const attrs = orderedMatch
    ? { order: Number(orderedMatch[1]) }
    : taskMatch?.[1].endsWith('.')
      ? { order: Number.parseInt(taskMatch[1], 10) }
      : null;

  if (dispatch) {
    const tr = state.tr.delete($from.start(), $from.pos);
    if (!appendCommandTransaction(state, tr, wrapInList(listType, attrs))) {
      return false;
    }

    if (taskMatch) {
      const checked = taskMatch[2].toLowerCase() === 'x' ? 'x' : ' ';
      tr.insertText(`[${checked}] `, tr.selection.from);
    }
    dispatch(tr.scrollIntoView());
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
    if (!visibleText) {
      return false;
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
    const firstText = findFirstTextInNode(itemNode);
    if (!firstText?.node.text) {
      return null;
    }

    const markerMatch = /^\[[ x]\]\s?/.exec(firstText.node.text);
    if (!markerMatch) {
      return null;
    }

    const markerFrom = itemPos + 1 + firstText.offset;
    return {
      itemNode,
      itemPos,
      itemIndex: $from.index(depth - 1),
      parentListNode,
      parentListPos: $from.before(depth - 1),
      markerLength: markerMatch[0].length,
      markerRange: {
        from: markerFrom,
        to: markerFrom + markerMatch[0].length,
      },
    };
  }
  return null;
}

function findFirstTextInNode(
  root: ProseMirrorNode,
): { node: ProseMirrorNode; offset: number } | null {
  let result: { node: ProseMirrorNode; offset: number } | null = null;
  root.descendants((node, pos) => {
    if (node.isText) {
      result = { node, offset: pos };
      return false;
    }
    return true;
  });
  return result;
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
  if (!scrollContainer) {
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
  const next = countImageSrcs(nextDoc);
  const deleted: string[] = [];

  for (const [src, previousCount] of previous) {
    if (previousCount > 0 && !next.has(src)) {
      deleted.push(src);
    }
  }

  return deleted;
}

function findFullyRemovedMarkdownImageSrcs(
  previousMarkdown: string,
  nextMarkdown: string,
): string[] {
  const previous = countMarkdownImageSrcs(previousMarkdown);
  const next = countMarkdownImageSrcs(nextMarkdown);
  const deleted: string[] = [];

  for (const [src, previousCount] of previous) {
    if (previousCount > 0 && !next.has(src)) {
      deleted.push(src);
    }
  }

  return deleted;
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

function countMarkdownImageSrcs(markdown: string): Map<string, number> {
  const counts = new Map<string, number>();
  const imagePattern = /!\[[^\]]*]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
  let match: RegExpExecArray | null;

  while ((match = imagePattern.exec(markdown)) !== null) {
    const src = match[1].trim();
    if (src) {
      counts.set(src, (counts.get(src) ?? 0) + 1);
    }
  }

  return counts;
}
