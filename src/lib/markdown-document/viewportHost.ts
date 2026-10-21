import type { EditorCoreOptions, EditorRuntimeOptions } from '../editor-core/types';
import type { ProgressiveMarkdownDocument } from './runtime';
import { MarkdownSemanticWindow } from './semanticWindow';
import { MarkdownSourceWindow } from './sourceWindow';
import {
  MarkdownHeightIndex,
  estimateMarkdownBlockHeight,
} from './viewport';
import { MarkdownStableGroups, MarkdownScrollDemand, MARKDOWN_VIEWPORT_POLICY, type MarkdownBlockGroup } from './mountWindow';
import { MarkdownLayoutCache } from './layoutCache';
import { MarkdownLayoutBatch, markdownWorkScheduler, MarkdownTaskPriority } from './scheduler';
import { TABLE_LAYOUT_READY_EVENT } from '../table-layout/dom';
import { subscribeTypographyInteraction } from '../typography/interaction';
import { DocumentConflictError, type DocumentSelection, type DocumentWindow } from './protocol';
import { utf8ToUtf16, utf8Length } from './utf8Text';
import type { MarkdownStructureIndex } from './structureIndex';

interface Mounted {
  key: string;
  groupId?: string;
  signature?: string;
  layoutKey?: string;
  from?: number;
  to?: number;
  element: HTMLDivElement;
  window: DocumentWindow;
  semantic?: MarkdownSemanticWindow;
  source?: MarkdownSourceWindow;
}

