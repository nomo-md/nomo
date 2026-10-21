import { ProseMirrorEditorCore } from '../editor-core/ProseMirrorEditorCore';
import type { EditorCoreOptions } from '../editor-core/types';
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
  constructor(
    session: MarkdownSessionController,
    window: DocumentWindow,
    options: Omit<EditorCoreOptions, 'markdown' | 'documentProjection'> & {
      references?: Record<string, { href: string; title: string }>;
      onStale(): void;
      onHistory(): void;
      onFailure(error: unknown): void;
    },
  ) {
    let core: ProseMirrorEditorCore | undefined;
    this.bridge = new MarkdownWindowBridge(session, window, {
      onStale: () => {
        core?.updateOptions({ readonly: true });
        options.onStale();
      },
      onRebase: (fromByte) => core?.rebaseSourceProjection(fromByte),
      onError: options.onFailure,
      onState: (state) =>
        core?.updateOptions({
          readonly: !!options.readonly || state.historyBusy || state.closing || this.bridge.isStale,
        }),
    });
    this.core = core = new ProseMirrorEditorCore({
      ...options,
      markdown: window.text,
      onError: (error) => {
        options.onError?.(error);
        options.onFailure(error.cause ?? error);
      },
      documentProjection: {
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
  /** 调用方先等待组字和确认；失败窗口必须继续保留，可供重试或复制恢复。 */
  dispose() {
    this.core.commitPendingEdits();
    if (this.bridge.pinned || this.core.hasUnsettledProjection())
      throw new Error('正在编辑的语义区域不能回收。');
    this.detachEdits();
    this.bridge.dispose();
    this.core.destroy();
  }
}
