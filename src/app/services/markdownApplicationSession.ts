import { createEditorCore, type EditorCore } from '../../lib/editor-core';
import type {
  EditorCoreOptions,
  EditorChangeEvent,
  EditorCommand,
} from '../../lib/editor-core/types';
import { createTocBlock } from '../../lib/toc/tocService';
import { extractFrontMatterBlock, type FrontMatterBlock } from '../../lib/markdown/frontMatter';
import {
  createMemoryMarkdownPort,
  createNativeMarkdownPort,
  readCompleteSnapshot,
} from '../../lib/markdown-document/port';
import { ProgressiveMarkdownDocument } from '../../lib/markdown-document/runtime';
import { MarkdownViewportHost, type MarkdownPresentation } from '../../lib/markdown-document/viewportHost';
import { utf8Length, utf16ToUtf8 } from '../../lib/markdown-document/utf8Text';
import { getSourceTextChanges } from '../../lib/markdown-document/textChanges';
import { DocumentConflictError } from '../../lib/markdown-document/protocol';
import type { DocumentSelection } from '../../lib/markdown-document/protocol';
import { mapMarkdownSelection } from '../../lib/markdown-document/positions';
import type { MarkdownSourceWindow } from '../../lib/markdown-document/sourceWindow';
import type { MarkdownTabState } from '../types';
import { writeEditorClipboard } from './clipboard';

const sessions = new WeakMap<EditorCore, MarkdownApplicationSession>();
const memoryPort = createMemoryMarkdownPort();
const nativePort = createNativeMarkdownPort();
export const getMarkdownApplicationSession = (core: EditorCore | undefined) =>
  core && sessions.get(core);

/** 应用侧只持有身份、元数据和按需视图；完整正文只通过异步快照读取。 */
export class MarkdownApplicationSession {
  readonly editor: EditorCore;
  readonly ready: Promise<ProgressiveMarkdownDocument>;
  document?: ProgressiveMarkdownDocument;
  frontMatter: FrontMatterBlock | null = null;
  private fallback: EditorCore;
  private active?: EditorCore;
  private source?: MarkdownSourceWindow;
  private semanticHost?: MarkdownViewportHost;
  private sourceHost?: MarkdownViewportHost;
  private listeners = new Set<(event: EditorChangeEvent) => void>();
  private disposed = false;
  private generation = 0;
  private unsubscribe?: () => void;
  private unsubscribeSelection?: () => void;
  private options: EditorCoreOptions;
  private sourceTarget?: HTMLElement;
  private semanticTarget?: HTMLElement;
  private viewMode: 'semantic' | 'source' | 'split';
  private teardown: Promise<void> = Promise.resolve();
  private scrollOrigin: 'semantic' | 'source' | undefined;
  private scrollUntil = 0;
  private frontMatterRevision = -1;
  private pendingFocus = false;
  private sourceFocus = false;
  private activeHostMode: 'semantic' | 'source' | undefined;
  private presentations = new Map<'semantic' | 'source', MarkdownPresentation>();
  private sourceRequest = 0;
  private modeGeneration = 0;
  private pendingSourceByte?: { byte: number; revision: number };
  private openings = new Map<HTMLElement, { timer: ReturnType<typeof setTimeout>; node?: HTMLElement }>();