/** 真实滚动宿主。占位只存在于宿主 DOM；所有编辑器都绑定完整会话的局部投影。 */
export class MarkdownViewportHost {
  private mounted: Mounted[] = [];
  private heights = new MarkdownHeightIndex();
  private index: MarkdownStructureIndex | null = null;
  private unsubscribe: () => void;
  private observer: ResizeObserver | undefined;
  private frame = 0;
  private generation = 0;
  private disposed = false;
  private hidden = false;
  private busy = false;
  private again = false;
  private focusAfterRemount = false;
  private focusFrame = 0;
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private expanded = 0;
  private active: Mounted | undefined;
  private globalSelection = false;
  private pointerAnchor: number | undefined;
  private pointerView?: Mounted;
  private preferredByte: number | undefined;
  private preferredFocus = false;
  private scrollByte: number | undefined;
  private input: HTMLTextAreaElement | undefined;
  private composing = false;
  private compositionWaiters = new Set<() => void>();
  private detachInput: () => void;
  private renderDone: Promise<void> = Promise.resolve();
  private readonly layoutCache = new MarkdownLayoutCache();
  private readonly grouping = new MarkdownStableGroups();
  private groups: readonly MarkdownBlockGroup[] = [];
  private groupById = new Map<string, MarkdownBlockGroup>();
  private spacers = new Map<string, HTMLDivElement>();
  private readonly demand = new MarkdownScrollDemand();
  private quietTimer?: ReturnType<typeof setTimeout>;
  private layoutContext = '';
  private geometryVersion = 0;
  private assetVersion = 0;
  private programScroll?: number;
  private navigation = 0;
  private preparationTimer?: ReturnType<typeof setTimeout>;
  private preparationIndicator?: HTMLDivElement;
  private preparationStarted = 0;
  private readingAnchor: ReturnType<MarkdownViewportHost['captureAnchor']>;
  private readonly dirty = new Set<Mounted>();
  private blockPositions = new Map<string, number>();
  private layoutBatch: MarkdownLayoutBatch;
  private typographySubscription: { destroy(): void };
  private readonly stats = { ranges: 0, created: 0, destroyed: 0, moved: 0,
    measurements: 0, heightChanges: 0, tablePublications: 0, tableRestores: 0,
    anchorCorrections: 0, overBudget: 0, peakMounted: 0, batches: 0, maxPreparationMs: 0,
    preparationHints: 0, maxJumpWaitMs: 0 };
  private installedLayout = '';
  private scale = 1;
  private contentTop = 0;
  private lastIndexedBlock: MarkdownStructureIndex['blocks'][number] | undefined;
  private readonly scroll: HTMLElement;
  constructor(
    readonly document: ProgressiveMarkdownDocument,
    readonly target: HTMLElement,
    readonly mode: 'semantic' | 'source',
    private options: Omit<EditorCoreOptions, 'markdown' | 'documentProjection'>,
    private callbacks: {
      onActive(
        core: MarkdownSemanticWindow['core'] | undefined,
        source: MarkdownSourceWindow | undefined,
      ): void;
      onFailure(error: unknown): void;
      writeClipboard(text: string): Promise<void>;
      onScroll?(byte: number): void;
    },
  ) {
    this.scroll = target.closest<HTMLElement>('.semantic-pane, .source-pane') ?? target;
    this.layoutBatch = new MarkdownLayoutBatch(() => this.captureAnchor(), anchor => this.restoreAnchor(anchor));
    this.typographySubscription = subscribeTypographyInteraction(snapshot => {
      if (snapshot.geometryRevision === this.geometryVersion) return;
      this.geometryVersion = snapshot.geometryRevision;
      this.invalidateGeometry();
    });
    const restoredSelection = document.selection.selection;
    this.globalSelection = restoredSelection.anchorByte !== restoredSelection.headByte;
    target.classList.add('markdown-session-host');
    this.scroll.classList.add('markdown-session-scroll');
    this.scroll.addEventListener('scroll', this.onScroll, { passive: true });
    target.addEventListener('keydown', this.onKey, true);
    target.addEventListener('keyup', this.onSelectionChanged, true);
    target.addEventListener('copy', this.onCopy, true);
    target.addEventListener('cut', this.onCopy, true);
    target.addEventListener('beforeinput', this.onBeforeInput, true);
    target.addEventListener('pointerdown', this.onPointer, true);
    globalThis.document.addEventListener('pointerup', this.onPointerUp, true);
    globalThis.document.addEventListener('pointercancel', this.onPointerUp, true);
    globalThis.document.addEventListener('pointermove', this.onPointerMove, true);
    globalThis.document.addEventListener('pointerdown', this.onOutsidePointer, true);
    globalThis.document.addEventListener('focusin', this.onDocumentFocus, true);
    target.addEventListener('input', this.onInput, true);
    target.addEventListener('focusin', this.onFocus, true);
    target.addEventListener(TABLE_LAYOUT_READY_EVENT, this.onTableLayout);
    target.addEventListener('load', this.onAsset, true);
    target.ownerDocument.fonts?.addEventListener('loadingdone', this.onFont);
    if (typeof ResizeObserver !== 'undefined')
      this.observer = new ResizeObserver(entries => {
        for (const entry of entries) {
          const view = this.mounted.find(view => view.element === entry.target);
          if (view) this.dirty.add(view);
        }
        this.measureDirty();
        this.schedule();
      });
    this.observer?.observe(target);
    this.detachInput = document.session.attachEdits({
      awaitCompositionEnd: () =>
        this.composing
          ? new Promise((resolve) => this.compositionWaiters.add(resolve))
          : Promise.resolve(),
      commitPendingEdits() {},
    });
    this.unsubscribe = document.subscribe(() => this.schedule());
  }
  get activeCore() {
    return this.active?.semantic?.core;
  }
  get activeSource() {
    return this.active?.source;
  }
  get hasGlobalSelection() {
    return this.globalSelection;
  }
  captureSelection() {
    if (!this.globalSelection) {
      const selection = this.selection();
      if (selection) this.document.selection.set(selection);
    }
    return this.document.selection.selection;
  }
  get position() {
    const first = this.index?.blocks[this.heights.indexAt(this.localScrollTop)];
    return {
      byteOffset: first?.fromByte ?? this.active?.window.fromByte ?? 0,
      line: first?.fromLine ?? 1,
    };
  }
  private get localScrollTop() {
    return Math.max(0, (this.scroll.scrollTop - this.contentTop) / this.scale);
  }
  private scrollTo(top: number) {
    this.scroll.scrollTop = this.contentTop + top * this.scale;
    this.programScroll = this.scroll.scrollTop;
    this.demand.reset(top, performance.now());
  }
  private pinned(view: Mounted) {
    return !!(
      view.source?.pinned ||
      view === this.pointerView ||
      (this.composing && view === this.active) ||
      view.semantic?.bridge.pinned ||
      view.semantic?.core.hasUnsettledProjection()
    );
  }
  private selection(view = this.active): DocumentSelection | undefined {
    return view?.source?.selection ?? view?.semantic?.core.getSourceSelection(false);
  }
  private onSelectionChanged = () => {
    if (!this.composing) this.captureSelection();
  };
  private onInput = () => {
    this.focusAfterRemount = this.target.contains(globalThis.document.activeElement);
    this.document.interacting();
  };
  private onOutsidePointer = (event: PointerEvent) => {
    if (event.target instanceof Node && !this.target.contains(event.target)) this.focusAfterRemount = false;
  };
  private onDocumentFocus = (event: FocusEvent) => {
    // WebView2 在过期投影变为只读时会暂时把焦点交给 body；这不代表用户离开编辑区。
    if (event.target !== globalThis.document.body && event.target instanceof Node &&
      !this.target.contains(event.target)) this.focusAfterRemount = false;
  };
  private onScroll = () => {
    if (this.programScroll !== undefined && Math.abs(this.scroll.scrollTop - this.programScroll) < 1) {
      this.programScroll = undefined;
      this.schedule();
      return;
    }
    this.programScroll = undefined;
    this.requestPreparation();
    this.generation++;
    this.readingAnchor = undefined;
    this.demand.sample(this.localScrollTop, (this.scroll.clientHeight || 800) / this.scale, performance.now());
    clearTimeout(this.quietTimer);
    this.quietTimer = setTimeout(() => {
      this.quietTimer = undefined;
      this.readingAnchor = this.captureAnchor();
      this.schedule();
    }, MARKDOWN_VIEWPORT_POLICY.quietMs);
    this.document.interacting();
    this.schedule();
    this.callbacks.onScroll?.(this.position.byteOffset);
  };
  private requestPreparation() {
    if (this.preparationTimer || this.preparationIndicator || this.hidden) return;
    this.preparationStarted = performance.now();
    this.preparationTimer = setTimeout(() => {
      this.preparationTimer = undefined;
      if (this.hidden || this.disposed || this.hasVisibleContent()) return;
      const indicator = this.preparationIndicator = globalThis.document.createElement('div');
      indicator.className = 'markdown-session-preparing';
      indicator.setAttribute('role', 'status');
      indicator.textContent = '正在准备此处内容…';
      Object.assign(indicator.style, { position: 'absolute', top: `${this.localScrollTop + 12}px`,
        left: '12px', pointerEvents: 'none', fontSize: '0.875em', opacity: '0.7' });
      this.target.append(indicator);
      this.stats.preparationHints++;
    }, 100);
  }
  private hasVisibleContent() {
    const top = this.localScrollTop;
    return this.mounted.some(view => {
      if (this.preferredByte !== undefined && (this.preferredByte < view.window.fromByte ||
        this.preferredByte > view.window.toByte)) return false;
      if (view.from !== undefined && view.to! > view.from)
        return this.heights.top(view.from) <= top && this.heights.top(view.to!) > top;
      // 索引未就绪的局部原文没有块序号，按实际窗口位置判断。
      const rect = view.element.getBoundingClientRect();
      const viewport = this.scroll.getBoundingClientRect();
      return rect.height > 0 && rect.top <= viewport.top + 4 && rect.bottom > viewport.top;
    });
  }
  private finishPreparation() {
    if (!this.hasVisibleContent()) return;
    if (this.preparationStarted) this.stats.maxJumpWaitMs = Math.max(this.stats.maxJumpWaitMs, performance.now() - this.preparationStarted);
    this.preparationStarted = 0;
    clearTimeout(this.preparationTimer); this.preparationTimer = undefined;
    this.preparationIndicator?.remove(); this.preparationIndicator = undefined;
  }
  private onFocus = (event: FocusEvent) => {
    const view = this.mounted.find((view) => view.element.contains(event.target as Node));
    if (view) {
      this.focusAfterRemount = true;
      this.active = view;
      this.callbacks.onActive(view.semantic?.core, view.source);
    }
  };
  get diagnostics() {
    return { ...this.stats, mounted: this.mounted.length, cacheEntries: this.layoutCache.size,
      residentWindowBytes: this.mounted.reduce((sum, view) => sum + view.window.toByte - view.window.fromByte, 0),
      cacheBytes: this.layoutCache.estimatedBytes, cacheHits: this.layoutCache.hits,
      cacheMisses: this.layoutCache.misses, preparationMs: this.demand.preparationMs };
  }
  private invalidateGeometry() {
    this.layoutContext = '';
    this.layoutCache.clear();
    for (const view of this.mounted) { view.layoutKey = undefined; this.dirty.add(view); }
    this.schedule();
  }
  private onFont = () => { this.assetVersion++; this.invalidateGeometry(); };
  private onAsset = () => { this.assetVersion++; this.invalidateGeometry(); };
  private onTableLayout = (event: Event) => {
    this.stats.tablePublications++;
    const view = this.mounted.find(view => event.target instanceof Node && view.element.contains(event.target));
    if (view) { this.dirty.add(view); this.measureDirty(); }
  };
  private blockLayoutKey(block: MarkdownStructureIndex['blocks'][number]) {
    return `${block.id}:${block.contentVersion ?? block.version}:${block.sourceHash}:${JSON.stringify(block.references.map(key => this.index?.references[key]))}:${this.layoutContext}`;
  }
  private captureAnchor() {
    if (this.preferredByte !== undefined || !this.index) return undefined;
    const index = this.heights.indexAt(this.localScrollTop);
    const block = this.index.blocks[index];
    if (!block) return undefined;
    const rect = this.scroll.getBoundingClientRect();
    const view = this.mounted.find(view => view.from !== undefined && view.from <= index && view.to! > index);
    let semantic: ReturnType<NonNullable<Mounted['semantic']>['core']['captureBlockReadingAnchor']>;
    try { semantic = view?.semantic?.core.captureBlockReadingAnchor(this.target.getBoundingClientRect().left + 12, rect.top + 4); }
    catch { /* 未完成布局时使用块级高度锚点。 */ }
    const line = semantic && view?.from !== undefined ? this.index.blocks[view.from].fromLine + semantic.line - 1 : undefined;
    const hit = line === undefined || view?.from === undefined ? block :
      this.index.blocks.slice(view.from, view.to).reverse().find(block => block.fromLine <= line) ?? block;
    return { id: hit.id, offset: semantic?.offset ?? 0,
      screen: semantic ? semantic.top - rect.top : undefined,
      fallback: this.localScrollTop - this.heights.top(this.blockPositions.get(hit.id) ?? index),
      generation: this.generation };
  }
  private restoreAnchor(value: unknown) {
    const anchor = value as ReturnType<MarkdownViewportHost['captureAnchor']>;
    if (!anchor || anchor.generation !== this.generation || this.preferredByte !== undefined) return;
    const index = this.blockPositions.get(anchor.id);
    if (index === undefined) return;
    const block = this.index!.blocks[index];
    const view = this.mounted.find(view => block.fromByte >= view.window.fromByte && block.fromByte < view.window.toByte);
    const rect = anchor.screen === undefined || view?.from === undefined ? undefined :
      view.semantic?.core.blockReadingAnchorRect(block.fromLine - this.index!.blocks[view.from].fromLine + 1, anchor.offset);
    const top = rect && anchor.screen !== undefined
      ? this.localScrollTop + (rect.top - this.scroll.getBoundingClientRect().top - anchor.screen) / this.scale
      : this.heights.top(index) + anchor.fallback;
    if (Math.abs(top - this.localScrollTop) >= 1) { this.stats.anchorCorrections++; this.scrollTo(top); }
  }
  private measureDirty() {
    if (!this.dirty.size || this.hidden || this.disposed) return;
    this.layoutBatch.enqueue(this, () => {
      const values = [...this.dirty].filter(view => this.mounted.includes(view)).map(view =>
        ({ view, height: view.element.offsetHeight, top: view.element.getBoundingClientRect().top,
          blocks: view.semantic?.core.measureStructureBlocks(), tables: view.semantic?.core.snapshotTableLayout() }));
      this.dirty.clear();
      this.stats.measurements += values.length;
      return values;
    }, values => {
      for (const { view, height, top, blocks, tables } of values) {
        if (view.layoutKey && height > 0 && this.hasOriginalSource(view)) this.layoutCache.put(view.layoutKey, { height, tables });
        if (height < 1 || view.from === undefined || view.to === undefined || !this.index) continue;
        if (view.window.fromByte !== this.index.blocks[view.from]?.fromByte ||
          view.window.toByte !== this.index.blocks[view.to - 1]?.toByte) continue;
        const old = this.heights.top(view.to) - this.heights.top(view.from);
        const measured: number[] = [];
        const tops = new Map(blocks?.map(block => [block.line, block.top]) ?? []);
        for (let i = view.from; i < view.to; i++) {
          const line = this.index.blocks[i].fromLine - this.index.blocks[view.from].fromLine + 1;
          const nextLine = this.index.blocks[i + 1]?.fromLine - this.index.blocks[view.from].fromLine + 1;
          const start = i === view.from ? top : tops.get(line);
          const end = i + 1 === view.to ? top + height * this.scale : tops.get(nextLine);
          measured.push(Math.max(1, start !== undefined && end !== undefined
            ? (end - start) / this.scale : this.heights.height(i) * height / Math.max(1, old)));
        }
        // 引用定义等无 DOM 块保留最小占位；有界组的合计高度必须收敛到实测值。
        const flexible = measured.reduce((sum, value) => sum + Math.max(0, value - 1), 0);
        let changed = false;
        measured.forEach((value, offset) => {
          const adjusted = flexible > 0 ? 1 + Math.max(0, height - measured.length) * (value - 1) / flexible : Math.max(1, height / measured.length);
          const i = view.from! + offset;
          if (Math.abs(this.heights.height(i) - adjusted) >= .1) { this.heights.update(i, adjusted); changed = true; }
          if (this.hasOriginalSource(view)) this.layoutCache.put(this.blockLayoutKey(this.index!.blocks[i]), { height: adjusted });
        });
        if (changed) this.stats.heightChanges++;
      }
      this.schedule();
    });
  }
  private hasOriginalSource(view: Mounted) {
    return (view.semantic?.bridge.text ?? view.source?.bridge.text) === view.window.text;
  }
  private mountedGroups(ids: Set<string | undefined>, pinned: number[]) {
    const result: MarkdownBlockGroup[] = [];
    for (const id of ids) { const group = id && this.groupById.get(id); if (group) result.push(group); }
    for (const index of pinned) {
      let low = 0, high = this.groups.length;
      while (low < high) { const mid = (low + high) >>> 1;
        if (this.groups[mid].to <= index) low = mid + 1; else high = mid; }
      if (this.groups[low]) result.push(this.groups[low]);
    }
    return result;
  }
  private onPointer = (event: PointerEvent) => {
    this.globalSelection = false;
    this.removeInput();
    this.clearSelectionPaint();
    this.pointerView = this.mounted.find((view) => view.element.contains(event.target as Node));
    this.pointerAnchor =
      this.byteAtCoords(this.pointerView, event) ?? this.selection(this.pointerView)?.headByte;
    if (this.pointerView) this.active = this.pointerView;
  };
  private byteAtCoords(view: Mounted | undefined, event: PointerEvent) {
    return (
      view?.source?.sourceByteAtCoords(event.clientX, event.clientY) ??
      view?.semantic?.core.sourceByteAtCoords(event.clientX, event.clientY)
    );
  }
  private onPointerMove = (event: PointerEvent) => {
    if (this.pointerAnchor === undefined) return;
    const view = this.mounted.find((view) => view.element.contains(event.target as Node));
    if (!view || (!this.globalSelection && view === this.pointerView)) return;
    const headByte = this.byteAtCoords(view, event);
    if (headByte === undefined) return;
    event.preventDefault();
    this.globalSelection = true;
    this.active = view;
    this.document.selection.set({ anchorByte: this.pointerAnchor, headByte });
    this.paintSelection();
  };
  private onPointerUp = () => {
    if (this.pointerAnchor === undefined) return;
    const selected = this.selection();
    if (selected && !this.globalSelection) this.document.selection.set(selected);
    this.pointerAnchor = undefined;
    this.pointerView = undefined;
    if (this.globalSelection) this.focusGlobalInput();
    this.schedule();
  };
  private onCopy = (event: ClipboardEvent) => {
    if (!this.globalSelection) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const action =
      event.type === 'cut'
        ? this.document.selection.cut(this.callbacks.writeClipboard)
        : this.document.selection.copy().then(this.callbacks.writeClipboard);
    void action.catch(this.callbacks.onFailure);
  };
  private onBeforeInput = (event: InputEvent) => {
    if (!this.globalSelection || event.isComposing || this.composing) return;
    if (!event.inputType.startsWith('delete') && !event.inputType.startsWith('insert')) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const inserted = event.inputType.startsWith('delete')
      ? ''
      : (event.data ?? (event.inputType === 'insertParagraph' ? '\n' : ''));
    this.commitGlobalInput(inserted);
  };
  private commitGlobalInput(text: string) {
    const selection = this.document.selection.selection;
    const range = this.document.selection.range;
    const byte = range.fromByte + utf8Length(text);
    const pending = this.document.session.applyEdits({
      viewGeneration: this.document.session.state.viewGeneration,
      edits: [{ ...range, insertedText: text }],
      historyGroupId: globalThis.crypto.randomUUID(),
      selectionBefore: selection,
      selectionAfter: { anchorByte: byte, headByte: byte },
    });
    this.document.selection.set({ anchorByte: byte, headByte: byte });
    this.globalSelection = false;
    this.clearSelectionPaint();
    this.removeInput();
    void pending.then(() => this.reveal(byte, true)).catch(this.callbacks.onFailure);
  }
  private focusGlobalInput() {
    if (this.input) {
      this.input.focus({ preventScroll: true });
      return;
    }
    const input = (this.input = globalThis.document.createElement('textarea'));
    input.className = 'markdown-session-input';
    input.style.top = `${this.localScrollTop}px`;
    input.setAttribute('aria-label', '文档选区输入');
    input.addEventListener('compositionstart', () => {
      this.composing = true;
    });
    input.addEventListener('compositionend', () => {
      // compositionend 后浏览器仍可能补发 input；等本轮 DOM 输入落定再提交一笔事务。
      queueMicrotask(() => {
        const text = input.value;
        this.composing = false;
        if (text) this.commitGlobalInput(text);
        for (const resolve of this.compositionWaiters) resolve();
        this.compositionWaiters.clear();
      });
    });
    input.addEventListener('paste', (event) => {
      if (event.clipboardData?.files.length) return;
      event.preventDefault();
      event.stopPropagation();
      this.commitGlobalInput(event.clipboardData?.getData('text/plain') ?? '');
    });
    this.target.append(input);
    input.focus({ preventScroll: true });
  }
  private removeInput() {
    if (!this.composing) {
      this.input?.remove();
      this.input = undefined;
    }
  }
  private onKey = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
      event.preventDefault();
      event.stopImmediatePropagation();
      this.selectAll();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && ['Home', 'End'].includes(event.key)) {
      event.preventDefault();
      event.stopImmediatePropagation();
      const byte = event.key === 'Home' ? 0 : this.document.state.status.byteLength;
      void this.navigate(byte, event.shiftKey);
      return;
    }
    if (
      this.globalSelection &&
      !this.composing &&
      ['Backspace', 'Delete', 'Enter'].includes(event.key)
    ) {
      event.preventDefault();
      event.stopImmediatePropagation();
      this.commitGlobalInput(event.key === 'Enter' ? '\n' : '');
      return;
    }
    if (
      this.globalSelection &&
      !this.composing &&
      ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Escape'].includes(event.key)
    ) {
      event.preventDefault();
      event.stopImmediatePropagation();
      const range = this.document.selection.range;
      this.globalSelection = false;
      this.removeInput();
      this.clearSelectionPaint();
      void this.navigate(
        event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? range.fromByte : range.toByte,
        false,
      ).catch(this.callbacks.onFailure);
      return;
    }
    if (!this.active) return;
    const left = ['ArrowLeft', 'ArrowUp', 'Backspace'].includes(event.key);
    const right = ['ArrowRight', 'ArrowDown', 'Delete'].includes(event.key);
    if (!left && !right) return;
    const edge = left ? this.active.window.fromByte : this.active.window.toByte;
    const semanticBoundary = this.active.semantic?.core.atSourceWindowBoundary(left ? -1 : 1);
    const selection =
      this.selection() ?? (semanticBoundary ? { anchorByte: edge, headByte: edge } : undefined);
    if (
      !selection ||
      (!semanticBoundary && selection.headByte !== edge) ||
      (!event.shiftKey && selection.headByte !== selection.anchorByte)
    )
      return;
    if ((left && edge === 0) || (right && edge === this.document.state.status.byteLength)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    void this.adjacentByte(edge, left ? -1 : 1)
      .then(async (byte) => {
        if (event.key === 'Backspace' || event.key === 'Delete') {
          this.document.selection.set({ anchorByte: edge, headByte: byte });
          await this.document.selection.replaceSelection('');
          await this.reveal(Math.min(edge, byte), true);
        } else await this.navigate(byte, event.shiftKey);
      })
      .catch(this.callbacks.onFailure);
  };
  private async adjacentByte(byte: number, direction: -1 | 1) {
    await this.document.session.flushEdits();
    const status = this.document.state.status;
    const window = await this.document.port.readWindow(
      status.documentId,
      status.revision,
      Math.max(0, byte - 8),
      Math.min(status.byteLength, byte + 8),
    );
    const offset = utf8ToUtf16(window.text, byte - window.fromByte);
    const text = direction < 0 ? window.text.slice(0, offset) : window.text.slice(offset);
    const character =
      direction < 0
        ? text.endsWith('\r\n')
          ? '\r\n'
          : (Array.from(text).at(-1) ?? '')
        : text.startsWith('\r\n')
          ? '\r\n'
          : (Array.from(text)[0] ?? '');
    return byte + direction * utf8Length(character);
  }
  private async navigate(byte: number, extend: boolean) {
    const anchor = this.globalSelection
      ? this.document.selection.selection.anchorByte
      : (this.selection()?.anchorByte ?? byte);
    await this.reveal(byte, true);
    const selected = this.selection();
    if (selected) {
      this.document.selection.set({
        anchorByte: extend ? anchor : selected.headByte,
        headByte: selected.headByte,
      });
      this.globalSelection = extend;
      if (extend) this.paintSelection();
    }
  }
  selectAll() {
    this.activeCore?.commitPendingEdits();
    this.globalSelection = true;
    this.document.selection.selectAll();
    this.paintSelection();
    this.focusGlobalInput();
  }
  async selectRange(selection: DocumentSelection) {
    await this.reveal(selection.headByte, true);
    this.document.selection.set(selection);
    this.globalSelection = true;
    this.paintSelection();
    this.focusGlobalInput();
  }
  private clearSelectionPaint() {
    this.target
      .querySelectorAll('.markdown-session-selection')
      .forEach((element) => element.remove());
  }
  private paintSelection() {
    this.clearSelectionPaint();
    const range = this.document.selection.range;
    for (const view of this.mounted) {
      const selection = {
        anchorByte: Math.max(range.fromByte, view.window.fromByte),
        headByte: Math.min(range.toByte, view.window.toByte),
      };
      if (selection.anchorByte > selection.headByte) continue;
      if (selection.anchorByte === selection.headByte) continue;
      const rects =
        view.source?.sourceRangeRects(selection.anchorByte, selection.headByte) ??
        view.semantic?.core.sourceRangeRects(selection.anchorByte, selection.headByte) ??
        [];
      const bounds = view.element.getBoundingClientRect();
      for (const rect of rects) {
        const mark = globalThis.document.createElement('span');
        mark.className = 'markdown-session-selection';
        mark.setAttribute('aria-hidden', 'true');
        Object.assign(mark.style, {
          top: `${(rect.top - bounds.top) / this.scale}px`,
          left: `${(rect.left - bounds.left) / this.scale}px`,
          width: `${rect.width / this.scale}px`,
          height: `${rect.height / this.scale}px`,
        });
        view.element.append(mark);
      }
    }
  }
  private schedule() {
    if (this.disposed || this.hidden || this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      void this.render().catch((error) => {
        if (
          error instanceof DocumentConflictError ||
          (error as { code?: string })?.code === 'revision-conflict'
        )
          this.schedule();
        else this.callbacks.onFailure(error);
      });
    });
  }
  private async render() {
    if (this.disposed || this.hidden) return;
    if (this.busy) {
      this.again = true;
      await this.renderDone;
      return;
    }
    if (this.document.session.state.pendingTransactions) {
      if (!this.retryTimer)
        this.retryTimer = setTimeout(() => {
          this.retryTimer = undefined;
          this.schedule();
        }, 100);
      return;
    }
    this.busy = true;
    let finish!: () => void;
    this.renderDone = new Promise((resolve) => {
      finish = resolve;
    });
    const generation = this.generation;
    const created: Mounted[] = [];
    try {
      await this.document.session.awaitTransactions();
      const state = this.document.state;
      const rect = this.target.getBoundingClientRect();
      this.scale = this.target.offsetWidth > 0 ? rect.width / this.target.offsetWidth || 1 : 1;
      this.contentTop = rect.width || rect.height
        ? rect.top - this.scroll.getBoundingClientRect().top + this.scroll.scrollTop : 0;
      const index = state.index?.revision === state.status.revision ? state.index : null;
      // 编辑后索引暂时失效时保留原视图，不能把尾部光标退回首屏。
      if (!index && this.mounted.length) return;
      const style = getComputedStyle(this.target);
      const context = `${this.target.clientWidth}|${style.font}|${style.lineHeight}|${style.letterSpacing}|${this.scale}|${this.geometryVersion}|${this.assetVersion}`;
      const geometryChanged = context !== this.layoutContext;
      const previousAnchor = geometryChanged ? (this.readingAnchor ?? this.captureAnchor()) :
        index !== this.index ? this.captureAnchor() : undefined;
      this.layoutContext = context;
      const previousLength = this.heights.length;
      const structureChanged = index !== this.index || (index?.blocks.length ?? 0) !== previousLength || index?.blocks.at(-1) !== this.lastIndexedBlock;
      if (index !== this.index || (index?.blocks.length ?? 0) < previousLength || geometryChanged) {
        this.index = index;
        this.heights = new MarkdownHeightIndex();
      }
      const heightFor = (block: MarkdownStructureIndex['blocks'][number]) => this.layoutCache.get(this.blockLayoutKey(block))?.height ?? estimateMarkdownBlockHeight(block);
      if (previousLength && this.heights.length === previousLength && index?.blocks[previousLength - 1] !== this.lastIndexedBlock)
        this.heights.update(previousLength - 1, heightFor(index!.blocks[previousLength - 1]));
      for (let i = this.heights.length; i < (index?.blocks.length ?? 0); i++) this.heights.append(heightFor(index!.blocks[i]));
      this.lastIndexedBlock = index?.blocks.at(-1);
      if (structureChanged) {
        this.groups = this.grouping.update(index?.blocks ?? []);
        this.groupById = new Map(this.groups.map(group => [group.id, group]));
        this.blockPositions = new Map(index?.blocks.map((block, i) => [block.id, i]) ?? []);
      }
      if (geometryChanged) for (const view of this.mounted) this.dirty.add(view);
      this.restoreAnchor(previousAnchor);
      const requestedByte = this.preferredByte ?? this.scrollByte;
      if (requestedByte !== undefined && index) {
        const blockIndex = this.blockPositions.get(index.blockAt(requestedByte)?.id ?? '') ?? -1;
        if (blockIndex >= 0) {
          const block = index.blocks[blockIndex];
          const fraction =
            block.mode === 'raw' || this.mode === 'source'
              ? (requestedByte - block.fromByte) / Math.max(1, block.toByte - block.fromByte)
              : 0;
          this.scrollTo(this.heights.top(blockIndex) + this.heights.height(blockIndex) * fraction);
          this.scrollByte = undefined;
        }
      }
      const plans: Array<{
        groupId?: string;
        signature?: string;
        from: number;
        to: number;
        fromByte: number;
        toByte: number;
        semantic: boolean;
        top: number;
        height: number;
      }> = [];
      const unindexed = index && index.progress.state !== 'ready' && (
        (this.preferredByte ?? 0) > index.progress.indexedBytes || this.localScrollTop > this.heights.totalHeight);
      if (unindexed) {
        const estimatedByte = this.preferredByte ?? Math.floor(this.localScrollTop / 24 * 100);
        const fromByte = Math.min(Math.max(0, state.status.byteLength - 16 * 1024), Math.max(0, estimatedByte - 8192));
        plans.push({ from: 0, to: 0, fromByte, toByte: Math.min(state.status.byteLength, fromByte + 16 * 1024), semantic: false, top: fromByte / 100 * 24, height: 16 * 1024 / 100 * 24 });
      } else if (index?.blocks.length) {
        const selected =
          this.preferredByte === undefined ? this.selection() : { headByte: this.preferredByte };
        const pinned =
          this.target.contains(globalThis.document.activeElement) && selected
            ? [
                this.blockPositions.get(index.blockAt(selected.headByte)?.id ?? '') ?? -1,
              ]
            : [];
        const height = (this.scroll.clientHeight || 800) / this.scale;
        const top = this.localScrollTop;
        const demand = this.demand.range(height, performance.now());
        const pressure = this.mounted.length > MARKDOWN_VIEWPORT_POLICY.softViews ||
          this.mounted.reduce((sum, view) => sum + view.window.toByte - view.window.fromByte, 0) > MARKDOWN_VIEWPORT_POLICY.softWindowBytes;
        if (pressure) { demand.before = demand.after = height * 3; demand.retainBefore = demand.retainAfter = height * 3; }
        const from = this.heights.indexAt(Math.max(0, top - demand.before));
        const to = this.heights.indexAt(top + height + demand.after) + 1;
        if (state.mountMode === 'progressive-full') {
          this.expanded = Math.min(
            index.blocks.length,
            Math.max(this.expanded, to) + 16,
          );
        }
        const retained = new Set(this.mounted.filter(view => this.pinned(view) || (demand.scrolling && !pressure) ||
          (view.from !== undefined && this.heights.top(view.to!) >= top - demand.retainBefore &&
            this.heights.top(view.from) <= top + height + demand.retainAfter)).map(view => view.groupId));
        // 二分定位空间候选；只额外查找已挂载/交互保护组，不在滚动热路径遍历全文块。
        const candidateIds = new Set(retained);
        const wanted = state.mountMode === 'progressive-full' ? 0 : from;
        let low = 0, high = this.groups.length;
        while (low < high) { const middle = (low + high) >>> 1;
          if (this.groups[middle].to <= wanted) low = middle + 1; else high = middle; }
        const end = state.mountMode === 'progressive-full' ? this.expanded : to;
        const candidates = new Map<string, MarkdownBlockGroup>();
        for (let i = low; i < this.groups.length && this.groups[i].from < end; i++) candidates.set(this.groups[i].id, this.groups[i]);
        for (const group of this.mountedGroups(candidateIds, pinned)) candidates.set(group.id, group);
        for (const range of [...candidates.values()].sort((a, b) => a.from - b.from)) {
          const first = index.blocks[range.from];
          const last = index.blocks[range.to - 1];
          const semantic = this.mode === 'semantic' && first.mode === 'semantic';
          let fromByte = first.fromByte;
          let toByte = last.toByte;
          if (!semantic && toByte - fromByte > 32 * 1024) {
            const fraction = Math.max(
              0,
              Math.min(
                1,
                (this.localScrollTop - this.heights.top(range.from)) /
                  Math.max(1, this.heights.height(range.from)),
              ),
            );
            fromByte = Math.max(
              fromByte,
              Math.floor(fromByte + fraction * (toByte - fromByte) - 8192),
            );
            toByte = Math.min(toByte, fromByte + 32 * 1024);
          }
          const wholeHeight = this.heights.top(range.to) - this.heights.top(range.from);
          const wholeBytes = Math.max(1, last.toByte - first.fromByte);
          plans.push({
            ...range,
            groupId: range.id,
            signature: JSON.stringify(index.blocks.slice(range.from, range.to).map(block =>
              [block.id, block.contentVersion ?? block.version, block.sourceHash,
                block.references.map(key => index.references[key])])),
            fromByte,
            toByte,
            semantic,
            top:
              this.heights.top(range.from) +
              (wholeHeight * (fromByte - first.fromByte)) / wholeBytes,
            height: (wholeHeight * (toByte - fromByte)) / wholeBytes,
          });
        }
      } else {
        const fromByte = Math.max(0, (this.preferredByte ?? 0) - 8192);
        plans.push({
          from: 0,
          to: 0,
          fromByte,
          toByte: Math.min(state.status.byteLength, fromByte + 16 * 1024),
          semantic: this.mode === 'semantic' && state.status.byteLength === 0,
          top: (fromByte / 100) * 24,
          height: 24,
        });
      }
      const next: Mounted[] = [];
      const admitted: typeof plans = [];
      // 保护窗口原位保留；邻接新组不能与其覆盖范围重叠。
      for (const view of this.mounted.filter(view => this.pinned(view))) {
        for (let i = plans.length - 1; i >= 0; i--) if (plans[i].fromByte < view.window.toByte && plans[i].toByte > view.window.fromByte) plans.splice(i, 1);
        const from = index ? this.blockPositions.get(index.blockAt(view.window.fromByte)?.id ?? '') ?? 0 : 0;
        const last = index ? this.blockPositions.get(index.blockAt(Math.max(view.window.fromByte, view.window.toByte - 1))?.id ?? '') ?? from : from;
        plans.push({ groupId: view.groupId, signature: view.signature, from, to: last + 1,
          fromByte: view.window.fromByte, toByte: view.window.toByte, semantic: !!view.semantic,
          top: this.heights.top(from), height: this.heights.top(last + 1) - this.heights.top(from) });
      }
      const viewportTop = this.localScrollTop;
      const bottom = viewportTop + (this.scroll.clientHeight || 800) / this.scale;
      const distance = (plan: typeof plans[number]) => Math.max(0, plan.top - bottom, viewportTop - plan.top - plan.height);
      plans.sort((a, b) => distance(a) - distance(b));
      const batchStart = performance.now();
      let createdCount = 0;
      const focused = this.focusAfterRemount || this.target.contains(globalThis.document.activeElement);
      const selected = this.globalSelection
        ? this.document.selection.selection
        : (this.selection() ?? this.document.selection.selection);
      for (const plan of plans) {
        const key = `${plan.groupId ?? `${plan.fromByte}:${plan.toByte}`}:${plan.semantic}`;
        let view = this.mounted.find(
          (view) =>
            (view.key === key || !plan.groupId) &&
            (this.pinned(view) || (view.signature === plan.signature && view.window.fromByte === plan.fromByte &&
              view.window.toByte === plan.toByte &&
              !!view.semantic === plan.semantic &&
              !(view.source?.bridge.isStale || view.semantic?.bridge.isStale))),
        );
        if (!view) {
          // 软预算优先释放远处保留带；可见区、基础缓冲和交互保护不被预算挤掉。
          if (distance(plan) > ((this.scroll.clientHeight || 800) / this.scale) * 3 &&
            (next.length >= MARKDOWN_VIEWPORT_POLICY.softViews ||
             next.reduce((sum, view) => sum + view.window.toByte - view.window.fromByte, 0) + plan.toByte - plan.fromByte > MARKDOWN_VIEWPORT_POLICY.softWindowBytes)) continue;
          if (createdCount && distance(plan) > 0 && performance.now() - batchStart >= MARKDOWN_VIEWPORT_POLICY.batchMs) {
            this.again = true;
            continue;
          }
          const started = performance.now();
          const window = await this.document.port.readWindow(
            state.status.documentId,
            state.status.revision,
            plan.fromByte,
            plan.toByte,
          );
          if (
            this.disposed ||
            generation !== this.generation ||
            this.document.session.state.pendingTransactions ||
            this.document.state.status.revision !== state.status.revision
          )
            return;
          const element = globalThis.document.createElement('div');
          element.className = 'markdown-session-window';
          view = { key, element, window, groupId: plan.groupId, signature: plan.signature };
          created.push(view);
          const stale = () => this.schedule();
          if (plan.semantic) {
            view.semantic = new MarkdownSemanticWindow(this.document.session, window, {
              ...this.options,
              target: element,
              references: index?.references,
              onStale: stale,
              onHistory: () => {
                void this.reveal(this.document.selection.selection.headByte, true);
              },
              onFailure: this.callbacks.onFailure,
            });
          } else {
            view.source = new MarkdownSourceWindow(this.document.session, window, {
              target: element,
              onStale: stale,
              onHistory: () => {
                void this.reveal(this.document.selection.selection.headByte, true);
              },
              emojiSuggestions: index?.blocks[plan.from]?.mode === 'semantic',
              onFailure: this.callbacks.onFailure,
              onSelectAll: () => {
                this.selectAll();
                return true;
              },
            });
          }
          const elapsed = performance.now() - started;
          this.demand.prepared(elapsed);
          this.stats.maxPreparationMs = Math.max(this.stats.maxPreparationMs, elapsed);
          if (elapsed > MARKDOWN_VIEWPORT_POLICY.batchMs) this.stats.overBudget++;
          createdCount++;
          this.stats.created++;
          view.semantic?.core.prepareTableViewport(this.scroll,
            (this.scroll.clientHeight || 800) * MARKDOWN_VIEWPORT_POLICY.forwardScreens);
          const saved = this.layoutCache.get(`${key}:${plan.signature ?? state.status.revision}:${this.layoutContext}`);
          if (saved?.tables && view.semantic?.core.restoreTableLayout(saved.tables)) this.stats.tableRestores++;
          this.dirty.add(view);
          this.observer?.observe(view.element);
        }
        view.from = plan.from;
        view.to = plan.to;
        view.layoutKey = `${key}:${plan.signature ?? state.status.revision}:${this.layoutContext}`;
        view.element.dataset.markdownGroup = plan.groupId ?? key;
        next.push(view);
        admitted.push(plan);
        if (view.semantic && index) {
          let low = 0, high = index.outline.length;
          while (low < high) { const mid = (low + high) >>> 1;
            if (index.outline[mid].byte < view.window.fromByte) low = mid + 1; else high = mid; }
          const headings = [];
          for (let i = low; i < index.outline.length && index.outline[i].byte < view.window.toByte; i++) headings.push(index.outline[i]);
          view.semantic.core.updateHeadingAnchors(headings.map(heading => heading.id));
        }
      }
      const ordered = next.map((view, i) => ({ view, plan: admitted[i] })).sort((a, b) => a.plan.top - b.plan.top);
      next.splice(0, next.length, ...ordered.map(item => item.view));
      plans.splice(0, plans.length, ...ordered.map(item => item.plan));
      if (generation !== this.generation || this.disposed || this.hidden) return;
      // 所有读取成功后才替换旧视图；过期读取不得清空可编辑内容。
      const layout = JSON.stringify([
        plans.map((plan) => [plan.top, plan.height]),
        this.heights.totalHeight,
        index?.progress.state,
      ]);
      const unchanged =
        next.length === this.mounted.length &&
        next.every((view, i) => view === this.mounted[i]) &&
        layout === this.installedLayout;
      if (unchanged) {
        this.finishPreparation();
        this.measureDirty();
        this.restorePreferred();
        return;
      }
      this.installedLayout = layout;
      this.stats.batches++;
      this.stats.ranges++;
      const anchor = this.captureAnchor();
      for (const view of this.mounted) if (!next.includes(view)) this.destroyView(view);
      const desired: Node[] = [];
      const neededSpacers = new Set<string>();
      const gap = (key: string, height: number) => {
        if (height <= 0) return;
        let spacer = this.spacers.get(key);
        if (!spacer) { spacer = this.spacer(height); this.spacers.set(key, spacer); }
        if (spacer.style.height !== `${height}px`) spacer.style.height = `${height}px`;
        neededSpacers.add(key);
        desired.push(spacer);
      };
      let top = 0;
      for (let i = 0; i < next.length; i++) {
        const plan = plans[i];
        gap(next[i].key, plan.top - top);
        desired.push(next[i].element);
        top = plan.top + plan.height;
      }
      const indexedHeight = this.heights.totalHeight;
      const total =
        index?.progress.state === 'ready'
          ? indexedHeight
          : Math.max(indexedHeight, (state.status.byteLength / 100) * 24);
      gap('tail', total - top);
      for (const [key, spacer] of this.spacers) if (!neededSpacers.has(key)) { spacer.remove(); this.spacers.delete(key); }
      if (this.input) desired.push(this.input);
      if (this.preparationIndicator) desired.push(this.preparationIndicator);
      let cursor = this.target.firstChild;
      for (const node of desired) {
        if (node === cursor) cursor = cursor.nextSibling;
        else {
          if (node.parentNode === this.target && node instanceof HTMLElement && node.classList.contains('markdown-session-window')) this.stats.moved++;
          this.target.insertBefore(node, cursor);
        }
      }
      this.mounted = next;
      this.finishPreparation();
      this.stats.peakMounted = Math.max(this.stats.peakMounted, next.length);
      this.restoreAnchor(anchor);
      this.measureDirty();
      this.active =
        next.find(
          (view) =>
            selected &&
            selected.headByte >= view.window.fromByte &&
            selected.headByte <= view.window.toByte,
        ) ?? next[0];
      if (this.active && selected && !this.pinned(this.active) && created.includes(this.active)) {
        const selection = {
          anchorByte: Math.max(
            this.active.window.fromByte,
            Math.min(selected.anchorByte, this.active.window.toByte),
          ),
          headByte: Math.max(
            this.active.window.fromByte,
            Math.min(selected.headByte, this.active.window.toByte),
          ),
        };
        this.active.source?.restoreSelection(selection);
        this.active.semantic?.core.restoreSourceSelection(selection);
      }
      if (
        focused ||
        !globalThis.document.activeElement ||
        globalThis.document.activeElement === globalThis.document.body
      )
        this.callbacks.onActive(this.active?.semantic?.core, this.active?.source);
      if (focused && (this.focusAfterRemount || this.target.contains(globalThis.document.activeElement)) &&
        this.active && created.includes(this.active)) this.focus();
      if (this.globalSelection) this.paintSelection();
      this.document.noteVisible('readable');
      this.document.noteVisible('editable');
      if (state.mountMode === 'progressive-full' && this.expanded < (index?.blocks.length ?? 0))
        this.again = true;
      this.restorePreferred();
      if (performance.now() - this.demand.lastScroll >= MARKDOWN_VIEWPORT_POLICY.quietMs)
        this.readingAnchor = this.captureAnchor();
    } finally {
      for (const view of created) if (!this.mounted.includes(view)) this.destroyView(view);
      this.busy = false;
      finish();
      if (this.again) {
        this.again = false;
        markdownWorkScheduler.enqueue(this, 'mount', MarkdownTaskPriority.NearViewport, () => this.schedule());
      }
    }
  }
  private spacer(height: number) {
    const element = globalThis.document.createElement('div');
    element.setAttribute('aria-hidden', 'true');
    element.contentEditable = 'false';
    element.className = 'markdown-session-spacer';
    element.style.height = `${Math.max(0, height)}px`;
    return element;
  }
  private destroyView(view: Mounted) {
    // 布局结果已由测量批次发布；回收只释放视图，避免 DOM 写入之间再同步读布局。
    this.observer?.unobserve(view.element);
    this.dirty.delete(view);
    view.semantic?.dispose();
    view.source?.dispose();
    view.element.remove();
    this.stats.destroyed++;
  }
  scrollToByte(byte: number) {
    this.requestPreparation();
    this.navigation++;
    this.generation++;
    this.preferredByte = undefined;
    this.scrollByte = byte;
    this.schedule();
  }
  async reveal(byte: number, focus = false) {
    this.requestPreparation();
    const navigation = ++this.navigation;
    this.generation++;
    await this.document.session.flushEdits();
    if (navigation !== this.navigation || this.disposed || this.hidden) return;
    this.globalSelection = false;
    this.removeInput();
    this.clearSelectionPaint();
    this.preferredByte = byte;
    this.preferredFocus = focus;
    if (this.busy) await this.renderDone;
    if (navigation !== this.navigation || this.disposed || this.hidden) return;
    await this.render();
    if (this.preferredByte !== undefined) this.schedule();
  }
  private restorePreferred() {
    const byte = this.preferredByte;
    if (byte === undefined) return;
    const view = this.mounted.find(
      (view) => byte >= view.window.fromByte && byte <= view.window.toByte,
    );
    if (view) {
      this.preferredByte = undefined;
      this.active = view;
      // readWindow 会修正 UTF-8 边界，导航到窗口边缘时使用其确认坐标。
      const selection = { anchorByte: byte, headByte: byte };
      try {
        view.source?.restoreSelection(selection);
        // 标题 byte 指向 Markdown 标记（如 #），不是可编辑文本光标；使用同一锚点的 PM 位置。
        const heading = this.index?.outline.find(heading => heading.byte === byte);
        if (!heading || !view.semantic?.core.revealHeadingAnchor(heading.id))
          view.semantic?.core.restoreSourceSelection(selection);
      } catch {
        const edge = { anchorByte: view.window.fromByte, headByte: view.window.fromByte };
        view.source?.restoreSelection(edge);
      }
      this.callbacks.onActive(view.semantic?.core, view.source);
      if (this.preferredFocus) this.focus();
    }
  }
  focus() {
    this.focusAfterRemount = true;
    cancelAnimationFrame(this.focusFrame);
    if (this.globalSelection) this.focusGlobalInput();
    else {
      this.active?.source?.view.focus();
      this.active?.semantic?.core.focus();
      const active = this.active;
      // 新 PM 视图首帧才启用 contenteditable，必须在其启用后补一次有条件的聚焦。
      if (active && !this.target.contains(globalThis.document.activeElement))
        this.focusFrame = requestAnimationFrame(() => {
          this.focusFrame = 0;
          if (!this.hidden && !this.disposed && this.focusAfterRemount && this.active === active) {
            active.source?.view.focus();
            active.semantic?.core.focus();
          }
        });
    }
  }
  updateOptions(options: Partial<EditorRuntimeOptions>) {
    this.options = { ...this.options, ...options };
    for (const view of this.mounted) view.semantic?.core.updateOptions(options);
  }
  updateTheme(theme: NonNullable<EditorCoreOptions['theme']>) {
    this.options.theme = theme;
    for (const view of this.mounted) view.semantic?.core.updateTheme(theme);
    this.invalidateGeometry();
  }
  async setHidden(hidden: boolean) {
    if (this.hidden === hidden) return;
    this.hidden = hidden;
    if (hidden) this.focusAfterRemount = false;
    if (hidden) cancelAnimationFrame(this.focusFrame);
    clearTimeout(this.preparationTimer); this.preparationTimer = undefined;
    this.preparationIndicator?.remove(); this.preparationIndicator = undefined;
    this.generation++;
    this.navigation++;
    this.layoutBatch.cancel(this);
    markdownWorkScheduler.cancel(this);
    if (hidden) await this.unmountViews();
    else this.schedule();
  }
  private async unmountViews() {
    await this.document.session.flushEdits();
    await this.renderDone;
    this.removeInput();
    for (const view of this.mounted) this.destroyView(view);
    this.mounted = [];
    this.active = undefined;
    this.callbacks.onActive(undefined, undefined);
    this.target.replaceChildren();
    this.spacers.clear();
  }
  async dispose() {
    if (this.disposed) return;
    this.hidden = true;
    this.generation++;
    await this.unmountViews();
    this.disposed = true;
    this.generation++;
    this.unsubscribe();
    this.typographySubscription.destroy();
    this.layoutBatch.cancel(this);
    markdownWorkScheduler.cancel(this);
    this.layoutCache.clear();
    this.grouping.clear();
    this.groups = [];
    this.groupById.clear();
    this.blockPositions.clear();
    this.observer?.disconnect();
    cancelAnimationFrame(this.frame);
    cancelAnimationFrame(this.focusFrame);
    this.detachInput();
    if (this.retryTimer) clearTimeout(this.retryTimer);
    clearTimeout(this.quietTimer);
    clearTimeout(this.preparationTimer);
    this.preparationIndicator?.remove(); this.preparationIndicator = undefined;
    this.scroll.removeEventListener('scroll', this.onScroll);
    this.target.removeEventListener('keydown', this.onKey, true);
    this.target.removeEventListener('keyup', this.onSelectionChanged, true);
    this.target.removeEventListener('copy', this.onCopy, true);
    this.target.removeEventListener('cut', this.onCopy, true);
    this.target.removeEventListener('beforeinput', this.onBeforeInput, true);
    this.target.removeEventListener('pointerdown', this.onPointer, true);
    globalThis.document.removeEventListener('pointerup', this.onPointerUp, true);
    globalThis.document.removeEventListener('pointercancel', this.onPointerUp, true);
    globalThis.document.removeEventListener('pointermove', this.onPointerMove, true);
    globalThis.document.removeEventListener('pointerdown', this.onOutsidePointer, true);
    globalThis.document.removeEventListener('focusin', this.onDocumentFocus, true);
    this.target.removeEventListener('input', this.onInput, true);
    this.target.removeEventListener('focusin', this.onFocus, true);
    this.target.removeEventListener(TABLE_LAYOUT_READY_EVENT, this.onTableLayout);
    this.target.removeEventListener('load', this.onAsset, true);
    this.target.ownerDocument.fonts?.removeEventListener('loadingdone', this.onFont);
  }
}
