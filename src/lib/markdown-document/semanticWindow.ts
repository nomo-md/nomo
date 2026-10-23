import { ProseMirrorEditorCore } from '../editor-core/ProseMirrorEditorCore';
import type { EditorCoreOptions, EditorRuntimeOptions } from '../editor-core/types';
import type { MarkdownSessionController } from './controller';
import type { DocumentWindow } from './protocol';
import { MarkdownWindowBridge } from './windowBridge';

/** 有界语义窗口。全文操作必须通过 session，core 的同步接口只供窗口内交互使用。 */
export class MarkdownSemanticWindow {
  readonly core: ProseMirrorEditorCore;
  readonly bridge: MarkdownWindowBridge;
  private detachEdits: () => void;
  private group = '';
  private lastEdit = 0;
  private composing = false;
  private attached = false;
  private ready: boolean;
  private opened = false;
  private disposed = false;
  private applicationReadonly: boolean;
  private failed = false;
  private effectiveReadonly = true;
  private resolveReady!: () => void;
  readonly whenReady = new Promise<void>(resolve => { this.resolveReady = resolve; });
  constructor(
    private readonly session: MarkdownSessionController,
    window: DocumentWindow,
    private readonly options: Omit<EditorCoreOptions, 'markdown' | 'documentProjection'> & {
      references?: Record<string, { href: string; title: string }>;
      onStale(): void;
      onHistory(): void;
      onFailure(error: unknown): void;
      onPresentationChange?(): void;
    },
  ) {
    this.ready = window.text.length === 0;
    if (this.ready) this.resolveReady();
    this.applicationReadonly = !!options.readonly;
    let core: ProseMirrorEditorCore | undefined;
    this.bridge = new MarkdownWindowBridge(session, window, {
      onStale: () => {
        this.syncPermission();
        options.onStale();
      },
      onRebase: (fromByte) => core?.rebaseSourceProjection(fromByte),
      onError: options.onFailure,
      onState: () => this.syncPermission(),
    });
    this.core = core = new ProseMirrorEditorCore({
      ...options,
      readonly: true,
      markdown: window.text,
      onError: (error) => {
        options.onError?.(error);
        options.onFailure(error.cause ?? error);
      },
      documentProjection: {
        onViewReady: () => { this.ready = true; this.syncPermission(); this.resolveReady(); },
        fromByte: window.fromByte,
        references: options.references,
        commit: (change) => {
          const now = performance.now();
          if (!this.group || (!this.composing && now - this.lastEdit > 500))
            this.group = globalThis.crypto.randomUUID();
          this.lastEdit = now;
          this.bridge.commit(change, this.group);
        },
        history: (redo) => {
          this.group = '';
          void session.undo(redo).then(options.onHistory, options.onFailure);
          return true;
        },
        canHistory: (redo) =>
          redo
            ? session.state.committed.canRedo
            : session.state.pendingTransactions > 0 || session.state.committed.canUndo,
        historyBoundary: (kind) => {
          this.group = '';
          this.composing = kind === 'composition-start';
        },
      },
    });
    this.detachEdits = session.attachEdits({
      awaitCompositionEnd: () => this.core.awaitCompositionEnd(),
      commitPendingEdits: () => this.core.commitPendingEdits(),
      hasUnconfirmedDraft: () => this.core.hasPendingMathInlineDraft(),
    });
  }
  get editable() { return this.attached && !this.effectiveReadonly; }
  get bodyReady() { return this.attached && !!this.options.target?.isConnected && this.ready && !this.disposed && !this.bridge.isStale; }
  get preview() { return !this.opened; }
  /** DOM 插入后启用，生命周期未就绪时仍保持预览；不重建 PM 或源文映射。 */
  activate() { this.attached = true; this.syncPermission(); }
  /** 仅在发布 DOM 的同步批次调用；未接入前仍不报告正文或交互就绪。 */
  prepareAttachment() {
    if (this.disposed || this.attached || !this.options.target || this.options.target.isConnected) return;
    this.core.prepareDetachedProjection();
    this.syncPermission(true);
  }
  setFailure(failed: boolean) { this.failed = failed; this.syncPermission(); }
  updateOptions(options: Partial<EditorRuntimeOptions>) {
    if (options.readonly !== undefined) this.applicationReadonly = options.readonly;
    const { readonly: _, ...rest } = options;
    this.core.updateOptions({ ...rest, readonly: this.effectiveReadonly });
    this.syncPermission();
  }
  private syncPermission(preparingAttachment = false) {
    if (!this.core || this.disposed) return;
    const state = this.session.state;
    const blocked = this.applicationReadonly || this.failed || !!state.failure ||
      state.historyBusy || state.closing || this.bridge.isStale;
    // 开放后允许原窗口连续乐观输入；不能因自己的待确认事务中断 IME。
    if (!this.opened && (this.attached || preparingAttachment) && this.ready && !blocked && !state.pendingTransactions)
      this.opened = true;
    const readonly = !this.opened || blocked;
    if (readonly === this.effectiveReadonly) return;
    this.effectiveReadonly = readonly;
    this.core.updateOptions({ readonly });
    this.options.onPresentationChange?.();
  }
  /** 调用方先等待组字和确认；失败窗口必须继续保留，可供重试或复制恢复。 */
  dispose(prepared = false) {
    if (!prepared) this.core.commitPendingEdits();
    if (this.bridge.pinned || this.core.hasUnsettledProjection())
      throw new Error('正在编辑的语义区域不能回收。');
    this.detachEdits();
    this.disposed = true;
    this.resolveReady();
    this.bridge.dispose();
    this.core.destroy({ prepared });
  }
}
