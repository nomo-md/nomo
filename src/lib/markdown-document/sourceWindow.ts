import { EditorState, Compartment, Transaction } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { defaultKeymap } from '@codemirror/commands';
import type { MarkdownSessionController } from './controller';
import type { DocumentSelection, DocumentWindow, SourceEdit } from './protocol';
import { MarkdownWindowBridge } from './windowBridge';
import { utf16ToUtf8, utf8ToUtf16 } from './utf8Text';
import { sourceEmojiSuggestions } from '../emoji/sourceSuggestions';
import { getSourceTextChanges } from './textChanges';

let sourceWindowTheme: ReturnType<typeof EditorView.theme> | undefined;
function getSourceWindowTheme() {
  return (sourceWindowTheme ??= EditorView.theme({
    '&': { backgroundColor: 'transparent' },
    '.cm-scroller': { fontFamily: 'var(--md-editor-font-mono)', lineHeight: '1.75' },
    '.cm-content': { padding: '0' },
    '.cm-line': { padding: '0' },
    '&.cm-focused': { outline: 'none' },
  }));
}

/** CodeMirror 的逻辑换行与原文件的混合换行之间的精确窗口映射。 */
export class SourceWindowCoordinates {
  readonly text: string;
  private readonly offsets: number[] = [];
  constructor(readonly source: string) {
    const characters: string[] = [];
    for (let offset = 0; offset < source.length; offset++) {
      this.offsets.push(offset);
      if (source[offset] === '\r') {
        characters.push('\n');
        if (source[offset + 1] === '\n') offset++;
      } else characters.push(source[offset]);
    }
    this.offsets.push(source.length);
    this.text = characters.join('');
  }
  sourceOffset(position: number) {
    const offset = this.offsets[position];
    if (offset === undefined) throw new RangeError('源码窗口坐标越界。');
    return offset;
  }
  viewOffset(sourceOffset: number) {
    let low = 0;
    let high = this.offsets.length - 1;
    while (low <= high) {
      const mid = (low + high) >>> 1;
      if (this.offsets[mid] === sourceOffset) return mid;
      if (this.offsets[mid] < sourceOffset) low = mid + 1;
      else high = mid - 1;
    }
    throw new RangeError('选区不能落在换行符内部。');
  }
  newlineAt(position: number) {
    return (
      /\r\n|\r|\n/.exec(this.source.slice(this.sourceOffset(position)))?.[0] ??
      this.source.match(/\r\n|\r|\n/g)?.at(-1) ??
      '\n'
    );
  }
}

