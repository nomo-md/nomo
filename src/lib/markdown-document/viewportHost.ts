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
import { getOutlineViewportOffset } from '../outline/outlineViewport';
import { setNavigationRenderContext, type NavigationRenderContext, type NavigationEnhancement } from '../editor-core/nodeViews/renderVisibility';

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
  restricted?: boolean;
  restrictedByte?: number;
  measured?: boolean;
}
export interface OutlineNavigationClock { requestedAt: number; startedAt?: number; }
interface OutlineNavigationSample {
  revision: number; byte: number; duration?: number; inputGateMs: number;
  kind: 'animated' | 'direct'; windowLimited: boolean; targetLoaded: boolean;
  requestedAt: number; positionedMs?: number;
  firstMotionMs?: number; animationMs?: number; basicReadyMs?: number; enhancementMs?: number;
  readMs: number; buildMs: number; measureMs: number; created: number; pendingEnhancements: number;
  deferred: Record<NavigationEnhancement, number>;
  deadlineMiss?: boolean; status: 'preparing' | 'ready' | 'late' | 'cancelled' | 'failed' | 'restricted';
}
interface ReadingAnchor { id: string; offset: number; screen?: number; fallback: number; generation: number; }
interface OutlineNavigation {
  kind: 'animated' | 'direct';
  originAnchor?: ReadingAnchor; originGroups?: Set<string>;
  overflowAnchor: string;
  notifiedGoal?: number;
  id: string; byte: number; revision: number; aligned: boolean;
  startedAt: number; goal: number; elapsed: number; initialTop: number;
  frame: number; animationDone: boolean; ready: boolean; recorded: boolean;
  firstFrame?: Promise<void>; motionStarted?: () => void;
  controller: AbortController; context: NavigationRenderContext;
  sample: OutlineNavigationSample; settle?: () => void;
}
export interface MarkdownPresentation {
  documentId: string;
  revision: number;
  generation: number;
  mode: 'semantic' | 'source';
  /** 所有时间均相对于本次会话打开；started 是当前目标开始时刻。 */
  started: number;
  phase: 'preparing' | 'preview' | 'editable' | 'restricted' | 'failed';
  coverage: 'pending' | 'semantic' | 'mixed' | 'source';
  /** 生命周期代理计时，不能代替实机实际绘制测量。每个目标只保留当前记录。 */
  timings: { preview?: number; editable?: number; covered?: number; stable?: number; indexed?: number };
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
  private locatedByte: number | undefined;
  // 只保留一次大纲导航的语义目标；资源重排后继续校准，用户接管或修订变化即释放。
  private outlineNavigation?: OutlineNavigation;
  private readonly navigationSamples: OutlineNavigationSample[] = [];
  private interruptRead?: () => void;
  private publishedOutlineId = '';
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
  private stableTimer?: ReturnType<typeof setTimeout>;
  private presentationRevision = -1;
  private presentationGeneration = -1;
  private presentationStarted = 0;
  private presentationTimings: MarkdownPresentation['timings'] = {};
  private presentationSignature = '';
  private visibleGeometry = '';
  private viewFailure: unknown;
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
      onNavigationIntent?(): void;
      onPresentation?(state: MarkdownPresentation): void;
      onOpenSource?(range: { fromByte: number; toByte: number; byte: number }): void;
    },
  ) {
    this.scroll = target.closest<HTMLElement>('.semantic-pane, .source-pane') ?? target;
    this.presentationStarted = document.elapsed;
    this.presentationRevision = document.state.status.revision;
    this.presentationGeneration = this.navigation;
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
    this.scroll.addEventListener('wheel', this.onNavigationIntent, { passive: true });
    this.scroll.addEventListener('touchstart', this.onNavigationIntent, { passive: true });
    this.scroll.addEventListener('pointerdown', this.onNavigationIntent, true);
    target.addEventListener('keydown', this.onKey, true);
    target.addEventListener('keyup', this.onSelectionChanged, true);
    target.addEventListener('copy', this.onCopy, true);
    target.addEventListener('cut', this.onCopy, true);
    target.addEventListener('beforeinput', this.onBeforeInput, true);
    target.addEventListener('paste', this.guardWrite, true);
    target.addEventListener('drop', this.guardWrite, true);
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
          if (view) this.markDirty(view);
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
    this.unsubscribe = document.subscribe(() => {
      if (this.outlineNavigation && this.outlineNavigation.revision !== document.state.status.revision)
        this.cancelOutlineNavigation();
      for (const view of this.mounted) {
        view.semantic?.setFailure(!!document.state.failure || !!this.viewFailure);
        view.source?.setReadonly(!!this.options.readonly || !!document.state.failure || !!this.viewFailure);
      }
      this.schedule();
    });
    this.requestPreparation();
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
  get writable() {
    const state = this.document.session.state;
    if (this.hidden || this.disposed || this.options.readonly || this.viewFailure ||
      this.document.state.failure || state.historyBusy || state.closing || state.failure) return false;
    return !!(this.active?.semantic?.editable ||
      (this.active?.source && !this.active.source.bridge.isStale));
  }
  private guardWrite = (event: Event) => {
    const view = this.mounted.find(view => view.element.contains(event.target as Node));
    const allowed = this.writable && (!view?.semantic || view.semantic.editable);
    if (allowed) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  private visibleViews() {
    const top = this.localScrollTop;
    const bottom = top + (this.scroll.clientHeight || 800) / this.scale;
    return this.mounted.filter(view => view.from !== undefined && view.to! > view.from
      ? this.heights.top(view.from) < bottom && this.heights.top(view.to!) > top
      : this.document.state.status.byteLength === 0 || this.hasVisibleContent());
  }
  get presentation(): MarkdownPresentation {
    const state = this.document.state;
    const visible = this.visibleViews();
    const target = this.preferredByte ?? this.scrollByte;
    const atTarget = target === undefined || visible.some(view => target >= view.window.fromByte && target <= view.window.toByte);
    let through = this.localScrollTop;
    for (const view of visible) {
      if (view.from === undefined || view.to! <= view.from) continue;
      if (this.heights.top(view.from) > through + 1) break;
      through = Math.max(through, this.heights.top(view.to!));
    }
    const bottom = this.localScrollTop + (this.scroll.clientHeight || 800) / this.scale;
    const covered = visible.length > 0 && atTarget && (state.status.byteLength === 0 ||
      through >= bottom - 1 || (state.progress.state === 'ready' && through >= this.heights.totalHeight - 1));
    const semantic = visible.some(view => !!view.semantic);
    return {
      documentId: state.status.documentId, revision: state.status.revision,
      generation: this.navigation, mode: this.mode, started: this.presentationStarted,
      phase: state.failure || this.viewFailure ? 'failed' : !visible.length || !atTarget ? 'preparing' :
        visible.some(view => view.semantic?.editable || (this.mode === 'source' && view.source)) ? 'editable' :
          semantic ? 'preview' : 'restricted',
      coverage: !covered ? 'pending' : this.mode === 'source' ? 'source' :
        visible.every(view => !!view.semantic) ? 'semantic' : 'mixed',
      timings: { ...this.presentationTimings, indexed: state.timings.indexed },
    };
  }
  private publishPresentation() {
    if (this.hidden || this.disposed) return;
    if (this.input) this.input.readOnly = !this.writable;
    if (this.presentationRevision !== this.document.state.status.revision || this.presentationGeneration !== this.navigation) {
      this.presentationRevision = this.document.state.status.revision;
      this.presentationGeneration = this.navigation;
      this.presentationStarted = this.document.elapsed;
      this.presentationTimings = {};
    }
    const state = this.presentation;
    const visible = this.visibleViews();
    const geometry = JSON.stringify([this.localScrollTop, this.scroll.clientHeight, state.coverage,
      visible.map(view => [view.key, view.from === undefined ? 0 : this.heights.top(view.from),
        view.to === undefined ? 0 : this.heights.top(view.to)])]);
    if (geometry !== this.visibleGeometry) { this.visibleGeometry = geometry; this.invalidateStable(); }
    const elapsed = this.document.elapsed;
    const semantic = visible.some(view => !!view.semantic);
    if (semantic && state.phase !== 'preparing' && state.phase !== 'failed') {
      this.presentationTimings.preview ??= elapsed;
      this.document.noteVisible('readable');
      if (state.phase === 'editable') {
        this.presentationTimings.editable ??= elapsed;
        this.document.noteVisible('editable');
      }
    }
    if (state.coverage !== 'pending') this.presentationTimings.covered ??= elapsed;
    if (!this.stableTimer && state.coverage !== 'pending' && this.presentationTimings.stable === undefined)
      this.stableTimer = setTimeout(() => {
        this.stableTimer = undefined;
        if (this.hidden || this.disposed || this.presentation.coverage === 'pending') return;
        this.presentationTimings.stable = this.document.elapsed;
        this.publishPresentation();
      }, MARKDOWN_VIEWPORT_POLICY.quietMs);
    const next = this.presentation;
    const signature = JSON.stringify(next);
    if (signature !== this.presentationSignature) {
      this.presentationSignature = signature;
      this.target.dataset.presentation = next.phase;
      this.callbacks.onPresentation?.(next);
    }
  }
  private invalidateStable() {
    clearTimeout(this.stableTimer); this.stableTimer = undefined;
    this.presentationTimings.stable = undefined;
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
    const restricted = this.mounted.find(view => view.restricted && view.from !== undefined &&
      this.index?.blocks[view.from]?.id === first?.id);
    return {
      byteOffset: this.locatedByte ?? restricted?.restrictedByte ?? first?.fromByte ?? this.active?.window.fromByte ?? 0,
      line: first?.fromLine ?? 1,
    };
  }
  /** 章节高亮读取参考线的真实几何位置，不使用 locatedByte 的导航意图。 */
  get activeOutlineId(): string {
    if (this.outlineNavigation && !this.outlineNavigation.aligned) return this.outlineNavigation.id;
    const headings = this.index?.outline;
    if (!headings?.length) return '';
    if (this.outlineNavigation && !this.outlineNavigation.aligned) return this.outlineNavigation.id;
    const bounds = this.scroll.getBoundingClientRect();
    const reference = bounds.top + getOutlineViewportOffset(this.scroll.clientHeight);
    let low = 0, high = headings.length;
    // 未挂载标题使用高度索引，已挂载标题使用实际坐标；滚动路径不遍历全文标题。
    while (low < high) {
      const middle = (low + high) >>> 1;
      const heading = headings[middle];
      const block = this.blockPositions.get(heading.blockId);
      const top = this.headingTop(heading.byte, heading.id) ?? (block === undefined ? Infinity :
        bounds.top + this.contentTop + this.heights.top(block) * this.scale - this.scroll.scrollTop);
      if (top <= reference + 1) low = middle + 1;
      else high = middle;
    }
    // 文末无法继续滚动时，保留明确点击且仍可见的标题，直到用户接管。
    const target = this.outlineNavigation;
    if (target && this.scroll.scrollTop >= this.scroll.scrollHeight - this.scroll.clientHeight - 1) {
      const top = this.headingTop(target.byte, target.id);
      if (top !== undefined && top > reference && top < bounds.bottom) return target.id;
    }
    return headings[Math.max(0, low - 1)].id;
  }
  private headingTop(byte: number, id: string): number | undefined {
    const view = this.mounted.find(view => byte >= view.window.fromByte && byte < view.window.toByte);
    if (view?.restricted) return view.element.getBoundingClientRect().top;
    return view?.semantic?.core.headingAnchorTop(id) ?? view?.source?.byteTop(byte);
  }
  cancelOutlineNavigation() {
    const target = this.outlineNavigation;
    if (!target) return false;
    this.outlineNavigation = undefined;
    this.interruptRead?.();
    cancelAnimationFrame(target.frame);
    target.motionStarted?.(); target.motionStarted = undefined;
    target.controller.abort();
    this.scroll.style.overflowAnchor = target.overflowAnchor;
    setNavigationRenderContext(this.target, undefined);
    if (target.sample.status === 'preparing') target.sample.status = 'cancelled';
    this.recordNavigation(target);
    target.settle?.();
    if (this.preferredByte === target.byte) this.preferredByte = undefined;
    this.locatedByte = undefined;
    this.preferredFocus = false;
    this.navigation++;
    this.generation++;
    return true;
  }
  private onNavigationIntent = () => {
    if (this.cancelOutlineNavigation()) this.callbacks.onNavigationIntent?.();
  };
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
    if (view?.restricted) {
      const byte = view.restrictedByte ?? view.window.fromByte;
      return { anchorByte: byte, headByte: byte };
    }
    return view?.source?.selection ?? view?.semantic?.core.getSourceSelection(false);
  }
  private onSelectionChanged = () => {
    if (!this.composing) this.captureSelection();
  };
  private onInput = () => {
    this.onNavigationIntent();
    this.focusAfterRemount = this.target.contains(globalThis.document.activeElement);
    this.document.interacting();
  };
  private onOutsidePointer = (event: PointerEvent) => {
    if (event.target instanceof Node && !this.target.contains(event.target)) {
      this.focusAfterRemount = false;
      this.preferredFocus = false;
    }
  };
  private onDocumentFocus = (event: FocusEvent) => {
    // WebView2 在过期投影变为只读时会暂时把焦点交给 body；这不代表用户离开编辑区。
    if (event.target !== globalThis.document.body && event.target instanceof Node &&
      !this.target.contains(event.target)) { this.focusAfterRemount = false; this.preferredFocus = false; }
  };
  private onScroll = () => {
    if (this.programScroll !== undefined && Math.abs(this.scroll.scrollTop - this.programScroll) < 1) {
      if (!this.outlineNavigation || this.outlineNavigation.animationDone) this.schedule();
      return;
    }
    const target = this.outlineNavigation;
    if (target?.kind === 'direct' && !target.aligned) {
      // 实际输入已由前置意图事件取消；准备导致的浏览器几何调整仍保持原阅读锚点。
      this.programScroll = this.scroll.scrollTop;
      this.schedule();
      return;
    }
    const maximum = Math.max(0, this.scroll.scrollHeight - this.scroll.clientHeight);
    if (target && target.goal >= maximum && Math.abs(this.scroll.scrollTop - maximum) < 1) {
      // 文末占位实测缩短时浏览器会截断 scrollTop；真实输入已由前置意图事件取消。
      this.programScroll = this.scroll.scrollTop;
      this.schedule();
      return;
    }
    this.programScroll = undefined;
    this.onNavigationIntent();
    this.locatedByte = undefined;
    this.document.requestVisibleStructure(this, null);
    this.invalidateStable();
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
    if (this.preparationTimer || this.preparationIndicator || this.hidden || this.disposed) return;
    this.preparationStarted = performance.now();
    this.preparationTimer = setTimeout(() => {
      this.preparationTimer = undefined;
      if (this.hidden || this.disposed || this.hasVisibleContent()) return;
      const indicator = this.preparationIndicator = globalThis.document.createElement('div');
      indicator.className = 'markdown-session-preparing';
      indicator.setAttribute('role', 'status');
      indicator.textContent = '正在准备正文…';
      Object.assign(indicator.style, { position: 'absolute', top: `${this.localScrollTop + 12}px`,
        left: '12px', pointerEvents: 'none', fontSize: '0.875em', opacity: '0.7' });
      this.target.append(indicator);
      this.stats.preparationHints++;
    }, 100);
  }
  private hasVisibleContent() {
    if (this.document.state.status.byteLength === 0 && this.mounted.some(view => !!view.semantic || !!view.source)) return true;
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
  private showFailure() {
    if (this.hidden || this.disposed) return;
    if (this.outlineNavigation) this.outlineNavigation.sample.status = 'failed';
    this.cancelOutlineNavigation();
    this.document.requestVisibleStructure(this, null);
    clearTimeout(this.preparationTimer); this.preparationTimer = undefined;
    this.invalidateStable();
    for (const view of this.mounted) view.semantic?.setFailure(true);
    for (const view of this.mounted) view.source?.setReadonly(true);
    if (!this.preparationIndicator || this.preparationIndicator.dataset.failure !== 'true') {
      this.preparationIndicator?.remove();
      const element = this.preparationIndicator = globalThis.document.createElement('div');
      element.className = 'markdown-session-preparing';
      element.dataset.failure = 'true';
      element.setAttribute('role', 'alert');
      const message = globalThis.document.createElement('span');
      const error = this.viewFailure ?? this.document.state.failure;
      message.textContent = `正文准备失败：${error instanceof Error ? error.message : String(error)}`;
      const retry = globalThis.document.createElement('button');
      retry.type = 'button'; retry.textContent = '重试';
      retry.onclick = () => {
        this.viewFailure = undefined;
        element.remove(); this.preparationIndicator = undefined;
        void this.document.retry().then(() => {
          if (!this.hidden && !this.disposed) { this.requestPreparation(); this.schedule(); }
        }).catch(error => { this.viewFailure = error; this.showFailure(); });
      };
      element.append(message, retry); this.target.append(element);
      if (this.mode === 'semantic' && this.callbacks.onOpenSource && !this.document.session.state.failure) {
        const source = globalThis.document.createElement('button');
        source.type = 'button'; source.textContent = '在源码中打开';
        source.onclick = () => {
          if (this.hidden || this.disposed) return;
          const byte = this.preferredByte ?? this.scrollByte ?? this.position.byteOffset;
          this.callbacks.onOpenSource?.({ fromByte: byte, toByte: byte, byte });
        };
        element.append(source);
      }
    }
    this.publishPresentation();
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
      residentWindowBytes: this.mounted.reduce((sum, view) => sum + this.windowBytes(view), 0),
      cacheBytes: this.layoutCache.estimatedBytes, cacheHits: this.layoutCache.hits,
      cacheMisses: this.layoutCache.misses, preparationMs: this.demand.preparationMs,
      navigation: this.outlineNavigation ? this.copyNavigationSample(this.outlineNavigation.sample) : undefined,
      navigations: this.navigationSamples.map(sample => this.copyNavigationSample(sample)) };
  }
  private windowBytes(view: Mounted) { return view.restricted ? 0 : view.window.toByte - view.window.fromByte; }
  private invalidateGeometry() {
    this.invalidateStable();
    this.layoutContext = '';
    this.layoutCache.clear();
    for (const view of this.mounted) { view.layoutKey = undefined; this.markDirty(view); }
    this.schedule();
  }
  private onFont = () => { this.assetVersion++; this.invalidateGeometry(); };
  private onAsset = (event: Event) => {
    const view = this.mounted.find(view => event.target instanceof Node && view.element.contains(event.target));
    if (!view) return;
    this.invalidateStable();
    if (view.layoutKey) this.layoutCache.delete(view.layoutKey);
    if (this.index && view.from !== undefined && view.to !== undefined)
      for (let i = view.from; i < view.to; i++) this.layoutCache.delete(this.blockLayoutKey(this.index.blocks[i]));
    this.markDirty(view);
    this.measureDirty();
  };
  private onTableLayout = (event: Event) => {
    this.stats.tablePublications++;
    const view = this.mounted.find(view => event.target instanceof Node && view.element.contains(event.target));
    if (view) { this.markDirty(view); this.measureDirty(); }
  };
  private blockLayoutKey(block: MarkdownStructureIndex['blocks'][number]) {
    if (this.mode === 'semantic' && block.mode === 'raw') return `restricted:${block.id}:${this.layoutContext}`;
    return `${block.id}:${block.contentVersion ?? block.version}:${block.sourceHash}:${JSON.stringify(block.references.map(key => this.index?.references[key]))}:${this.layoutContext}`;
  }
  private captureAnchor(forDirect = false): ReadingAnchor | undefined {
    if (!forDirect && this.outlineNavigation && !this.outlineNavigation.aligned) return undefined;
    if ((!forDirect && this.preferredByte !== undefined && !this.outlineNavigation) || !this.index) return undefined;
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
  private restoreAnchor(value: unknown, forDirect = false) {
    if (this.outlineNavigation && !forDirect) return;
    const anchor = value as ReturnType<MarkdownViewportHost['captureAnchor']>;
    if (!anchor || anchor.generation !== this.generation ||
      (!forDirect && this.preferredByte !== undefined && !this.outlineNavigation)) return;
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
  private markDirty(view: Mounted) {
    view.measured = false;
    this.dirty.add(view);
    const target = this.outlineNavigation;
    if (target && !target.aligned && target.context.targets.has(view.element)) {
      target.ready = false;
      target.context.ready = false;
    }
  }
  private measureDirty() {
    if (!this.dirty.size || this.hidden || this.disposed) return;
    this.layoutBatch.enqueue(this, () => {
      const started = performance.now();
      const values = [...this.dirty].filter(view => this.mounted.includes(view)).map(view =>
        ({ view, height: view.element.offsetHeight, top: view.element.getBoundingClientRect().top,
          blocks: view.semantic?.core.measureStructureBlocks(), tables: view.semantic?.core.snapshotTableLayout() }));
      this.dirty.clear();
      this.stats.measurements += values.length;
      if (this.outlineNavigation && !this.outlineNavigation.aligned) this.outlineNavigation.sample.measureMs += performance.now() - started;
      return values;
    }, values => {
      for (const { view, height, top, blocks, tables } of values) {
        view.measured = height > 0;
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
      this.alignOutlineHeading();
      this.schedule();
    });
  }
  private hasOriginalSource(view: Mounted) {
    if (view.restricted) return true;
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
    this.focusAfterRemount = true;
    this.globalSelection = false;
    this.removeInput();
    this.clearSelectionPaint();
    this.pointerView = this.mounted.find((view) => view.element.contains(event.target as Node));
    this.pointerAnchor =
      this.byteAtCoords(this.pointerView, event) ?? this.selection(this.pointerView)?.headByte;
    if (this.pointerView) this.active = this.pointerView;
  };
  private byteAtCoords(view: Mounted | undefined, event: PointerEvent) {
    if (view?.restricted) return view.restrictedByte ?? view.window.fromByte;
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
    else if (this.focusAfterRemount && this.active?.semantic?.editable) this.focus();
    this.schedule();
  };
  private onCopy = (event: ClipboardEvent) => {
    if (event.type === 'cut' && !this.writable) { this.guardWrite(event); return; }
    if (!this.globalSelection) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const action =
      event.type === 'cut'
        ? this.document.selection.cut(this.callbacks.writeClipboard, undefined, () => this.writable)
        : this.document.selection.copy().then(this.callbacks.writeClipboard);
    void action.catch(this.callbacks.onFailure);
  };
  private onBeforeInput = (event: InputEvent) => {
    if (!this.writable) { this.guardWrite(event); return; }
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
    if (!this.writable) return;
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
    input.readOnly = !this.writable;
    input.className = 'markdown-session-input';
    input.style.top = `${this.localScrollTop}px`;
    input.setAttribute('aria-label', '文档选区输入');
    input.addEventListener('compositionstart', () => {
      if (!this.writable) return;
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
    this.onNavigationIntent();
    if (event.target instanceof HTMLElement && event.target.closest('button') &&
      ['Enter', ' '].includes(event.key)) return;
    if (this.active?.restricted && (event.ctrlKey || event.metaKey) && ['z', 'y'].includes(event.key.toLowerCase())) {
      event.preventDefault(); event.stopImmediatePropagation();
      const state = this.document.session.state;
      if (!this.options.readonly && !this.hidden && !state.failure && !this.document.state.failure && !state.closing && !state.historyBusy)
        void this.document.session.undo(event.key.toLowerCase() === 'y' || event.shiftKey)
          .then(() => this.reveal(this.document.selection.selection.headByte, true)).catch(this.callbacks.onFailure);
      return;
    }
    if (!this.writable && (['Backspace', 'Delete', 'Enter'].includes(event.key) ||
      (!event.ctrlKey && !event.metaKey && !event.altKey && (event.key.length === 1 || event.key === 'Process')) ||
      ((event.ctrlKey || event.metaKey) && ['v', 'x', 'z', 'y'].includes(event.key.toLowerCase())))) {
      event.preventDefault(); event.stopImmediatePropagation(); return;
    }
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
    if (this.active.restricted && ['ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown'].includes(event.key)) {
      event.preventDefault(); event.stopImmediatePropagation();
      const left = event.key === 'ArrowLeft' || event.key === 'ArrowUp';
      const edge = left ? this.active.window.fromByte : this.active.window.toByte;
      if ((left && edge === 0) || (!left && edge === this.document.state.status.byteLength)) return;
      void (left ? this.adjacentByte(edge, -1) : Promise.resolve(edge))
        .then(byte => this.navigate(byte, event.shiftKey)).catch(this.callbacks.onFailure);
      return;
    }
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
          if (!this.writable) return;
          this.document.selection.set({ anchorByte: edge, headByte: byte });
          await this.document.selection.replaceSelection('', () => this.writable);
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
        (view.restricted ? [view.element.getBoundingClientRect()] : undefined) ??
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
        else { this.viewFailure = error; this.showFailure(); this.callbacks.onFailure(error); }
      });
    });
  }
  private async render() {
    if (this.disposed || this.hidden) return;
    if (this.document.state.failure || this.viewFailure) { this.showFailure(); return; }
    for (const view of this.mounted) {
      view.semantic?.setFailure(false);
      view.source?.setReadonly(!!this.options.readonly);
    }
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
      if (this.disposed || this.hidden || generation !== this.generation) return;
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
      const appendOnly = index === this.index && (index?.blocks.length ?? 0) >= previousLength;
      const structureChanged = index !== this.index || (index?.blocks.length ?? 0) !== previousLength || index?.blocks.at(-1) !== this.lastIndexedBlock;
      if (index !== this.index || (index?.blocks.length ?? 0) < previousLength || geometryChanged) {
        this.index = index;
        this.heights = new MarkdownHeightIndex();
      }
      const heightFor = (block: MarkdownStructureIndex['blocks'][number]) => this.layoutCache.get(this.blockLayoutKey(block))?.height ??
        (this.mode === 'semantic' && block.mode === 'raw' ? 104 : estimateMarkdownBlockHeight(block));
      if (previousLength && this.heights.length === previousLength && index?.blocks[previousLength - 1] !== this.lastIndexedBlock)
        this.heights.update(previousLength - 1, heightFor(index!.blocks[previousLength - 1]));
      for (let i = this.heights.length; i < (index?.blocks.length ?? 0); i++) this.heights.append(heightFor(index!.blocks[i]));
      this.lastIndexedBlock = index?.blocks.at(-1);
      if (structureChanged) {
        const previousGroups = this.groups.length;
        this.groups = this.grouping.update(index?.blocks ?? []);
        if (appendOnly) {
          for (let i = Math.max(0, previousGroups - 1); i < this.groups.length; i++) this.groupById.set(this.groups[i].id, this.groups[i]);
          for (let i = Math.max(0, previousLength - 1); i < (index?.blocks.length ?? 0); i++) this.blockPositions.set(index!.blocks[i].id, i);
        } else {
          this.groupById = new Map(this.groups.map(group => [group.id, group]));
          this.blockPositions = new Map(index?.blocks.map((block, i) => [block.id, i]) ?? []);
        }
      }
      if (geometryChanged) for (const view of this.mounted) this.markDirty(view);
      this.restoreAnchor(previousAnchor);
      this.alignOutlineHeading();
      const navigating = this.outlineNavigation && !this.outlineNavigation.aligned;
      const requestedByte = this.preferredByte ?? this.scrollByte;
      let requestedTop = navigating ? Math.max(0, (this.outlineNavigation!.goal - this.contentTop) / this.scale) : undefined;
      if (requestedTop === undefined && requestedByte !== undefined && index) {
        const blockIndex = this.blockPositions.get(index.blockAt(requestedByte)?.id ?? '') ?? -1;
        if (blockIndex >= 0) {
          const block = index.blocks[blockIndex];
          const fraction =
            this.mode === 'source'
              ? (requestedByte - block.fromByte) / Math.max(1, block.toByte - block.fromByte)
              : 0;
          requestedTop = this.heights.top(blockIndex) + this.heights.height(blockIndex) * fraction;
          if (!this.outlineNavigation) this.scrollTo(requestedTop);
          if (this.mode === 'semantic' && block.mode === 'raw') this.preferredByte = requestedByte;
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
        restricted?: boolean;
        top: number;
        height: number;
      }> = [];
      const unindexed = index && index.progress.state !== 'ready' && (
        (this.preferredByte ?? this.scrollByte ?? 0) >= index.progress.indexedBytes || this.localScrollTop > this.heights.totalHeight);
      const needsStructure = this.mode === 'semantic' && state.progress.state !== 'ready' &&
        (!index?.blocks.length || unindexed || this.localScrollTop + (this.scroll.clientHeight || 800) / this.scale > this.heights.totalHeight);
      this.document.requestVisibleStructure(this, needsStructure ? {
        byte: Math.max(this.preferredByte ?? this.scrollByte ?? 0, index?.progress.indexedBytes ?? 0),
        revision: state.status.revision, generation: this.navigation,
      } : null);
      if (unindexed) {
        const estimatedByte = this.preferredByte ?? this.scrollByte ?? Math.floor(this.localScrollTop / 24 * 100);
        const fromByte = Math.min(Math.max(0, state.status.byteLength - 16 * 1024), Math.max(0, estimatedByte - 8192));
        if (this.mode === 'source') plans.push({ from: 0, to: 0, fromByte, toByte: Math.min(state.status.byteLength, fromByte + 16 * 1024), semantic: false, top: fromByte / 100 * 24, height: 16 * 1024 / 100 * 24 });
      } else if (index?.blocks.length) {
        const selected =
          this.outlineNavigation ? { headByte: this.outlineNavigation.byte } :
            this.preferredByte === undefined ? this.selection() : { headByte: this.preferredByte };
        const pinned =
          this.target.contains(globalThis.document.activeElement) && selected
            ? [
                this.blockPositions.get(index.blockAt(selected.headByte)?.id ?? '') ?? -1,
              ]
            : [];
        // 只额外保留目标所在组，沿途仍按可见窗口准备，不把两点间的正文全部挂载。
        const outlineBlock = this.outlineNavigation && this.blockPositions.get(index.blockAt(this.outlineNavigation.byte)?.id ?? '');
        if (outlineBlock !== undefined) pinned.push(outlineBlock);
        const height = (this.scroll.clientHeight || 800) / this.scale;
        // 空宿主尚无可滚动高度，浏览器会截断 scrollTop；计划仍应覆盖目标位置。
        const top = requestedTop ?? this.localScrollTop;
        const demand = this.demand.range(height, performance.now());
        if (navigating) demand.before = demand.after = 0;
        const pressure = this.mounted.length > MARKDOWN_VIEWPORT_POLICY.softViews ||
          this.mounted.reduce((sum, view) => sum + this.windowBytes(view), 0) > MARKDOWN_VIEWPORT_POLICY.softWindowBytes;
        if (pressure) { if (!navigating) demand.before = demand.after = height * 3; demand.retainBefore = demand.retainAfter = height * 3; }
        const from = this.heights.indexAt(Math.max(0, top - demand.before));
        const to = this.heights.indexAt(top + height + demand.after) + 1;
        if (state.mountMode === 'progressive-full' && !navigating) {
          this.expanded = Math.min(
            index.blocks.length,
            Math.max(this.expanded, to) + 16,
          );
        }
        const retained = new Set(this.mounted.filter(view => this.pinned(view) || (demand.scrolling && !pressure) ||
          (navigating && view.groupId && this.outlineNavigation?.originGroups?.has(view.groupId)) ||
          (this.outlineNavigation && !this.outlineNavigation.aligned && view.from !== undefined &&
            this.heights.top(view.to!) > this.localScrollTop && this.heights.top(view.from) < this.localScrollTop + height) ||
          (view.from !== undefined && this.heights.top(view.to!) >= top - demand.retainBefore &&
            this.heights.top(view.from) <= top + height + demand.retainAfter)).map(view => view.groupId));
        // 二分定位空间候选；只额外查找已挂载/交互保护组，不在滚动热路径遍历全文块。
        const candidateIds = new Set(retained);
        const wanted = state.mountMode === 'progressive-full' && !navigating ? 0 : from;
        let low = 0, high = this.groups.length;
        while (low < high) { const middle = (low + high) >>> 1;
          if (this.groups[middle].to <= wanted) low = middle + 1; else high = middle; }
        const end = state.mountMode === 'progressive-full' && !navigating ? this.expanded : to;
        const candidates = new Map<string, MarkdownBlockGroup>();
        for (let i = low; i < this.groups.length && this.groups[i].from < end; i++) candidates.set(this.groups[i].id, this.groups[i]);
        for (const group of this.mountedGroups(candidateIds, pinned)) candidates.set(group.id, group);
        for (const range of [...candidates.values()].sort((a, b) => a.from - b.from)) {
          const first = index.blocks[range.from];
          const last = index.blocks[range.to - 1];
          const semantic = this.mode === 'semantic' && first.mode === 'semantic';
          const restricted = this.mode === 'semantic' && first.mode === 'raw';
          let fromByte = first.fromByte;
          let toByte = last.toByte;
          if (!semantic && !restricted && toByte - fromByte > 32 * 1024) {
            const fraction = Math.max(
              0,
              Math.min(
                1,
                (top - this.heights.top(range.from)) /
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
            restricted,
            top:
              this.heights.top(range.from) +
              (wholeHeight * (fromByte - first.fromByte)) / wholeBytes,
            height: (wholeHeight * (toByte - fromByte)) / wholeBytes,
          });
        }
      } else if (this.mode === 'source' || state.status.byteLength === 0) {
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
          fromByte: view.window.fromByte, toByte: view.window.toByte, semantic: !!view.semantic, restricted: view.restricted,
          top: this.heights.top(from), height: this.heights.top(last + 1) - this.heights.top(from) });
      }
      const viewportTop = requestedTop ?? this.localScrollTop;
      const bottom = viewportTop + (this.scroll.clientHeight || 800) / this.scale;
      const distance = (plan: typeof plans[number]) => Math.max(0, plan.top - bottom, viewportTop - plan.top - plan.height);
      const targetByte = this.outlineNavigation?.byte;
      const containsTarget = (plan: typeof plans[number]) => targetByte !== undefined && plan.fromByte <= targetByte && plan.toByte > targetByte ? 1 : 0;
      plans.sort((a, b) => containsTarget(b) - containsTarget(a) || distance(a) - distance(b));
      const batchStart = performance.now();
      let createdCount = 0;
      const focused = this.focusAfterRemount || this.target.contains(globalThis.document.activeElement);
      // 标题导航已有经过索引确认的全局坐标；调度不重复序列化表格来反算当前光标。
      const selected = this.outlineNavigation ? { anchorByte: this.outlineNavigation.byte, headByte: this.outlineNavigation.byte } : this.globalSelection
        ? this.document.selection.selection
        : (this.selection() ?? this.document.selection.selection);
      for (const plan of plans) {
        const key = `${plan.groupId ?? `${plan.fromByte}:${plan.toByte}`}:${plan.restricted ? 'restricted' : plan.semantic}`;
        let view = this.mounted.find(
          (view) =>
            (view.key === key || !plan.groupId) &&
            ((view.restricted && plan.restricted) || this.pinned(view) || (view.signature === plan.signature && view.window.fromByte === plan.fromByte &&
              view.window.toByte === plan.toByte &&
              !!view.semantic === plan.semantic &&
              !(view.source?.bridge.isStale || view.semantic?.bridge.isStale))),
        );
        if (!view) {
          // 软预算优先释放远处保留带；可见区、基础缓冲和交互保护不被预算挤掉。
          if (!(this.outlineNavigation && plan.fromByte <= this.outlineNavigation.byte && plan.toByte > this.outlineNavigation.byte) &&
            distance(plan) > ((this.scroll.clientHeight || 800) / this.scale) * 3 &&
            (next.length >= MARKDOWN_VIEWPORT_POLICY.softViews ||
             next.reduce((sum, view) => sum + this.windowBytes(view), 0) + (plan.restricted ? 0 : plan.toByte - plan.fromByte) > MARKDOWN_VIEWPORT_POLICY.softWindowBytes)) continue;
          // 屏内也遵守同一批次预算，先发布已准备的完整组，不等整屏准备完毕。
          if (createdCount && performance.now() - batchStart >= MARKDOWN_VIEWPORT_POLICY.batchMs) {
            this.again = true;
            continue;
          }
          const started = performance.now();
          let interrupt!: () => void;
          const cancelled = new Promise<undefined>(resolve => { interrupt = () => resolve(undefined); });
          this.interruptRead = interrupt;
          const window: DocumentWindow | undefined = plan.restricted ? {
            documentId: state.status.documentId, revision: state.status.revision,
            fromByte: plan.fromByte, toByte: plan.toByte, text: '',
          } : await Promise.race([this.document.port.readWindow(
            state.status.documentId,
            state.status.revision,
            plan.fromByte,
            plan.toByte,
          ), cancelled]);
          const readFinished = performance.now();
          if (this.interruptRead === interrupt) this.interruptRead = undefined;
          const preparing = this.outlineNavigation;
          if (window && preparing?.kind === 'animated' && preparing.elapsed === 0 && this.scroll.clientHeight > 0) {
            // 读取与运动并行；新编辑器的同步构建放在首个运动帧之后的任务，避免挡住首次绘制。
            await preparing.firstFrame;
            await new Promise<void>(resolve => setTimeout(resolve, 0));
          }
          if (
            !window ||
            this.disposed ||
            this.hidden ||
            generation !== this.generation ||
            this.document.session.state.pendingTransactions ||
            this.document.state.status.revision !== state.status.revision
          )
            return;
          const buildStarted = performance.now();
          if (this.outlineNavigation && !this.outlineNavigation.aligned) this.outlineNavigation.sample.readMs += readFinished - started;
          const element = globalThis.document.createElement('div');
          element.className = 'markdown-session-window';
          view = { key, element, window, groupId: plan.groupId, signature: plan.signature };
          created.push(view);
          const stale = () => this.schedule();
          if (plan.restricted) {
            view.restricted = true;
            element.classList.add('markdown-session-restricted');
            element.contentEditable = 'false';
            const message = globalThis.document.createElement('p');
            message.textContent = '此内容暂不支持排版显示';
            const button = globalThis.document.createElement('button');
            button.type = 'button'; button.textContent = '在源码中打开';
            const restrictedView = view;
            button.onclick = () => {
              if (this.hidden || this.disposed || !this.mounted.includes(restrictedView)) return;
              this.callbacks.onOpenSource?.({ fromByte: restrictedView.window.fromByte,
                toByte: restrictedView.window.toByte, byte: restrictedView.restrictedByte ?? restrictedView.window.fromByte });
            };
            element.append(message, button);
          } else if (plan.semantic) {
            view.semantic = new MarkdownSemanticWindow(this.document.session, window, {
              ...this.options,
              target: element,
              references: index?.references,
              onStale: stale,
              onHistory: () => {
                void this.reveal(this.document.selection.selection.headByte, true);
              },
              onFailure: this.callbacks.onFailure,
              onPresentationChange: () => {
                if (this.hidden || this.disposed || !this.mounted.includes(view!)) return;
                if (this.input) this.input.readOnly = !this.writable;
                this.publishPresentation();
                this.markDirty(view!);
                this.measureDirty();
                this.schedule();
                if (view === this.active && this.focusAfterRemount && !this.pointerView && view!.semantic?.editable) this.focus();
              },
            });
          } else {
            view.source = new MarkdownSourceWindow(this.document.session, window, {
              target: element,
              readonly: this.options.readonly,
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
          if (this.outlineNavigation && !this.outlineNavigation.aligned) {
            this.outlineNavigation.sample.created++;
            this.outlineNavigation.sample.buildMs += performance.now() - buildStarted;
          }
          view.semantic?.core.prepareTableViewport(this.scroll,
            (this.scroll.clientHeight || 800) * MARKDOWN_VIEWPORT_POLICY.forwardScreens);
          const saved = this.layoutCache.get(`${key}:${plan.signature ?? state.status.revision}:${this.layoutContext}`);
          if (saved?.tables && view.semantic?.core.restoreTableLayout(saved.tables)) this.stats.tableRestores++;
          this.markDirty(view);
          this.observer?.observe(view.element);
        }
        view.from = plan.from;
        view.to = plan.to;
        if (view.restricted) {
          Object.assign(view.window, { fromByte: plan.fromByte, toByte: plan.toByte, revision: state.status.revision });
          view.signature = plan.signature;
          if (view.restrictedByte !== undefined) view.restrictedByte = Math.max(plan.fromByte, Math.min(view.restrictedByte, plan.toByte));
        }
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
        if (requestedTop !== undefined && !this.outlineNavigation) { this.scrollTo(requestedTop); this.scrollByte = undefined; }
        this.finishPreparation();
        if (!this.hasVisibleContent()) this.requestPreparation();
        this.measureDirty();
        this.restorePreferred();
        this.publishPresentation();
        return;
      }
      this.installedLayout = layout;
      const navigationScroll = this.outlineNavigation?.kind === 'animated' ? this.scroll.scrollTop : undefined;
      this.stats.batches++;
      this.stats.ranges++;
      const anchor = requestedTop === undefined || this.outlineNavigation ? this.captureAnchor() : undefined;
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
      this.refreshNavigationTargets();
      // 正文及占位高度安装后再次应用同一目标，避免新建源码栏停在文首。
      if (requestedTop !== undefined && !this.outlineNavigation) { this.scrollTo(requestedTop); this.scrollByte = undefined; }
      this.publishPresentation();
      for (const view of next) view.semantic?.activate();
      this.finishPreparation();
      if (!this.hasVisibleContent()) this.requestPreparation();
      this.stats.peakMounted = Math.max(this.stats.peakMounted, next.length);
      this.restoreAnchor(anchor);
      this.measureDirty();
      const preparingDirect = this.outlineNavigation?.kind === 'direct' && !this.outlineNavigation.aligned;
      if (!preparingDirect) this.active =
        next.find(
          (view) =>
            selected &&
            selected.headByte >= view.window.fromByte &&
            selected.headByte <= view.window.toByte,
        ) ?? next[0];
      if (!this.outlineNavigation && this.active && selected && !this.pinned(this.active) && created.includes(this.active)) {
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
      if (!preparingDirect && (
        focused ||
        !globalThis.document.activeElement ||
        globalThis.document.activeElement === globalThis.document.body
      ))
        this.callbacks.onActive(this.active?.semantic?.core, this.active?.source);
      if (!preparingDirect && focused && (this.focusAfterRemount || this.target.contains(globalThis.document.activeElement)) &&
        this.active && created.includes(this.active)) this.focus();
      if (navigationScroll !== undefined)
        this.scrollTo((navigationScroll - this.contentTop) / this.scale);
      if (this.globalSelection) this.paintSelection();
      this.publishPresentation();
      if (!navigating && state.mountMode === 'progressive-full' && this.expanded < (index?.blocks.length ?? 0))
        this.again = true;
      this.restorePreferred();
      if (performance.now() - this.demand.lastScroll >= MARKDOWN_VIEWPORT_POLICY.quietMs)
        this.readingAnchor = this.captureAnchor();
    } catch (error) {
      if (!(error instanceof DocumentConflictError) && (error as { code?: string })?.code !== 'revision-conflict') {
        this.viewFailure = error;
        this.showFailure();
      }
      throw error;
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
  /** 模式改变宽度前保留现有块锚点，后续仍由批量测量恢复屏幕偏移。 */
  preserveReadingPosition() {
    this.readingAnchor = this.captureAnchor();
  }
  scrollToByte(byte: number) {
    this.cancelOutlineNavigation();
    this.locatedByte = byte;
    this.document.requestVisibleStructure(this, null);
    this.invalidateStable();
    this.requestPreparation();
    this.navigation++;
    this.generation++;
    this.preferredByte = undefined;
    this.scrollByte = byte;
    this.schedule();
  }
  private isOutlineTargetLoaded(byte: number) {
    return this.mounted.some(view => byte >= view.window.fromByte && byte < view.window.toByte &&
      view.window.revision === this.document.state.status.revision && view.element.isConnected &&
      (!!view.semantic?.bodyReady || (!!view.source && !view.source.bridge.isStale)));
  }
  async reveal(byte: number, focus = false, alignment: 'selection' | 'outline' = 'selection',
    clock: OutlineNavigationClock = { requestedAt: performance.now() }) {
    this.cancelOutlineNavigation();
    this.interruptRead?.();
    const heading = alignment === 'outline' ? this.document.state.index?.outline.find(item => item.byte === byte) : undefined;
    this.locatedByte = byte;
    this.preferredFocus = focus;
    this.document.requestVisibleStructure(this, null);
    this.invalidateStable();
    this.requestPreparation();
    const navigation = ++this.navigation;
    this.generation++;
    await this.document.session.flushEdits();
    if (navigation !== this.navigation || this.disposed || this.hidden) return;
    this.globalSelection = false;
    this.removeInput();
    this.clearSelectionPaint();
    this.preferredByte = byte;
    let completion: Promise<void> | undefined;
    if (heading) {
      const targetLoaded = this.isOutlineTargetLoaded(byte);
      const windowLimited = this.document.state.mountMode === 'windowed' && this.document.state.windowLimited;
      const kind = windowLimited && !targetLoaded ? 'direct' : 'animated';
      if (kind === 'animated') clock.startedAt ??= performance.now();
      const startedAt = kind === 'animated' ? clock.startedAt! : performance.now();
      this.scrollByte = undefined;
      let settle!: () => void;
      completion = new Promise<void>(resolve => { settle = resolve; });
      const reduced = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      const duration = kind === 'animated' ? (reduced ? 140 : 260) : undefined;
      const controller = new AbortController();
      let motionStarted: (() => void) | undefined;
      const firstFrame = kind === 'animated' ? new Promise<void>(resolve => { motionStarted = resolve; }) : undefined;
      const sample: OutlineNavigationSample = { revision: this.document.state.status.revision, byte, duration,
        kind, windowLimited, targetLoaded, requestedAt: clock.requestedAt,
        inputGateMs: startedAt - clock.requestedAt, readMs: 0, buildMs: 0, measureMs: 0, created: 0,
        pendingEnhancements: 0, deferred: { code: 0, typography: 0, diagram: 0 },
        deadlineMiss: kind === 'animated' ? false : undefined, status: 'preparing' };
      const deferred = { code: new WeakSet<HTMLElement>(), typography: new WeakSet<HTMLElement>(), diagram: new WeakSet<HTMLElement>() };
      const originTop = this.localScrollTop, originBottom = originTop + (this.scroll.clientHeight || 800) / this.scale;
      const target: OutlineNavigation = { kind, id: heading.id, byte, revision: sample.revision, aligned: false,
        originAnchor: kind === 'direct' ? this.captureAnchor(true) : undefined,
        originGroups: kind === 'direct' ? new Set(this.mounted.filter(view => view.groupId && view.from !== undefined &&
          this.heights.top(view.to!) > originTop && this.heights.top(view.from) < originBottom).map(view => view.groupId!)) : undefined,
        overflowAnchor: this.scroll.style.overflowAnchor,
        startedAt, goal: this.scroll.scrollTop,
        elapsed: 0, initialTop: this.scroll.scrollTop, frame: 0, animationDone: false, ready: false, recorded: false,
        firstFrame, motionStarted,
        controller, sample, settle, context: {
          revision: sample.revision, generation: navigation,
          deadline: duration === undefined ? undefined : startedAt + duration, preparationOnly: kind === 'direct',
          signal: controller.signal, ready: false, targets: new Set(),
          viewport: () => { const top = this.scroll.getBoundingClientRect().top + target.goal - this.scroll.scrollTop;
            return { top, bottom: top + this.scroll.clientHeight }; },
          deferred: (kind, element) => {
            if (!deferred[kind].has(element)) { deferred[kind].add(element); sample.deferred[kind]++; }
          },
          started: () => {
            sample.pendingEnhancements++;
            let finished = false;
            return () => { if (finished) return; finished = true; sample.pendingEnhancements--;
              if (!controller.signal.aborted && !sample.pendingEnhancements)
                sample.enhancementMs = performance.now() - target.startedAt;
            };
          },
        } };
      this.outlineNavigation = target;
      this.scroll.style.overflowAnchor = 'none';
      const block = this.index?.blockAt(byte);
      const position = block && this.blockPositions.get(block.id);
      if (position !== undefined) target.goal = Math.max(0, Math.min(this.scroll.scrollHeight - this.scroll.clientHeight,
        this.contentTop + this.heights.top(position) * this.scale - getOutlineViewportOffset(this.scroll.clientHeight)));
      this.refreshNavigationTargets();
      if (kind === 'animated') target.frame = requestAnimationFrame(this.advanceOutlineMotion);
      this.document.interacting();
      this.publishOutlinePosition();
    }
    // 先登记导航策略（动画分支同时启动时钟），再读取目标；旧读取不允许发布。
    if (this.busy) await this.renderDone;
    if (navigation !== this.navigation || this.disposed || this.hidden) return;
    await this.render();
    const view = this.active;
    if (!heading && view?.semantic && byte >= view.window.fromByte && byte <= view.window.toByte)
      await view.semantic.whenReady;
    if (navigation !== this.navigation || this.disposed || this.hidden) return;
    this.alignOutlineHeading();
    if (this.preferredByte !== undefined) this.schedule();
    if (this.scroll.clientHeight > 0) await completion;
  }
  private copyNavigationSample(sample: OutlineNavigationSample): OutlineNavigationSample {
    return { ...sample, deferred: { ...sample.deferred } };
  }
  private recordNavigation(target: OutlineNavigation) {
    if (target.recorded) return;
    target.recorded = true;
    this.navigationSamples.push(target.sample);
    if (this.navigationSamples.length > 32) this.navigationSamples.shift();
  }
  private refreshNavigationTargets() {
    const target = this.outlineNavigation;
    if (!target || target.aligned) return;
    const top = Math.max(0, (target.goal - this.contentTop) / this.scale);
    const bottom = top + (this.scroll.clientHeight || 800) / this.scale;
    const targets = new Set(this.mounted.filter(view => view.from !== undefined &&
      this.heights.top(view.to!) > top && this.heights.top(view.from) < bottom).map(view => view.element));
    const ready = target.kind === 'animated' && target.ready && target.elapsed > 0;
    if (target.context.ready === ready && target.notifiedGoal === target.goal &&
      targets.size === target.context.targets.size && [...targets].every(element => target.context.targets.has(element))) return;
    target.context.targets = targets;
    target.notifiedGoal = target.goal;
    target.context.ready = ready;
    setNavigationRenderContext(this.target, target.context);
  }
  private alignOutlineHeading() {
    const target = this.outlineNavigation;
    if (!target) return;
    if (target.revision !== this.document.state.status.revision) { this.cancelOutlineNavigation(); return; }
    if (target.kind === 'direct' && !target.aligned) {
      this.restoreAnchor(target.originAnchor, true);
      const view = this.mounted.find(view => target.byte >= view.window.fromByte && target.byte < view.window.toByte);
      if (view?.restricted) {
        target.sample.status = 'restricted';
        this.cancelOutlineNavigation();
        this.callbacks.onFailure(new Error('目标内容暂不支持排版显示。'));
        return;
      }
    }
    const top = this.headingTop(target.byte, target.id);
    if (top === undefined || this.scroll.clientHeight <= 0) return;
    const delta = top - this.scroll.getBoundingClientRect().top - getOutlineViewportOffset(this.scroll.clientHeight);
    target.goal = Math.max(0, Math.min(this.scroll.scrollHeight - this.scroll.clientHeight, this.scroll.scrollTop + delta));
    const localGoal = Math.max(0, (target.goal - this.contentTop) / this.scale);
    if (target.kind === 'direct' && !target.aligned) {
      const bottom = Math.min(localGoal + this.scroll.clientHeight / this.scale, this.heights.totalHeight);
      if (this.mounted.some(view => view.restricted && view.from !== undefined &&
        this.heights.top(view.to!) > localGoal && this.heights.top(view.from) < bottom)) {
        target.sample.status = 'restricted';
        this.cancelOutlineNavigation();
        this.callbacks.onFailure(new Error('目标首屏包含暂不支持排版显示的内容。'));
        return;
      }
    }
    const ready = this.outlineViewportCovered(localGoal);
    if (!target.aligned) {
      if (ready && !target.ready) target.sample.basicReadyMs = performance.now() - target.startedAt;
      target.ready = ready;
      this.refreshNavigationTargets();
    }
    if (target.kind === 'direct' && !target.aligned) {
      if (ready && !target.frame) target.frame = requestAnimationFrame(() => this.commitDirectNavigation(target));
      return;
    }
    if (target.animationDone) {
      if (Math.abs(target.goal - this.scroll.scrollTop) >= 1) this.scrollTo(localGoal);
      if (ready && !target.aligned) {
        this.finishOutlineNavigation(target);
      }
    }
  }
  private commitDirectNavigation(target: OutlineNavigation) {
    if (this.outlineNavigation !== target || target.controller.signal.aborted || this.hidden || this.disposed) return;
    // 保留 frame 标识直到校验结束，避免 align 在本次回调内重复安排定位。
    this.alignOutlineHeading();
    target.frame = 0;
    if (this.outlineNavigation !== target || !target.ready) return;
    target.animationDone = true;
    this.scrollTo(Math.max(0, (target.goal - this.contentTop) / this.scale));
    this.finishOutlineNavigation(target);
    this.restorePreferred();
    this.publishOutlinePosition();
  }
  private finishOutlineNavigation(target: OutlineNavigation) {
    target.aligned = true;
    target.originAnchor = undefined;
    target.originGroups = undefined;
    target.sample.positionedMs = performance.now() - target.sample.requestedAt;
    target.sample.status = target.sample.deadlineMiss ? 'late' : 'ready';
    this.recordNavigation(target);
    // 通知增强观察者可能启动工作，留给普通调度任务，避免挤入动画最后一帧。
    queueMicrotask(() => {
      target.context.targets = new Set();
      if (this.outlineNavigation === target) setNavigationRenderContext(this.target, undefined, true);
    });
    target.settle?.(); target.settle = undefined;
    this.schedule();
  }
  private advanceOutlineMotion = () => {
    const target = this.outlineNavigation;
    if (!target || target.kind !== 'animated' || target.animationDone || this.hidden || this.disposed) return;
    target.frame = 0;
    const now = performance.now();
    const duration = target.sample.duration!;
    const elapsed = Math.min(duration, Math.max(0, now - target.startedAt));
    const remaining = duration - target.elapsed;
    const fraction = remaining > 0 ? 1 - Math.pow((duration - elapsed) / remaining, 3) : 1;
    const next = this.scroll.scrollTop + (target.goal - this.scroll.scrollTop) * fraction;
    this.scrollTo((next - this.contentTop) / this.scale);
    if (target.sample.firstMotionMs === undefined && Math.abs(this.scroll.scrollTop - target.initialTop) >= 1)
      target.sample.firstMotionMs = now - target.startedAt;
    this.readingAnchor = undefined;
    target.elapsed = elapsed;
    if (target.motionStarted) {
      target.motionStarted(); target.motionStarted = undefined;
      markdownWorkScheduler.enqueue(this, 'navigation-enhancements', MarkdownTaskPriority.VisibleText, () => this.refreshNavigationTargets());
    }
    if (elapsed >= duration) {
      target.animationDone = true;
      target.sample.animationMs = now - target.startedAt;
      target.sample.deadlineMiss = !target.ready || (target.sample.basicReadyMs ?? Infinity) > duration;
      if (target.ready) this.finishOutlineNavigation(target);
      // 局部几何测量和正文就绪检查由调度批次执行，动画帧只写滚动位置。
      this.schedule();
      return;
    }
    target.frame = requestAnimationFrame(this.advanceOutlineMotion);
  };
  private outlineViewportCovered(top: number) {
    let through = Math.max(0, top);
    const bottom = Math.min(through + (this.scroll.clientHeight || 800) / this.scale, this.heights.totalHeight);
    if (bottom <= through) return false;
    for (const view of this.mounted) {
      if (view.from === undefined || view.to! <= view.from || this.heights.top(view.to!) <= through) continue;
      if (this.heights.top(view.from) > through + 1) break;
      if (!view.measured || !view.element.isConnected || view.window.revision !== this.document.state.status.revision ||
        (view.semantic && !view.semantic.bodyReady) || view.source?.bridge.isStale) return false;
      through = Math.max(through, this.heights.top(view.to!));
      if (through >= bottom - 1) return true;
    }
    return false;
  }
  private restorePreferred() {
    if (this.outlineNavigation?.kind === 'direct' && !this.outlineNavigation.aligned) {
      this.alignOutlineHeading();
      this.publishOutlinePosition();
      return;
    }
    const byte = this.preferredByte;
    if (byte === undefined) { this.alignOutlineHeading(); this.publishOutlinePosition(); return; }
    const view = this.mounted.find(
      (view) => byte >= view.window.fromByte && (byte < view.window.toByte ||
        byte === this.document.state.status.byteLength && byte === view.window.toByte),
    );
    if (view) {
      this.preferredByte = undefined;
      this.active = view;
      if (view.restricted) {
        view.restrictedByte = byte;
        this.document.selection.set({ anchorByte: byte, headByte: byte });
      }
      // readWindow 会修正 UTF-8 边界，导航到窗口边缘时使用其确认坐标。
      const selection = { anchorByte: byte, headByte: byte };
      try {
        view.source?.restoreSelection(selection);
        // 标题 byte 指向 Markdown 标记（如 #），不是可编辑文本光标；使用同一锚点的 PM 位置。
        const heading = this.index?.outline.find(heading => heading.byte === byte);
        if (!heading || !view.semantic?.core.revealHeadingAnchor(heading.id, !this.outlineNavigation))
          view.semantic?.core.restoreSourceSelection(selection);
      } catch {
        const edge = { anchorByte: view.window.fromByte, headByte: view.window.fromByte };
        view.source?.restoreSelection(edge);
      }
      this.callbacks.onActive(view.semantic?.core, view.source);
      if (this.preferredFocus) this.focus();
    }
    this.alignOutlineHeading();
    this.publishOutlinePosition();
  }
  private publishOutlinePosition() {
    const id = this.activeOutlineId;
    if (id === this.publishedOutlineId) return;
    this.publishedOutlineId = id;
    this.scroll.dispatchEvent(new Event('nomo-outline-position'));
  }
  focus() {
    this.focusAfterRemount = true;
    cancelAnimationFrame(this.focusFrame);
    if (this.globalSelection) this.focusGlobalInput();
    else {
      this.active?.source?.view.focus();
      if (this.active?.semantic?.editable) this.active.semantic.core.focus();
      if (this.active?.restricted) this.active.element.querySelector('button')?.focus({ preventScroll: true });
      const active = this.active;
      // 新 PM 视图首帧才启用 contenteditable，必须在其启用后补一次有条件的聚焦。
      if (active && !this.target.contains(globalThis.document.activeElement))
        this.focusFrame = requestAnimationFrame(() => {
          this.focusFrame = 0;
          if (!this.hidden && !this.disposed && this.focusAfterRemount && this.active === active) {
            active.source?.view.focus();
            if (active.semantic?.editable) active.semantic.core.focus();
          }
        });
    }
  }
  updateOptions(options: Partial<EditorRuntimeOptions>) {
    this.options = { ...this.options, ...options };
    for (const view of this.mounted) view.semantic?.updateOptions(options);
    for (const view of this.mounted) view.source?.setReadonly(!!this.options.readonly || !!this.document.state.failure || !!this.viewFailure);
    if (this.input) this.input.readOnly = !this.writable;
    this.publishPresentation();
  }
  updateTheme(theme: NonNullable<EditorCoreOptions['theme']>) {
    this.options.theme = theme;
    for (const view of this.mounted) view.semantic?.core.updateTheme(theme);
    this.invalidateGeometry();
  }
  async setHidden(hidden: boolean) {
    if (this.hidden === hidden) return;
    this.cancelOutlineNavigation();
    this.interruptRead?.();
    this.hidden = hidden;
    this.document.requestVisibleStructure(this, null);
    this.invalidateStable();
    if (hidden) { this.focusAfterRemount = false; this.preferredFocus = false; }
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
    this.cancelOutlineNavigation();
    this.interruptRead?.();
    this.hidden = true;
    this.document.requestVisibleStructure(this, null);
    this.invalidateStable();
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
    this.navigationSamples.length = 0;
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
    this.scroll.removeEventListener('wheel', this.onNavigationIntent);
    this.scroll.removeEventListener('touchstart', this.onNavigationIntent);
    this.scroll.removeEventListener('pointerdown', this.onNavigationIntent, true);
    this.target.removeEventListener('keydown', this.onKey, true);
    this.target.removeEventListener('keyup', this.onSelectionChanged, true);
    this.target.removeEventListener('copy', this.onCopy, true);
    this.target.removeEventListener('cut', this.onCopy, true);
    this.target.removeEventListener('beforeinput', this.onBeforeInput, true);
    this.target.removeEventListener('paste', this.guardWrite, true);
    this.target.removeEventListener('drop', this.guardWrite, true);
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
