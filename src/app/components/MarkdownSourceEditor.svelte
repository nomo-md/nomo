<script lang="ts">
  import { defaultKeymap, history, historyKeymap, redo, undo } from '@codemirror/commands';
  import {
    Compartment,
    EditorState,
    StateEffect,
    StateField,
    Transaction,
  } from '@codemirror/state';
  import {
    BlockType,
    Decoration,
    EditorView,
    WidgetType,
    keymap,
    type DecorationSet,
  } from '@codemirror/view';
  import { onDestroy, onMount } from 'svelte';
  import type { BlockAlignmentAnchor } from '../services/markdownBlockAlignment';
  import { getSourceTextChanges, type MarkdownSourceEditorHandle, type MarkdownSourceRuntimeState, type MarkdownSourceSelectionSnapshot } from './markdownSourceEditor';

  export let markdown: string;
  export let documentId = '';
  export let runtimeState: MarkdownSourceRuntimeState | undefined = undefined;
  export let onRuntimeStateChange: (state: MarkdownSourceRuntimeState) => void = () => undefined;
  export let readonlyDocumentMode = false;
  export let onMarkdownChange: (markdown: string) => void = () => undefined;
  export let onSelectionChange: ((selectedMarkdown: string) => void) | undefined = undefined;
  export let onSelectionSnapshotChange: (range: MarkdownSourceSelectionSnapshot) => void = () => undefined;
  let contentRevision = 0;
  export let onPaste: (event: ClipboardEvent) => void = () => undefined;
  export let onDrop: (event: DragEvent) => void = () => undefined;
  export let onScroll: () => void = () => undefined;
  export let onReady: (handle: MarkdownSourceEditorHandle) => void = () => undefined;
  export let onLayoutChange: () => void = () => undefined;
  export let sourceEditor: MarkdownSourceEditorHandle;

  interface SourceSpacerSpec {
    key: string;
    position: number;
    height: number;
  }

  class SourceSpacerWidget extends WidgetType {
    readonly key: string;
    readonly height: number;

    constructor(key: string, height: number) {
      super();
      this.key = key;
      this.height = height;
    }

    eq(other: SourceSpacerWidget) {
      return other.key === this.key && Math.abs(other.height - this.height) <= 0.01;
    }

    get estimatedHeight() {
      return this.height;
    }

    toDOM() {
      const spacer = document.createElement('div');
      spacer.className = 'source-block-alignment-spacer';
      spacer.dataset.alignmentKey = this.key;
      spacer.style.height = `${this.height}px`;
      spacer.setAttribute('aria-hidden', 'true');
      return spacer;
    }
  }

  const setSourceSpacers = StateEffect.define<readonly SourceSpacerSpec[]>();
  const sourceSpacerField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(value, transaction) {
      let next = value.map(transaction.changes);
      for (const effect of transaction.effects) {
        if (!effect.is(setSourceSpacers)) continue;
        next = Decoration.set(
          effect.value.map((spec) =>
            Decoration.widget({
              widget: new SourceSpacerWidget(spec.key, spec.height),
              block: true,
              side: -1,
            }).range(spec.position),
          ),
          true,
        );
      }
      return next;
    },
    provide: (field) => EditorView.decorations.from(field),
  });

  let host: HTMLDivElement;
  let view: EditorView | null = null;
  let cachedSourceMarkdown = markdown;
  let layoutRevision = 0;
  let measuredLayoutRevision = -1;
  let externalDispatchDepth = 0;
  let mountedDocumentId = documentId;
  let currentGaps = new Map<string, number>();
  const readonlyCompartment = new Compartment();
  const historyCompartment = new Compartment();

  function createEditorExtensions() {
    return [
          historyCompartment.of(history()),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          EditorView.lineWrapping,
          sourceSpacerField,
          readonlyCompartment.of(EditorView.editable.of(!readonlyDocumentMode)),
          EditorView.contentAttributes.of({ spellcheck: 'false', autocapitalize: 'off' }),
          EditorView.domEventHandlers({
            paste: (event) => {
              onPaste(event);
              return event.defaultPrevented;
            },
            drop: (event) => {
              onDrop(event);
              return event.defaultPrevented;
            },
            scroll: () => {
              onScroll();
              return false;
            },
          }),
          EditorView.updateListener.of((update) => {
            if (update.geometryChanged || update.viewportChanged) onLayoutChange();
            if (update.docChanged) {
              contentRevision++;
              cachedSourceMarkdown = update.state.doc.toString();
              measureChangedContent(update.view);
            }
            if (update.docChanged && externalDispatchDepth === 0) {
              onMarkdownChange(cachedSourceMarkdown);
            }
            if (update.docChanged || update.selectionSet) {
              notifySelectionChange(update.state);
            }
          }),
          EditorView.theme({
            '&': { height: '100%', backgroundColor: 'transparent' },
            '&.cm-focused': { outline: 'none' },
            '.cm-scroller': {
              overflow: 'auto',
              fontFamily: 'var(--md-editor-font-mono)',
              fontSize: '14px',
              lineHeight: '1.75',
            },
            '.cm-content': {
              minHeight: 'var(--md-editor-content-min-height, 0px)',
              padding: '0',
              caretColor: 'var(--md-editor-fg)',
            },
            '.cm-line': { padding: '0' },
            '.cm-cursor': { borderLeftColor: 'var(--md-editor-fg)' },
            '.cm-selectionBackground': {
              backgroundColor: 'color-mix(in srgb, var(--md-editor-accent) 22%, transparent)',
            },
          }),
    ];
  }

  function createDocumentState(saved: MarkdownSourceRuntimeState | undefined) {
    if (saved?.documentId === documentId) {
      // 复用历史字段，重新配置组件回调，避免挂回后回调旧文档实例。
      // 文档隐藏期间可能完成图片导入或语义编辑；映射新正文仍保留源码历史。
      return saved.state.update({
        changes: getSourceTextChanges(saved.state.doc.toString(), saved.state.toText(markdown).toString()),
        effects: StateEffect.reconfigure.of(createEditorExtensions()),
        annotations: Transaction.addToHistory.of(false),
      }).state;
    }
    return EditorState.create({ doc: markdown, extensions: createEditorExtensions() });
  }

  function restoredContentRevision(saved: MarkdownSourceRuntimeState | undefined) {
    if (saved?.documentId !== documentId) return 0;
    return saved.contentRevision + (saved.state.doc.toString() === saved.state.toText(markdown).toString() ? 0 : 1);
  }

  function restoreScroll(saved: MarkdownSourceRuntimeState | undefined) {
    const editorView = view;
    if (!editorView || saved?.documentId !== documentId) return;
    void requestEditorMeasure(editorView).then(() => {
      if (view !== editorView) return;
      editorView.scrollDOM.scrollTop = saved.scrollTop;
      editorView.scrollDOM.scrollLeft = saved.scrollLeft;
    });
  }

  onMount(() => {
    view = new EditorView({ parent: host, state: createDocumentState(runtimeState) });
    contentRevision = restoredContentRevision(runtimeState);
    cachedSourceMarkdown = view.state.doc.toString();
    measureChangedContent(view);
    sourceEditor = createHandle();
    onReady(sourceEditor);
    notifySelectionChange(view.state);
    restoreScroll(runtimeState);
  });

  onDestroy(() => {
    if (view) onRuntimeStateChange(captureRuntimeState());
    currentGaps.clear();
    view?.destroy();
    view = null;
  });

  $: if (view && documentId !== mountedDocumentId) {
    onRuntimeStateChange(captureRuntimeState());
    externalDispatchDepth += 1;
    try {
      view.setState(createDocumentState(runtimeState));
      cachedSourceMarkdown = view.state.doc.toString();
      contentRevision = restoredContentRevision(runtimeState);
      currentGaps.clear();
      mountedDocumentId = documentId;
      measureChangedContent(view);
      notifySelectionChange(view.state);
      restoreScroll(runtimeState);
    } finally {
      externalDispatchDepth -= 1;
    }
  }

  $: if (view) {
    sourceEditor.setMarkdown(markdown, { addToHistory: false });
  }

  $: if (view) {
    view.dispatch({
      effects: readonlyCompartment.reconfigure(EditorView.editable.of(!readonlyDocumentMode)),
    });
  }

  function createHandle(): MarkdownSourceEditorHandle {
    return {
      getRuntimeState: captureRuntimeState,
      getSelectionStatsSnapshot: () => selectionSnapshot(getView().state),
      restoreRuntimeState(saved) {
        if (saved.documentId !== documentId) return;
        const editorView = getView();
        editorView.setState(createDocumentState(saved));
        cachedSourceMarkdown = editorView.state.doc.toString();
        contentRevision = restoredContentRevision(saved);
        currentGaps.clear();
        measureChangedContent(editorView);
        notifySelectionChange(editorView.state);
        restoreScroll(saved);
      },
      // 滚动同步每帧读取快照，不重复拼接整个 CodeMirror 文档。
      getMarkdown: () => {
        getView();
        return cachedSourceMarkdown;
      },
      setMarkdown(nextMarkdown, options = {}) {
        const editorView = getView();
        // 由 CodeMirror 按自己的逻辑行规则规范化，仅作用于编辑器副本。
        const next = editorView.state.toText(nextMarkdown).toString();
        const changes = getSourceTextChanges(cachedSourceMarkdown, next);
        if (!changes.length) return;
        const suppressChange = !(options.addToHistory ?? false);
        if (suppressChange) externalDispatchDepth += 1;
        try {
          editorView.dispatch({
            changes,
            annotations: Transaction.addToHistory.of(options.addToHistory ?? false),
          });
        } finally {
          if (suppressChange) externalDispatchDepth -= 1;
        }
      },
      getSelection() {
        const selection = getView().state.selection.main;
        return { from: selection.from, to: selection.to, anchor: selection.anchor, head: selection.head };
      },
      setSelection(from, to = from) {
        const editorView = getView();
        const length = editorView.state.doc.length;
        editorView.dispatch({
          selection: {
            anchor: clamp(from, 0, length),
            head: clamp(to, 0, length),
          },
        });
      },
      getSelectedMarkdown() {
        const editorView = getView();
        const selection = editorView.state.selection.main;
        return editorView.state.sliceDoc(selection.from, selection.to);
      },
      focus(options) {
        getView().contentDOM.focus(options);
      },
      revealRange(from, to = from) {
        const editorView = getView();
        const length = editorView.state.doc.length;
        const anchor = clamp(from, 0, length);
        const head = clamp(to, 0, length);
        editorView.dispatch({
          selection: { anchor, head },
          effects: EditorView.scrollIntoView(head, { y: 'center' }),
        });
      },
      undo: () => undo(getView()),
      redo: () => redo(getView()),
      lineAtOffset(offset) {
        const editorView = getView();
        return editorView.state.doc.lineAt(clamp(offset, 0, editorView.state.doc.length)).number;
      },
      offsetAtLine(lineNumber) {
        const editorView = getView();
        return editorView.state.doc.line(clamp(lineNumber, 1, editorView.state.doc.lines)).from;
      },
      getLineCount: () => getView().state.doc.lines,
      getLineTop(lineNumber) {
        const editorView = getView();
        return (
          (editorView.documentPadding.top + getLineTextTop(editorView, lineNumber)) /
          getViewScale(editorView)
        );
      },
      lineAtHeight(height) {
        const editorView = getView();
        const block = editorView.lineBlockAtHeight(
          Math.max(0, height * getViewScale(editorView) - editorView.documentPadding.top),
        );
        return editorView.state.doc.lineAt(block.from).number;
      },
      getLineHeight() {
        const editorView = getView();
        return editorView.defaultLineHeight / getViewScale(editorView);
      },
      isComposing: () => getView().composing,
      isLayoutReady: () => view !== null && measuredLayoutRevision === layoutRevision,
      getSyncCaretTop() {
        const editorView = getView();
        const coordinates = editorView.coordsAtPos(editorView.state.selection.main.head);
        if (!coordinates) {
          const line = editorView.state.doc.lineAt(editorView.state.selection.main.head).number;
          return (editorView.documentPadding.top + getLineTextTop(editorView, line)) / getViewScale(editorView);
        }
        const rect = editorView.scrollDOM.getBoundingClientRect();
        const scale = rect.height / editorView.scrollDOM.clientHeight || 1;
        return (coordinates.top - rect.top) / scale + editorView.scrollDOM.scrollTop;
      },
      getScrollElement: () => getView().scrollDOM,
      getContentElement: () => getView().contentDOM,
      // CodeMirror 的 contentHeight 包含 cm-content 的 padding。双栏尾部的
      // scroll-past-end 正是通过 padding-bottom 实现，不能反向算进正文自然高度。
      getContentHeight: () => getNaturalContentHeight(getView()) / getViewScale(getView()),
      getBlockGeometry(anchors) {
        const editorView = getView();
        const scale = getViewScale(editorView);
        const naturalContentHeight = getNaturalContentHeight(editorView);
        const documentTop = editorView.documentPadding.top;
        return anchors.map((anchor, index) => {
          const anchorTop = getAnchorTop(editorView, anchor);
          const top = (documentTop + anchorTop) * scale;
          const next = anchors[index + 1];
          const nextTop = next
            ? (documentTop + getAnchorTop(editorView, next)) * scale
            : (documentTop + naturalContentHeight) * scale;
          if (import.meta.env.DEV && index < 4) {
            const line = editorView.state.doc.line(
              clamp(anchor.fromLine, 1, editorView.state.doc.lines),
            );
            const block = editorView.lineBlockAt(line.from);
            const coordinates = editorView.coordsAtPos(line.from);
            const scrollRect = editorView.scrollDOM.getBoundingClientRect();
            console.info(
              '[split-source-anchor-json]',
              JSON.stringify({
                key: anchor.key,
                fromLine: anchor.fromLine,
                lineText: line.text,
                documentTop,
                anchorTop,
                measuredTop: top,
                coordinateTop: coordinates?.top ?? null,
                coordinateInScroll:
                  coordinates == null
                    ? null
                    : coordinates.top - scrollRect.top + editorView.scrollDOM.scrollTop,
                scale,
                block: {
                  from: block.from,
                  to: block.to,
                  top: block.top,
                  bottom: block.bottom,
                  children: Array.isArray(block.type)
                    ? block.type.map((child) => ({
                        type: child.type,
                        from: child.from,
                        to: child.to,
                        top: child.top,
                        bottom: child.bottom,
                      }))
                    : null,
                },
              }),
            );
          }
          return {
            key: anchor.key,
            top,
            nextTop: Math.max(top, nextTop),
            existingGap: currentGaps.get(anchor.key) ?? 0,
          };
        });
      },
      applyBlockGaps(anchors, gaps, leadingGap = 0) {
        const editorView = getView();
        const scale = getViewScale(editorView);
        const specs: SourceSpacerSpec[] = [];
        currentGaps = new Map();
        // 首块补偿也必须进入 CodeMirror 的高度树。动态修改 cm-content 的
        // padding-top 不会可靠地刷新虚拟布局，滚到文档中后会累积成整节错位。
        editorView.dom.style.removeProperty('--nomo-block-alignment-leading-gap');
        if (leadingGap > 0.05) {
          specs.push({
            key: '__source-leading__',
            position: 0,
            height: leadingGap / scale,
          });
        }
        for (const [index, anchor] of anchors.entries()) {
          // 最后一块之后没有需要对齐的块起点；尾部高度由工作区总高度协调器负责。
          // 在文档末尾插 spacer 会把虚拟尾部重新污染为 Markdown 块高度。
          if (index === anchors.length - 1) continue;
          const height = gaps.get(anchor.key) ?? 0;
          if (height <= 0.05) continue;
          currentGaps.set(anchor.key, height);
          specs.push({
            key: anchor.key,
            position: getSpacerPosition(editorView.state, anchor.toLine),
            height: height / scale,
          });
        }
        if (import.meta.env.DEV) {
          console.info('[split-source-gap-apply]', {
            scale,
            gaps: specs.map((spec) => ({
              key: spec.key,
              localHeight: spec.height,
              physicalHeight:
                spec.key === '__source-leading__' ? leadingGap : (gaps.get(spec.key) ?? 0),
            })),
          });
        }
        editorView.dispatch({ effects: setSourceSpacers.of(specs) });
        editorView.requestMeasure();
        if (import.meta.env.DEV) {
          requestAnimationFrame(() => {
            const spacerMeasurements = [
              ...editorView.dom.querySelectorAll<HTMLElement>(
                '.source-block-alignment-spacer',
              ),
            ].map((spacer) => ({
              key: spacer.dataset.alignmentKey,
              styleHeight: spacer.style.height,
              rectHeight: spacer.getBoundingClientRect().height,
            }));
            console.info(
              '[split-source-gap-dom-json]',
              JSON.stringify({
                scale: getViewScale(editorView),
                spacers: spacerMeasurements,
              }),
            );
            console.info('[split-source-gap-dom]', {
              scale: getViewScale(editorView),
              spacers: spacerMeasurements,
            });
          });
        }
      },
      clearBlockGaps() {
        currentGaps.clear();
        const editorView = view;
        if (!editorView) return;
        editorView.dom.style.removeProperty('--nomo-block-alignment-leading-gap');
        editorView.dispatch({ effects: setSourceSpacers.of([]) });
        editorView.requestMeasure();
      },
      requestMeasure: () => requestEditorMeasure(view),
    };
  }

  function captureRuntimeState(): MarkdownSourceRuntimeState {
    const editorView = getView();
    return {
      documentId: mountedDocumentId,
      state: editorView.state,
      scrollTop: editorView.scrollDOM.scrollTop,
      scrollLeft: editorView.scrollDOM.scrollLeft,
      contentRevision,
    };
  }

  function getView() {
    if (!view) throw new Error('Markdown source editor is not mounted.');
    return view;
  }

  function notifySelectionChange(state: EditorState) {
    const selection = state.selection.main;
    onSelectionSnapshotChange(selectionSnapshot(state));
    if (onSelectionChange) onSelectionChange(state.sliceDoc(selection.from, selection.to));
  }

  function selectionSnapshot(state: EditorState): MarkdownSourceSelectionSnapshot {
    const { from, to } = state.selection.main;
    return { documentId: mountedDocumentId, from, to, contentRevision };
  }

  function getAnchorTop(editorView: EditorView, anchor: BlockAlignmentAnchor) {
    if (anchor.key === 'eof') return getNaturalContentHeight(editorView);
    return getLineTextTop(editorView, anchor.fromLine);
  }

  function getLineTextTop(editorView: EditorView, lineNumber: number) {
    const line = editorView.state.doc.line(
      clamp(lineNumber, 1, editorView.state.doc.lines),
    );
    const lineBlock = editorView.lineBlockAt(line.from);
    if (!Array.isArray(lineBlock.type)) return lineBlock.top;

    // 行首 block widget 会让 lineBlock.top 指向 spacer 顶部。锚点代表的是
    // Markdown 行文字起点，必须从复合逻辑行中选择 widget 之后的 Text 子块。
    const textBlock = lineBlock.type.find(
      (block) =>
        block.type === BlockType.Text && block.from <= line.from && block.to >= line.from,
    );
    return textBlock?.top ?? lineBlock.top;
  }

  function getNaturalContentHeight(editorView: EditorView) {
    const { top, bottom } = editorView.documentPadding;
    return Math.max(0, editorView.contentHeight - top - bottom);
  }

  function getViewScale(editorView: EditorView) {
    return Number.isFinite(editorView.scaleY) && editorView.scaleY > 0 ? editorView.scaleY : 1;
  }

  function requestEditorMeasure(editorView: EditorView | null) {
    if (!editorView) return Promise.resolve();
    return new Promise<void>((resolve) => {
      editorView.requestMeasure({
        read: () => undefined,
        write: () => resolve(),
      });
    });
  }

  function measureChangedContent(editorView: EditorView) {
    const revision = ++layoutRevision;
    // Promise 回调在整个 measure（包括 CodeMirror 的滚动锚点调整）之后执行。
    void requestEditorMeasure(editorView).then(() => {
      if (view !== editorView || revision !== layoutRevision) return;
      measuredLayoutRevision = revision;
      onLayoutChange();
    });
  }

  function getSpacerPosition(state: EditorState, toLine: number) {
    const lineNumber = clamp(toLine, 1, state.doc.lines);
    return lineNumber < state.doc.lines ? state.doc.line(lineNumber + 1).from : state.doc.length;
  }

  function clamp(value: number, min: number, max: number) {
    return Math.min(max, Math.max(min, value));
  }
</script>

<div class="source-editor" bind:this={host}></div>