/** 原文模式和巨型结构回退共用此视图；没有独立的 CodeMirror 撤销字段。 */
export class MarkdownSourceWindow {
  readonly view: EditorView;
  readonly bridge: MarkdownWindowBridge;
  private coordinates: SourceWindowCoordinates;
  private readonly readonly = new Compartment();
  private detachEdits: () => void;
  private destroyed = false;
  private group = '';
  private lastEdit = 0;
  private composing = false;
  private applicationReadonly = false;
  private compositionWaiters = new Set<() => void>();
  constructor(
    readonly session: MarkdownSessionController,
    window: DocumentWindow,
    options: {
      target: HTMLElement;
      onStale(): void;
      onHistory(): void;
      onFailure(error: unknown): void;
      onSelectAll?(): boolean;
      onBoundary?(direction: -1 | 1, extend: boolean): boolean;
      emojiSuggestions?: boolean;
      readonly?: boolean;
    },
  ) {
    this.applicationReadonly = !!options.readonly;
    this.coordinates = new SourceWindowCoordinates(window.text);
    let view: EditorView | undefined;
    this.bridge = new MarkdownWindowBridge(session, window, {
      onStale: options.onStale,
      onError: options.onFailure,
      onDerivedSource: source => {
        if (!view || this.composing || this.bridge.pinned) return false;
        const coordinates = new SourceWindowCoordinates(source);
        const changes = getSourceTextChanges(this.coordinates.text, coordinates.text);
        // 已提交的派生补丁只映射当前局部选区，不经 dispatchTransactions 再次写入会话。
        view.update([view.state.update({ changes, annotations: Transaction.addToHistory.of(false) })]);
        this.coordinates = coordinates;
        return true;
      },
      onState: () =>
        queueMicrotask(() => {
          if (this.destroyed || !view) return;
          const blocked = this.applicationReadonly || this.bridge.isStale || !!session.state.failure || session.state.historyBusy || session.state.nativeCommitBusy || session.state.closing;
          if (view.state.readOnly !== blocked)
            view.dispatch({
              effects: this.readonly.reconfigure([
                EditorState.readOnly.of(blocked),
                EditorView.editable.of(!blocked),
              ]),
            });
        }),
    });
    const history = (redo: boolean) => {
      this.group = '';
      void session.undo(redo).then(options.onHistory, options.onFailure);
      return true;
    };
    const boundary = (direction: -1 | 1, extend: boolean) => {
      if (
        !view ||
        (direction < 0
          ? view.state.selection.main.head > 0
          : view.state.selection.main.head < view.state.doc.length)
      )
        return false;
      return options.onBoundary?.(direction, extend) ?? false;
    };
    this.view = view = new EditorView({
      parent: options.target,
      state: EditorState.create({
        doc: this.coordinates.text,
        extensions: [
          this.readonly.of([EditorState.readOnly.of(false), EditorView.editable.of(true)]),
          keymap.of([
            { key: 'Mod-z', run: () => history(false) },
            { key: 'Mod-y', run: () => history(true) },
            { key: 'Mod-Shift-z', run: () => history(true) },
            { key: 'Mod-a', run: () => options.onSelectAll?.() ?? false },
            { key: 'ArrowUp', run: () => boundary(-1, false), shift: () => boundary(-1, true) },
            { key: 'ArrowDown', run: () => boundary(1, false), shift: () => boundary(1, true) },
            { key: 'ArrowLeft', run: () => boundary(-1, false), shift: () => boundary(-1, true) },
            { key: 'ArrowRight', run: () => boundary(1, false), shift: () => boundary(1, true) },
            ...defaultKeymap,
          ]),
          EditorView.lineWrapping,
          ...(options.emojiSuggestions ? [sourceEmojiSuggestions()] : []),
          getSourceWindowTheme(),
          EditorView.domEventHandlers({
            compositionstart: () => {
              this.composing = true;
              this.group = globalThis.crypto.randomUUID();
            },
            compositionend: () => {
              setTimeout(() => {
                this.composing = false;
                this.group = '';
                for (const resolve of this.compositionWaiters) resolve();
                this.compositionWaiters.clear();
              }, 0);
            },
          }),
        ],
      }),
      dispatchTransactions: (transactions, current) => {
        try {
          for (const transaction of transactions)
            if (transaction.docChanged) this.submit(transaction, this.bridge.window.fromByte);
          current.update(transactions);
        } catch (error) {
          options.onFailure(error);
        }
      },
    });
    this.detachEdits = session.attachEdits({
      hasUnsettledEdits: () => this.composing || this.bridge.pinned,
      awaitCompositionEnd: () =>
        this.composing
          ? new Promise((resolve) => this.compositionWaiters.add(resolve))
          : Promise.resolve(),
      commitPendingEdits() {},
    });
  }
  private submit(transaction: Transaction, fromByte: number) {
    if (this.bridge.isStale || this.session.state.historyBusy || this.session.state.nativeCommitBusy || this.session.state.closing)
      throw new Error('源码窗口正在更新，请稍后输入。');
    const previous = this.coordinates;
    const patches: Array<{ from: number; to: number; insert: string }> = [];
    transaction.changes.iterChanges((from, to, _nextFrom, _nextTo, inserted) =>
      patches.push({
        from: previous.sourceOffset(from),
        to: previous.sourceOffset(to),
        insert: inserted.toString().replace(/\n/g, previous.newlineAt(from)),
      }),
    );
    const edits: SourceEdit[] = patches.map((patch) => ({
      fromByte: fromByte + utf16ToUtf8(previous.source, patch.from),
      toByte: fromByte + utf16ToUtf8(previous.source, patch.to),
      insertedText: patch.insert,
    }));
    let source = previous.source;
    for (const patch of [...patches].reverse())
      source = source.slice(0, patch.from) + patch.insert + source.slice(patch.to);
    const next = new SourceWindowCoordinates(source);
    const before = transaction.startState.selection.main;
    const after = transaction.state.selection.main;
    const now = performance.now();
    if (
      !this.group ||
      (!this.composing && (now - this.lastEdit > 500 || !transaction.isUserEvent('input.type')))
    )
      this.group = globalThis.crypto.randomUUID();
    this.lastEdit = now;
    this.bridge.commit(
      {
        source,
        edits,
        selectionBefore: {
          anchorByte: fromByte + utf16ToUtf8(previous.source, previous.sourceOffset(before.anchor)),
          headByte: fromByte + utf16ToUtf8(previous.source, previous.sourceOffset(before.head)),
        },
        selectionAfter: {
          anchorByte: fromByte + utf16ToUtf8(source, next.sourceOffset(after.anchor)),
          headByte: fromByte + utf16ToUtf8(source, next.sourceOffset(after.head)),
        },
      },
      this.group,
    );
    this.coordinates = next;
    if (!this.composing && !transaction.isUserEvent('input.type')) this.group = '';
  }
  get selection(): DocumentSelection {
    const selected = this.view.state.selection.main;
    const byte = (position: number) =>
      this.bridge.window.fromByte +
      utf16ToUtf8(this.coordinates.source, this.coordinates.sourceOffset(position));
    return { anchorByte: byte(selected.anchor), headByte: byte(selected.head) };
  }
  sourceByteAtCoords(left: number, top: number): number | undefined {
    const position = this.view.posAtCoords({ x: left, y: top });
    return position === null ? undefined : this.bridge.window.fromByte + utf16ToUtf8(this.coordinates.source, this.coordinates.sourceOffset(position));
  }
  sourceRangeRects(fromByte: number, toByte: number): DOMRect[] {
    const position = (byte: number) => this.coordinates.viewOffset(utf8ToUtf16(this.coordinates.source, byte - this.bridge.window.fromByte));
    const start = this.view.domAtPos(position(fromByte)); const end = this.view.domAtPos(position(toByte));
    const range = document.createRange(); range.setStart(start.node, start.offset); range.setEnd(end.node, end.offset);
    return Array.from(range.getClientRects());
  }
  byteTop(byte: number): number {
    const position = this.coordinates.viewOffset(
      utf8ToUtf16(this.coordinates.source, byte - this.bridge.window.fromByte),
    );
    return this.view.documentTop + this.view.lineBlockAt(position).top;
  }
  restoreSelection(selection: DocumentSelection) {
    const position = (byte: number) =>
      this.coordinates.viewOffset(
        utf8ToUtf16(this.coordinates.source, byte - this.bridge.window.fromByte),
      );
    this.view.dispatch({
      selection: { anchor: position(selection.anchorByte), head: position(selection.headByte) },
    });
  }
  get pinned() {
    return this.composing || this.bridge.pinned;
  }
  setReadonly(readonly: boolean) {
    this.applicationReadonly = readonly;
    const state = this.session.state;
    const blocked = readonly || this.bridge.isStale || !!state.failure || state.historyBusy || state.nativeCommitBusy || state.closing;
    if (!this.destroyed && this.view.state.readOnly !== blocked)
      this.view.dispatch({ effects: this.readonly.reconfigure([EditorState.readOnly.of(blocked), EditorView.editable.of(!blocked)]) });
  }
  dispose() {
    if (this.composing || this.bridge.pinned) throw new Error('正在编辑的源码区域不能回收。');
    this.destroyed = true;
    this.detachEdits();
    this.bridge.dispose();
    this.view.destroy();
  }
}