  constructor(
    readonly tab: MarkdownTabState,
    private desktop: boolean,
    options: EditorCoreOptions,
    private onFailure: (error: unknown) => void,
    private onMetadata: () => void,
    private onSelection?: (selection: DocumentSelection) => void,
    private onRequestSource?: (isCurrent: () => boolean) => Promise<boolean>,
  ) {
    // 门面与窗口都不能携带旧的整篇正文/保存基线，否则每个局部 PM 又会解析一份全文。
    this.options = { ...options, markdown: '', savedMarkdown: undefined, dirty: false };
    this.viewMode = options.mode ?? 'semantic';
    this.fallback = createEditorCore(this.options);
    const overrides: Partial<EditorCore> = {
      mount: (target) => {
        this.semanticTarget = target;
        this.beginOpening(target);
        void this.mountViews().catch(error => this.failOpening(target, error));
      },
      unmount: () => {
        void this.unmount().catch(onFailure);
      },
      destroy: () => {
        void this.detach().catch(onFailure);
      },
      getMarkdown: () => {
        throw new Error('完整 Markdown 必须通过文档会话的异步快照读取。');
      },
      flushMarkdown: () => {
        throw new Error('局部窗口不能作为全文；请等待文档会话 flushEdits。');
      },
      getSnapshot: () => ({
        markdown: '',
        version: this.document?.state.status.revision ?? 0,
        contentRevision: this.document?.session.state.viewGeneration ?? 0,
        selection: this.active?.getSnapshot().selection,
        meta: { documentId: tab.documentId },
      }),
      subscribe: (listener) => {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
      },
      commitPendingEdits: () => {
        this.active?.commitPendingEdits();
      },
      awaitCompositionEnd: async () => {
        const doc = await this.ready;
        await doc.session.flushEdits();
      },
      setMarkdown: (text) => {
        if (!this.writable) return;
        void this.replaceMarkdown(text).catch(onFailure);
      },
      setDirty: () => {},
      setSavedMarkdownBaseline: () => {},
      refreshSemanticView: () => {},
      selectAll: () => {
        (this.source ? this.sourceHost : this.semanticHost)?.selectAll();
        return true;
      },
      focus: () => {
        if (this.currentHost) this.currentHost.focus();
        else this.queueFocus();
      },
      updateOptions: (next) => {
        this.options = { ...this.options, ...next };
        this.fallback.updateOptions(next);
        this.semanticHost?.updateOptions(next);
        this.sourceHost?.updateOptions(next);
        void this.mountViews().catch(onFailure);
      },
      updateTheme: (theme) => {
        this.options.theme = theme;
        this.fallback.updateTheme(theme);
        this.active?.updateTheme(theme);
        this.semanticHost?.updateTheme(theme);
      },
      execute: (command) => {
        if (command.type === 'undo' || command.type === 'redo') {
          if (!this.historyWritable) return false;
          void this.ready.then((doc) => doc.session.undo(command.type === 'redo')).catch(onFailure);
          return true;
        }
        if (!this.writable) return false;
        if (
          this.source ||
          this.currentHost?.hasGlobalSelection ||
          command.type === 'insertToc' ||
          (!this.active && command.type === 'insertImage')
        ) {
          void this.executeSourceCommand(command).catch(onFailure);
          return true;
        }
        return this.active?.execute(command) ?? false;
      },
      canExecute: (command) => command.type === 'undo'
          ? this.historyWritable && !!this.document?.state.status.canUndo
          : command.type === 'redo'
            ? this.historyWritable && !!this.document?.state.status.canRedo
            : this.writable && !this.disposed &&
              (this.source || this.currentHost?.hasGlobalSelection
                ? this.supportsSourceCommand(command) && !this.document?.state.failure
                : command.type === 'insertImage' || (this.active?.canExecute(command) ?? false)),
    };
    this.editor = new Proxy(this.fallback, {
      get: (_target, property) => {
        const target = property in overrides ? overrides : (this.active ?? this.fallback);
        const value = Reflect.get(target, property);
        return typeof value === 'function' ? value.bind(target) : value;
      },
    });
    sessions.set(this.editor, this);
    let originalText = tab.markdown;
    let savedText = tab.savedMarkdown;
    this.ready = (async () => {
      const port = desktop ? nativePort : memoryPort;
      const input = tab.documentId
        ? { recoveryId: tab.documentId }
        : desktop && tab.nativePath
          ? { path: tab.nativePath }
          : { text: savedText };
      const doc = await ProgressiveMarkdownDocument.open(port, input);
      if (!tab.documentId && tab.dirty && originalText !== savedText) {
        await doc.session.applyEdits({
          viewGeneration: 0,
          historyGroupId: 'legacy-draft-import',
          edits: [{ fromByte: 0, toByte: doc.state.status.byteLength, insertedText: originalText }],
        });
      }
      this.document = doc;
      this.unsubscribeSelection = doc.selection.subscribe((selection) =>
        this.onSelection?.(selection),
      );
      if (tab.globalSelection)
        doc.selection.set({
          anchorByte: Math.min(tab.globalSelection.anchorByte, doc.state.status.byteLength),
          headByte: Math.min(tab.globalSelection.headByte, doc.state.status.byteLength),
        });
      const prefix = await doc.port.readWindow(
        doc.session.documentId,
        doc.state.status.revision,
        0,
        Math.min(doc.state.status.byteLength, 32 * 1024),
      );
      this.frontMatter = extractFrontMatterBlock(prefix.text);
      tab.documentId = doc.session.documentId;
      // 旧恢复资料只在会话导入确认后退出热路径；持久化元数据保留旧 draftId 供迁移失败恢复。
      tab.markdown = '';
      tab.savedMarkdown = '';
      originalText = '';
      savedText = '';
      tab.largeDocumentMode = false;
      tab.readonlyDocumentMode = false;
      this.unsubscribe = doc.subscribe((state) => {
        const pending = doc.session.state.pendingTransactions > 0;
        tab.dirty = state.status.dirty || pending;
        tab.version = state.status.revision;
        tab.byteLength = state.status.byteLength;
        tab.encoding = state.status.encoding ?? tab.encoding;
        if (!pending && state.status.revision !== this.frontMatterRevision) {
          this.frontMatterRevision = state.status.revision;
          void doc.port
            .readWindow(
              doc.session.documentId,
              state.status.revision,
              0,
              Math.min(state.status.byteLength, 32 * 1024),
            )
            .then((prefix) => {
              if (this.disposed || prefix.revision !== doc.state.status.revision) return;
              this.frontMatter = extractFrontMatterBlock(prefix.text);
              this.onMetadata();
            })
            .catch((error) => {
              // 标签已关闭或修订已更新时，迟到的元数据读取失败不属于当前正文。
              if (!this.disposed && !doc.session.state.closing &&
                state.status.revision === doc.state.status.revision &&
                !(error instanceof DocumentConflictError)) this.onFailure(error);
            });
        }
        this.onMetadata();
        const event = {
          ...this.fallback.getSnapshot(),
          markdown: '',
          dirty: tab.dirty,
          mode: this.options.mode ?? 'semantic',
          readonly: !this.writable,
          reason: 'session-metadata',
          pendingInlineMarks: {},
          contentRevision: doc.session.state.viewGeneration,
        } as EditorChangeEvent;
        for (const listener of this.listeners) listener(event);
      });
      if (this.disposed) await this.detach();
      return doc;
    })();
    void this.ready.catch(onFailure);
  }
  private beginOpening(target: HTMLElement) {
    if (this.document || this.openings.has(target)) return;
    const opening = { timer: setTimeout(() => {
      if (this.disposed || !this.openings.has(target)) return;
      const node = globalThis.document.createElement('div');
      node.className = 'markdown-session-preparing'; node.setAttribute('role', 'status');
      node.textContent = '正在准备正文…'; opening.node = node; target.append(node);
    }, 100), node: undefined as HTMLElement | undefined };
    this.openings.set(target, opening);
  }
  private clearOpening(target?: HTMLElement) {
    if (!target) return;
    const opening = this.openings.get(target);
    if (opening) { clearTimeout(opening.timer); opening.node?.remove(); this.openings.delete(target); }
  }
  private failOpening(target: HTMLElement, error: unknown) {
    if (this.disposed || (target !== this.semanticTarget && target !== this.sourceTarget)) return;
    this.clearOpening(target);
    const node = globalThis.document.createElement('div');
    node.className = 'markdown-session-preparing'; node.setAttribute('role', 'alert');
    node.textContent = `正文准备失败：${error instanceof Error ? error.message : String(error)}`;
    target.append(node);
    this.onFailure(error);
  }
  private async mountViews() {
    const doc = await this.ready;
    await this.teardown;
    if (this.disposed) return;
    const callbacks = (mode: 'semantic' | 'source') => ({
      onActive: (core: EditorCore | undefined, source: MarkdownSourceWindow | undefined) => {
        if (!core && !source && this.activeHostMode !== mode) return;
        const target = mode === 'source' ? this.sourceTarget : this.semanticTarget;
        const expected = this.viewMode === 'split' ? this.activeHostMode ?? 'semantic' : this.viewMode;
        // 另一栏首次完成挂载不等于用户切换操作目标；只有真实焦点才接管活动栏。
        if ((core || source) && expected !== mode && !target?.contains(globalThis.document.activeElement)) return;
        this.activeHostMode = core || source ? mode : undefined;
        this.active = core;
        this.source = source;
        if (this.pendingFocus && (core || source)) {
          this.clearPendingFocus();
          (mode === 'source' ? this.sourceHost : this.semanticHost)?.focus();
        }
      },
      onPresentation: (state: MarkdownPresentation) => {
        this.presentations.set(mode, state);
        if ((this.activeHostMode ?? (this.viewMode === 'source' ? 'source' : 'semantic')) !== mode) return;
        const event = { ...this.fallback.getSnapshot(), markdown: '', dirty: this.tab.dirty,
          mode: this.options.mode ?? 'semantic', readonly: !this.writable, reason: 'presentation',
          pendingInlineMarks: {}, contentRevision: doc.session.state.viewGeneration } as EditorChangeEvent;
        for (const listener of this.listeners) listener(event);
      },
      onOpenSource: (range: { byte: number }) => { void this.openSource(range.byte).catch(this.onFailure); },
      onFailure: this.onFailure,
      writeClipboard: (text: string) => writeEditorClipboard({ text, html: '' }, this.desktop),
      onScroll: (byte: number) => {
        if (
          this.viewMode !== 'split' ||
          (this.scrollOrigin !== mode && performance.now() < this.scrollUntil)
        )
          return;
        this.scrollOrigin = mode;
        this.scrollUntil = performance.now() + 120;
        (mode === 'source' ? this.semanticHost : this.sourceHost)?.scrollToByte(byte);
      },
    });
    if (this.semanticTarget && !this.semanticHost) {
      this.clearOpening(this.semanticTarget);
      this.semanticHost = new MarkdownViewportHost(
        doc,
        this.semanticTarget,
        'semantic',
        { ...this.options, mode: 'semantic' },
        callbacks('semantic'),
      );
      this.semanticHost.scrollToByte(
        this.tab.globalScrollAnchor?.byteOffset ?? doc.selection.selection.headByte,
      );
    }
    if (this.sourceTarget && !this.sourceHost) {
      this.clearOpening(this.sourceTarget);
      this.sourceHost = new MarkdownViewportHost(
        doc,
        this.sourceTarget,
        'source',
        this.options,
        callbacks('source'),
      );
      this.sourceHost.scrollToByte(
        this.tab.globalScrollAnchor?.byteOffset ?? doc.selection.selection.headByte,
      );
    }
    await this.semanticHost?.setHidden(this.viewMode === 'source');
    await this.sourceHost?.setHidden(this.viewMode === 'semantic');
    const pending = this.pendingSourceByte;
    if (pending && this.sourceHost && this.viewMode !== 'semantic') {
      this.pendingSourceByte = undefined;
      const focus = this.sourceFocus;
      this.clearSourceFocus();
      if (pending.revision === doc.state.status.revision) await this.sourceHost.reveal(pending.byte, focus);
    }
    if (this.pendingFocus && this.currentHost) { this.clearPendingFocus(); this.currentHost.focus(); }
    doc.setHidden(!this.semanticTarget && !this.sourceTarget);
  }
  mountSource(target: HTMLElement) {
    this.sourceTarget = target;
    this.beginOpening(target);
    void this.mountViews().catch(error => this.failOpening(target, error));
    return () => {
      if (this.sourceTarget !== target) return;
      this.clearOpening(target);
      this.sourceTarget = undefined;
      this.source = undefined;
      const host = this.sourceHost;
      this.sourceHost = undefined;
      this.teardown = this.teardown.then(() => host?.dispose());
      void this.teardown.catch(this.onFailure);
    };
  }
  setViewMode(mode: 'semantic' | 'source' | 'split') {
    const generation = ++this.modeGeneration;
    if (mode === 'semantic') { this.sourceRequest++; this.pendingSourceByte = undefined; this.clearSourceFocus(); }
    this.captureSelection();
    const origin = this.currentHost;
    const anchor = origin?.position;
    origin?.preserveReadingPosition();
    // 源码宿主可能由稍后的 Svelte action 创建，必须接续模式切换前的位置。
    if (anchor) this.tab.globalScrollAnchor = anchor;
    this.viewMode = mode;
    this.options.mode = mode === 'source' ? 'source' : 'semantic';
    return this.mountViews()
      .then(() => {
        if (this.disposed || generation !== this.modeGeneration || !anchor) return;
        const visible = mode === 'split' ? [this.semanticHost, this.sourceHost] :
          [mode === 'source' ? this.sourceHost : this.semanticHost];
        for (const host of visible) if (host && host !== origin) host.scrollToByte(anchor.byteOffset);
      })
      .catch(this.onFailure);
  }
  async openSource(byte: number) {
    const request = ++this.sourceRequest;
    this.sourceFocus = true;
    this.watchFocusIntent();
    try {
      const doc = await this.ready;
      const revision = doc.state.status.revision;
      if (this.disposed || request !== this.sourceRequest) return;
      if (this.onRequestSource) {
        if (!await this.onRequestSource(() => !this.disposed && request === this.sourceRequest && revision === doc.state.status.revision)) return;
      } else await this.setViewMode('source');
      if (this.disposed || request !== this.sourceRequest || revision !== doc.state.status.revision || this.viewMode === 'semantic') return;
      this.pendingSourceByte = { byte, revision };
      await this.mountViews();
    } finally {
      if (request === this.sourceRequest && !this.pendingSourceByte) this.clearSourceFocus();
    }
  }
  async unmount() {
    this.sourceRequest++; this.pendingSourceByte = undefined;
    this.clearOpening(this.semanticTarget);
    this.clearPendingFocus();
    this.captureSelection();
    this.tab.globalScrollAnchor =
      (this.source ? this.sourceHost : this.semanticHost)?.position ?? this.tab.globalScrollAnchor;
    this.semanticTarget = undefined;
    const host = this.semanticHost;
    this.semanticHost = undefined;
    this.teardown = this.teardown.then(() => host?.dispose());
    await this.teardown;
    this.document?.setHidden(!this.sourceTarget);
    this.active = undefined;
    this.source = undefined;
  }
  async readMarkdown(signal?: AbortSignal) {
    return (await this.ready).readMarkdown(signal);
  }
  async capture(signal?: AbortSignal, intent: 'explicit' | 'automatic' = 'explicit') {
    const doc = await this.ready;
    return doc.session.withSnapshot(
      async (snapshot) => ({
        markdown: await readCompleteSnapshot(doc.port, snapshot, signal),
        revision: snapshot.revision,
      }),
      intent,
      signal,
    );
  }
  async replaceMarkdown(markdown: string, expectedGeneration?: number) {
    const doc = await this.ready;
    await doc.session.flushEdits();
    const generation = doc.session.state.viewGeneration;
    const current = await doc.session.withSnapshot((snapshot) =>
      readCompleteSnapshot(doc.port, snapshot),
    );
    if (
      generation !== doc.session.state.viewGeneration ||
      (expectedGeneration !== undefined && generation !== expectedGeneration)
    )
      throw new DocumentConflictError();
    const edits = getSourceTextChanges(current, markdown).map((change) => ({
      fromByte: utf16ToUtf8(current, change.from),
      toByte: utf16ToUtf8(current, change.to),
      insertedText: change.insert,
    }));
    await doc.session.applyEdits({
      viewGeneration: generation,
      historyGroupId: globalThis.crypto.randomUUID(),
      edits,
      selectionAfter: { anchorByte: utf8Length(markdown), headByte: utf8Length(markdown) },
    });
  }
  async transformMarkdown(transform: (text: string) => string) {
    this.assertWritable();
    const doc = await this.ready;
    await doc.session.flushEdits();
    const generation = doc.session.state.viewGeneration;
    const current = await this.readMarkdown();
    if (generation !== doc.session.state.viewGeneration) throw new DocumentConflictError();
    const next = transform(current);
    if (next !== current) {
      this.assertWritable();
      await this.replaceMarkdown(next, generation);
      this.frontMatter = extractFrontMatterBlock(next);
      this.onMetadata();
    }
  }
  private get currentHost() {
    return this.viewMode === 'source' || (this.viewMode === 'split' && this.activeHostMode === 'source')
      ? this.sourceHost : this.semanticHost;
  }
  private cancelFocusOutside = (event: Event) => {
    if (event.target === globalThis.document.body && event.type === 'focusin') return;
    if (event.target instanceof Node && !this.semanticTarget?.contains(event.target) && !this.sourceTarget?.contains(event.target))
      this.clearPendingFocus();
  };
  private queueFocus() {
    this.pendingFocus = true;
    this.watchFocusIntent();
  }
  private watchFocusIntent() {
    globalThis.document.addEventListener('pointerdown', this.cancelFocusOutside, true);
    globalThis.document.addEventListener('focusin', this.cancelFocusOutside, true);
  }
  private clearSourceFocus() {
    this.sourceFocus = false;
    if (!this.pendingFocus) this.releaseFocusIntent();
  }
  private clearPendingFocus() {
    this.pendingFocus = false;
    this.sourceFocus = false;
    this.releaseFocusIntent();
  }
  private releaseFocusIntent() {
    globalThis.document.removeEventListener('pointerdown', this.cancelFocusOutside, true);
    globalThis.document.removeEventListener('focusin', this.cancelFocusOutside, true);
  }
  get presentation() { return this.presentations.get(this.activeHostMode ?? (this.viewMode === 'source' ? 'source' : 'semantic')); }
  private get sessionWritable() {
    const state = this.document?.session.state;
    return !!state && !this.disposed && !this.options.readonly && !state.failure &&
      !this.document?.state.failure && !state.closing && !state.historyBusy;
  }
  get writable() { return this.sessionWritable && !!this.currentHost?.writable; }
  private get historyWritable() {
    if (this.currentHost?.mode === 'source') return this.writable;
    const phase = this.currentHost?.presentation.phase;
    return this.sessionWritable && !!phase && phase !== 'preparing' && phase !== 'preview';
  }
  private assertWritable() {
    if (!this.writable) throw new DocumentConflictError('此处尚未开放编辑。');
  }
  get readingByte() {
    return this.currentHost?.position.byteOffset ?? 0;
  }
  get isGlobalSelection() {
    return !!this.currentHost?.hasGlobalSelection;
  }
  captureSelection() {
    return this.currentHost?.captureSelection() ?? this.document?.selection.selection;
  }
  /** 异步资源导入保留文档字节书签，局部编辑映射位置；撤销/整版恢复后明确失效。 */
  createInsertionBookmark() {
    this.assertWritable();
    this.active?.commitPendingEdits();
    this.captureSelection();
    const doc = this.document;
    if (!doc) throw new Error('文档尚未准备好。');
    let selection = doc.selection.selection;
    let valid = true;
    const unsubscribe = doc.session.subscribeChanges((change) => {
      if (change.edits) selection = mapMarkdownSelection(selection, change.edits);
      else valid = false;
    });
    return {
      dispose: unsubscribe,
      insert: async (text: string) => {
        await doc.session.flushEdits();
        // 已授权的书签可在标签隐藏后提交；仍需校验会话权限和映射有效性。
        if (!valid || !this.sessionWritable) throw new DocumentConflictError('图片插入位置已失效。');
        const fromByte = Math.min(selection.anchorByte, selection.headByte);
        const toByte = Math.max(selection.anchorByte, selection.headByte);
        const after = fromByte + utf8Length(text);
        await doc.session.applyEdits({
          viewGeneration: doc.session.state.viewGeneration,
          edits: [{ fromByte, toByte, insertedText: text }],
          historyGroupId: globalThis.crypto.randomUUID(),
          selectionBefore: selection,
          selectionAfter: { anchorByte: after, headByte: after },
        });
        if (this.semanticTarget || this.sourceTarget) await this.reveal(after, false);
      },
    };
  }
  insertAt(selection: DocumentSelection, generation: number, text: string) {
    if (
      !this.writable ||
      !this.document ||
      generation !== this.document.session.state.viewGeneration ||
      this.disposed
    )
      return false;
    void this.insertText(text, selection, generation).catch(this.onFailure);
    return true;
  }
  async insertText(text: string, selection?: DocumentSelection, generation?: number) {
    this.assertWritable();
    const doc = await this.ready;
    await doc.session.flushEdits();
    this.assertWritable();
    if (generation !== undefined && generation !== doc.session.state.viewGeneration)
      throw new DocumentConflictError();
    if (selection) doc.selection.set(selection);
    else this.captureSelection();
    await doc.selection.replaceSelection(text, () => this.writable);
    await this.reveal(doc.selection.selection.headByte);
  }
  async paste(input: Parameters<EditorCore['pasteClipboard']>[0], mode: 'auto' | 'plain') {
    this.assertWritable();
    if (this.active && !this.source && !this.currentHost?.hasGlobalSelection) {
      this.active.pasteClipboard(input, { mode });
      await this.flush();
    } else await this.insertText(input.text ?? '');
  }
  async revealLine(line: number, column = 1) {
    const doc = await this.ready;
    await doc.session.withSnapshot(async (snapshot) => {
      const generation = doc.session.state.viewGeneration;
      const targetLine = Math.max(1, line);
      let remainingColumn = Math.max(0, column - 1);
      let currentLine = 1;
      let offset = 0;
      const navigate = async (byte: number) => {
        if (generation !== doc.session.state.viewGeneration) throw new DocumentConflictError();
        await this.reveal(byte);
      };
      while (offset < snapshot.byteLength) {
        const part = await doc.port.readSnapshot(
          snapshot,
          offset,
          Math.min(snapshot.byteLength, offset + 64 * 1024),
        );
        const lines = part.text.split(/(\r\n|\n|\r)/);
        for (let i = 0; i < lines.length; i += 2) {
          if (currentLine === targetLine) {
            // 一个长行可以跨越多个读取窗口，只有真实换行或 EOF 才截断目标列。
            if (remainingColumn <= lines[i].length || lines[i + 1] || part.toByte === snapshot.byteLength) {
              await navigate(offset + utf16ToUtf8(lines[i], Math.min(lines[i].length, remainingColumn)));
              return;
            }
            remainingColumn -= lines[i].length;
          }
          offset += utf8Length(lines[i]);
          if (lines[i + 1]) {
            currentLine++;
            offset += utf8Length(lines[i + 1]);
          }
        }
      }
      await navigate(snapshot.byteLength);
    });
  }
  private async executeSourceCommand(command: EditorCommand) {
    this.assertWritable();
    const doc = await this.ready;
    this.captureSelection();
    const before = doc.selection.selection;
    const generation = doc.session.state.viewGeneration;
    const selected = await doc.selection.copy();
    const wrap: Record<string, string> = {
      toggleBold: '**',
      toggleItalic: '*',
      toggleCode: '`',
      toggleStrikethrough: '~~',
      toggleUnderline: '++',
      toggleHighlight: '==',
      toggleSuperscript: '^',
      toggleSubscript: '~',
    };
    let replacement: string;
    if (wrap[command.type]) {
      const marker = wrap[command.type];
      replacement =
        selected.startsWith(marker) &&
        selected.endsWith(marker) &&
        selected.length >= marker.length * 2
          ? selected.slice(marker.length, -marker.length)
          : marker + selected + marker;
    } else
      switch (command.type) {
        case 'insertText':
          replacement = command.text;
          break;
        case 'insertImage':
          replacement = `![${command.alt ?? ''}](<${command.src}>)`;
          break;
        case 'insertLink':
          replacement = `[${command.text ?? selected}](<${command.href}>)`;
          break;
        case 'insertDetails':
          replacement = `<details>\n<summary></summary>\n\n${selected}\n\n</details>`;
          break;
        case 'insertToc':
          replacement = createTocBlock(await this.readMarkdown());
          break;
        case 'setHeading':
          replacement = `${'#'.repeat(command.level)} ${selected.replace(/^#{1,6}\s+/, '')}`;
          break;
        case 'setParagraph':
          replacement = selected.replace(/^#{1,6}\s+/gm, '');
          break;
        case 'toggleBlockquote':
          replacement = selected
            .split('\n')
            .map((line) => '> ' + line)
            .join('\n');
          break;
        case 'toggleBulletList':
          replacement = selected
            .split('\n')
            .map((line) => '- ' + line)
            .join('\n');
          break;
        case 'toggleOrderedList':
          replacement = selected
            .split('\n')
            .map((line, index) => `${index + 1}. ${line}`)
            .join('\n');
          break;
        case 'toggleTaskList':
          replacement = selected
            .split('\n')
            .map((line) => '- [ ] ' + line)
            .join('\n');
          break;
        case 'insertCodeBlock':
          replacement = `\n\n\`\`\`${command.language ?? ''}\n${command.code ?? selected}\n\`\`\`\n`;
          break;
        case 'insertMathBlock':
          replacement = `\n\n$$\n${command.tex ?? selected}\n$$\n`;
          break;
        case 'insertMathInline':
          replacement = `$${selected}$`;
          break;
        case 'insertMermaidBlock':
          replacement = `\n\n\`\`\`mermaid\n${command.code ?? selected}\n\`\`\`\n`;
          break;
        case 'insertHorizontalRule':
          replacement = '\n\n---\n\n';
          break;
        case 'insertParagraphAfter':
          replacement = selected + '\n\n';
          break;
        case 'insertParagraphBefore':
          replacement = '\n\n' + selected;
          break;
        default:
          throw new Error('此结构操作需要在语义区域内执行。');
      }
    if (
      generation !== doc.session.state.viewGeneration ||
      JSON.stringify(before) !== JSON.stringify(doc.selection.selection)
    )
      throw new DocumentConflictError();
    this.assertWritable();
    await doc.selection.replaceSelection(replacement, () => this.writable);
    await this.reveal(doc.selection.selection.headByte);
  }
  private supportsSourceCommand(command: EditorCommand) {
    return [
      'toggleBold',
      'toggleItalic',
      'toggleCode',
      'toggleStrikethrough',
      'toggleUnderline',
      'toggleHighlight',
      'toggleSuperscript',
      'toggleSubscript',
      'insertText',
      'insertImage',
      'insertLink',
      'insertDetails',
      'insertToc',
      'setHeading',
      'setParagraph',
      'toggleBlockquote',
      'toggleBulletList',
      'toggleOrderedList',
      'toggleTaskList',
      'insertCodeBlock',
      'insertMathBlock',
      'insertMathInline',
      'insertMermaidBlock',
      'insertHorizontalRule',
      'insertParagraphAfter',
      'insertParagraphBefore',
    ].includes(command.type);
  }
  reveal(byte: number, focus = true) {
    return (
      this.currentHost ?? (this.viewMode === 'source' ? this.sourceHost : this.semanticHost)
    )?.reveal(byte, focus);
  }
  selectRange(fromByte: number, toByte: number) {
    return this.currentHost?.selectRange({ anchorByte: fromByte, headByte: toByte });
  }
  async flush() {
    // 打开失败没有待提交视图；错误已通过 onFailure 显示，不能阻断其它标签的切换。
    const doc = await this.ready.catch(() => this.document);
    await doc?.session.flushEdits();
  }
  async prepareTransfer() {
    const doc = await this.ready;
    await doc.session.flushEdits();
    this.captureSelection();
    this.tab.globalSelection = doc.selection.selection;
    this.tab.globalScrollAnchor =
      (this.source ? this.sourceHost : this.semanticHost)?.position ?? this.tab.globalScrollAnchor;
    await this.semanticHost?.setHidden(true);
    await this.sourceHost?.setHidden(true);
    await doc.session.awaitSnapshotTasks();
    await doc.suspendTasks();
  }
  resume() {
    void this.mountViews().catch(this.onFailure);
  }
  /** 移交只释放本窗口引用；不能关闭已经交给目标窗口的同一原生会话。 */
  async detach() {
    if (this.disposed && !this.document) return;
    this.disposed = true;
    for (const target of this.openings.keys()) this.clearOpening(target);
    this.sourceRequest++; this.pendingSourceByte = undefined; this.presentations.clear();
    this.generation++;
    await this.unmount();
    await this.sourceHost?.dispose();
    this.sourceHost = undefined;
    this.sourceTarget = undefined;
    this.unsubscribe?.();
    this.unsubscribeSelection?.();
    this.listeners.clear();
    this.fallback.destroy();
    await this.document?.detach();
  }
  async close(discard: boolean) {
    this.clearPendingFocus();
    for (const target of this.openings.keys()) this.clearOpening(target);
    this.sourceRequest++; this.pendingSourceByte = undefined; this.presentations.clear();
    const doc = await this.ready.catch((error) => {
      if (!discard) throw error;
      return this.document;
    });
    if (!doc) {
      this.disposed = true;
      this.listeners.clear();
      this.fallback.destroy();
      return;
    }
    await this.unmount();
    await this.sourceHost?.dispose();
    this.sourceHost = undefined;
    await doc.session.awaitSnapshotTasks();
    await doc.close(discard);
    this.unsubscribe?.();
    this.unsubscribeSelection?.();
    this.listeners.clear();
    this.fallback.destroy();
    this.disposed = true;
  }
}
