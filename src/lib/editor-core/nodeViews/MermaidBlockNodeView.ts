import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import { getDiagramRenderer } from '../renderers';
import type { MermaidThemeDefinition } from '../../theme/types';
import { registerActiveEdit, unregisterActiveEdit } from './activeEditRegistry';
import { observeRenderVisibility, preserveRenderScrollAnchor, trackNavigationEnhancement } from './renderVisibility';
import {
  bindMermaidFullscreen,
  bindMermaidViewport,
  type MermaidViewportBinding,
  type MermaidViewMode,
  type MermaidViewPosition,
  normalizeMermaidSvgSize,
  normalizeRenderedMermaidViewport,
  type MermaidFullscreenBinding,
} from '../../services/mermaidDiagramView';

/**
 * Mermaid 图表块 NodeView。
 *
 * 职责：
 * 1. 常态把 Mermaid 源码渲染为 SVG 图表；
 * 2. 点击图表进入源码编辑态；
 * 3. 编辑态保持“源码在上、预览在下”的稳定布局。
 */
export class MermaidBlockNodeView {
  private static instances = new Set<MermaidBlockNodeView>();
  private static currentTheme: MermaidThemeDefinition = { theme: 'default' };
  private static readonly PREVIEW_DEBOUNCE_MS = 250;

  dom: HTMLElement;

  private node: ProseMirrorNode;
  private view: EditorView;
  private getPos: () => number;
  private renderId = 0;
  private renderedCode: string | null = null;
  private pendingRender: Promise<void> | null = null;
  private cancelPendingRender: (() => void) | null = null;
  private previewRenderId = 0;
  private previewDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  private editing = false;
  private readonly activeEditExitFn = () => this.exitEdit(true, 'preserve');
  private originalCode = '';
  private textarea: HTMLTextAreaElement | null = null;
  private previewEl: HTMLElement | null = null;
  private previewSnapshotEl: HTMLElement | null = null;
  private editSurfaceEl: HTMLElement | null = null;
  private fullscreenBinding: MermaidFullscreenBinding | null = null;
  private viewportBinding: MermaidViewportBinding | null = null;
  private viewMode: Exclude<MermaidViewMode, 'custom'> = 'readable';
  private viewPosition: MermaidViewPosition | undefined;
  private previewToolbar: HTMLElement | null = null;
  private destroyed = false;
  private unsubscribeLocale: () => void = () => undefined;
  private stopObserving: () => void = () => undefined;
  private renderAbort: AbortController | null = null;
  private themeVisible = typeof IntersectionObserver !== 'function';
  private themeDirty = false;

  constructor(node: ProseMirrorNode, view: EditorView, getPos: () => number) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    MermaidBlockNodeView.instances.add(this);

    this.dom = document.createElement('div');
    this.dom.className = 'mermaid-block';
    this.dom.contentEditable = 'false';
    this.dom.setAttribute('data-code', node.attrs.code as string);

    this.stopObserving = observeRenderVisibility(this.dom, (visible) => {
      this.themeVisible = visible;
      if (visible) void this.applyPendingTheme();
      else if (!this.editing && this.pendingRender) {
        this.renderId++;
        this.clearPendingRender();
        this.themeDirty = true;
      }
    }, 'diagram', () => !this.editing && (this.themeDirty || this.renderedCode !== this.node.attrs.code));

    this.dom.addEventListener('click', (event) => {
      if (
        !this.view.editable ||
        this.editing ||
        (event.target as Element).closest?.('.mermaid-view-toolbar')
      ) return;
      const target = event.target as Element;
      if (target.closest?.('.mermaid-managed-viewport') && !target.closest?.('svg')) return;
      event.preventDefault();
      event.stopPropagation();
      this.enterEdit();
    });

    this.unsubscribeLocale = onInterfaceLocaleChanged(() => {
      this.closeFullscreen();
      if (!this.editing) {
        void this.renderMermaid();
      } else {
        void this.updatePreview();
      }
    });
    this.renderMermaid();
  }

  static updateTheme(theme?: MermaidThemeDefinition): void {
    if (theme) {
      MermaidBlockNodeView.currentTheme = theme;
    }
    for (const instance of MermaidBlockNodeView.instances) {
      // 立即作废旧请求，即便节点在屏幕外也不能让旧异步结果覆盖新主题。
      instance.renderId += 1;
      instance.previewRenderId += 1;
      instance.clearPendingRender();
      instance.renderedCode = null;
      instance.themeDirty = true;
      if (instance.themeVisible) void instance.applyPendingTheme();
    }
  }

  /** 导出读取 DOM 前等待最新源码与主题渲染，包括当前编辑器的屏幕外图表。 */
  static async flushThemeUpdates(root: HTMLElement): Promise<void> {
    await Promise.all(
      [...MermaidBlockNodeView.instances]
        .filter((instance) => root.contains(instance.dom))
        .map((instance) => instance.flushPendingRender()),
    );
  }

  private async flushPendingRender(): Promise<void> {
    while (!this.destroyed) {
      if (this.editing && this.previewDebounceTimer !== null) {
        this.clearPreviewDebounce();
        void this.updatePreview();
      }
      const render = this.applyPendingTheme(true);
      if (!render) return;
      await render;
    }
  }

  /** 只读取最新主题；连续切换时屏幕外节点不积累中间主题的渲染任务。 */
  private applyPendingTheme(force = false): Promise<void> | null {
    if (this.destroyed) return null;
    if (!this.themeDirty) return this.pendingRender;
    if (!force && !this.themeVisible && !this.editing) return this.pendingRender;
    this.themeDirty = false;
    return this.editing ? this.updatePreview() : this.renderMermaid(force);
  }

  private clearPendingRender(): void {
    this.renderAbort?.abort();
    this.renderAbort = null;
    const cancel = this.cancelPendingRender;
    this.pendingRender = null;
    this.cancelPendingRender = null;
    cancel?.();
  }

  /** 旧请求失效时解除等待；底层渲染结果仍由 renderId 检查，不能覆盖新内容。 */
  private trackPendingRender(render: Promise<void>): Promise<void> {
    const cancelled = new Promise<void>((resolve) => {
      this.cancelPendingRender = resolve;
    });
    const pending = Promise.race([render, cancelled]).finally(() => {
      if (this.pendingRender === pending) {
        this.pendingRender = null;
        this.cancelPendingRender = null;
      }
    });
    this.pendingRender = pending;
    return pending;
  }

  static enterEditAt(view: EditorView, pos: number, caret: 'start' | 'end' = 'start'): boolean {
    if (!view.editable || view.isDestroyed) return false;
    for (const instance of MermaidBlockNodeView.instances) {
      if (instance.view !== view) continue;
      if (instance.getPos() === pos) {
        return instance.enterEdit(caret);
      }
    }
    return false;
  }

  static enterClosestEditAt(
    view: EditorView,
    pos: number,
    caret: 'start' | 'end' = 'start',
  ): boolean {
    if (!view.editable || view.isDestroyed) return false;
    let closestInstance: MermaidBlockNodeView | null = null;
    let closestDistance = Number.POSITIVE_INFINITY;

    for (const instance of MermaidBlockNodeView.instances) {
      if (instance.view !== view || instance.editing) continue;
      const distance = Math.abs(instance.getPos() - pos);
      if (distance < closestDistance) {
        closestDistance = distance;
        closestInstance = instance;
      }
    }

    if (!closestInstance || closestDistance > 4) return false;
    return closestInstance.enterEdit(caret);
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) return false;
    const codeChanged = node.attrs.code !== this.node.attrs.code;
    this.node = node;
    if (this.editing && !this.view.editable) {
      this.cleanupEdit();
      void this.renderMermaid();
    } else if (codeChanged) {
      this.dom.setAttribute('data-code', node.attrs.code as string);
      if (this.editing && this.textarea) {
        // 自身提交先结束编辑再 dispatch；仍处于编辑态的变化来自撤销或外部更新。
        const { selectionStart, selectionEnd, selectionDirection } = this.textarea;
        this.originalCode = node.attrs.code as string;
        this.textarea.value = this.originalCode;
        this.textarea.setSelectionRange(selectionStart, selectionEnd, selectionDirection);
        this.autoResizeTextarea();
        this.clearPreviewDebounce();
        void this.updatePreview();
      } else {
        // 焦点和选区事务不应重建查看器，否则全屏打开后会立即被销毁。
        void this.renderMermaid();
      }
    }
    return true;
  }

  selectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
  }

  deselectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
    if (this.editing) {
      this.exitEdit(true, 'preserve');
    }
  }

  stopEvent(event: Event): boolean {
    if (
      (event.target as Element).closest?.(
        '.mermaid-view-toolbar, .mermaid-block-fullscreen-button, .mermaid-managed-viewport',
      )
    )
      return true;
    if (this.editing && this.dom.contains(event.target as Node)) return true;
    return false;
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.destroyed = true;
    this.stopObserving();
    this.clearPendingRender();
    this.themeDirty = false;
    this.renderId += 1;
    this.previewRenderId += 1;
    this.disposeFullscreen();
    this.cleanupEdit();
    this.unsubscribeLocale();
    MermaidBlockNodeView.instances.delete(this);
  }

  private renderMermaid(force = false): Promise<void> {
    if (this.destroyed || this.editing) return Promise.resolve();
    this.clearPendingRender();
    const id = ++this.renderId;
    const code = this.node.attrs.code as string;
    this.dom.setAttribute('data-code', code);

    if (!code.trim()) {
      this.dom.style.removeProperty('min-height');
      this.renderEmptyDiagram();
      this.renderedCode = code;
      return Promise.resolve();
    }

    if (!force && !this.themeVisible) {
      this.themeDirty = true;
      // 首次渲染先保留块高度；已有图表在新结果就绪前继续显示。
      if (!this.dom.hasChildNodes()) this.dom.style.minHeight = '160px';
      return Promise.resolve();
    }
    this.themeDirty = false;

    const renderer = getDiagramRenderer();
    if (!renderer) {
      this.disposeFullscreen();
      this.dom.textContent = `\`\`\`mermaid\n${code}\n\`\`\``;
      this.renderedCode = code;
      return Promise.resolve();
    }

    const controller = new AbortController();
    this.renderAbort = controller;
    const finishNavigationEnhancement = !force
      ? trackNavigationEnhancement(this.dom, 'diagram', controller) : () => {};
    return this.trackPendingRender((async () => {
      try {
        const result = await renderer.renderMermaid(code, { theme: this.getTheme(), signal: controller.signal });
        if (this.destroyed || this.view.isDestroyed || this.editing || id !== this.renderId || controller.signal.aborted) return;
        preserveRenderScrollAnchor(this.dom, () => {
          this.dom.style.removeProperty('min-height');
          if (result.error) this.renderError(result.error, code);
          else this.renderDisplayDiagram(normalizeMermaidSvgSize(result.svg));
        });
        this.renderedCode = code;
      } catch (error) {
        if (this.destroyed || this.view.isDestroyed || this.editing || id !== this.renderId) return;
        this.renderError(error instanceof Error ? error.message : t.mermaidRenderFailed(), code);
        this.renderedCode = code;
      }
    })().finally(finishNavigationEnhancement));
  }

  private enterEdit(caret: 'start' | 'end' = 'start'): boolean {
    if (!this.view.editable || this.editing || this.destroyed || this.view.isDestroyed) return false;
    registerActiveEdit(this.view, this.activeEditExitFn);
    this.editing = true;
    this.renderId += 1;
    this.previewRenderId += 1;
    this.clearPendingRender();
    this.disposeFullscreen();
    this.originalCode = this.node.attrs.code as string;
    const snapshotCurrent = this.renderedCode === this.originalCode;
    this.previewSnapshotEl = this.takeRenderedPreviewSnapshot('mermaid-block-preview-snapshot');
    this.dom.classList.add('is-editing');
    this.dom.classList.remove('ProseMirror-selectednode');

    this.editSurfaceEl = document.createElement('div');
    this.editSurfaceEl.className = 'mermaid-block-edit-surface';
    this.dom.appendChild(this.editSurfaceEl);

    this.textarea = document.createElement('textarea');
    this.textarea.className = 'mermaid-block-textarea';
    this.textarea.value = this.originalCode;
    this.textarea.rows = Math.max(4, this.originalCode.split('\n').length);
    this.textarea.spellcheck = false;
    this.editSurfaceEl.appendChild(this.textarea);

    this.previewEl = document.createElement('div');
    this.previewEl.className = 'mermaid-block-preview';
    this.editSurfaceEl.appendChild(this.previewEl);
    if (this.previewSnapshotEl) {
      this.previewEl.appendChild(this.previewSnapshotEl);
      if (this.previewEl.querySelector('svg')) this.bindPreviewViewport();
    }

    if (!this.previewSnapshotEl || !snapshotCurrent) void this.updatePreview();

    this.textarea.addEventListener('input', () => {
      this.autoResizeTextarea();
      this.schedulePreviewUpdate();
    });
    this.textarea.addEventListener('keydown', (event) => this.handleKeyDown(event));
    this.textarea.addEventListener('blur', (event) => {
      if (!this.dom.contains(event.relatedTarget as Node)) this.exitEdit(true);
    });
    this.editSurfaceEl.addEventListener('focusout', (event) => {
      if (this.editing && !this.dom.contains(event.relatedTarget as Node)) this.exitEdit(true);
    });

    requestAnimationFrame(() => {
      if (!this.textarea || !this.editing || this.destroyed || !this.view.editable) return;
      this.textarea.focus({ preventScroll: true });
      const pos = caret === 'end' ? this.textarea.value.length : 0;
      this.textarea.setSelectionRange(pos, pos);
    });
    return true;
  }

  private exitEdit(save: boolean, selection: 'before' | 'after' | 'preserve' = 'preserve'): void {
    if (!this.editing) return;
    if (this.destroyed || this.view.isDestroyed) {
      this.cleanupEdit();
      return;
    }
    if (!this.view.editable) {
      save = false;
      selection = 'preserve';
    }

    const newCode = save && this.textarea ? this.textarea.value : this.originalCode;
    const oldCode = this.node.attrs.code as string;
    const pos = this.getPos();

    this.cleanupEdit();

    let tr = this.view.state.tr;
    if (save && newCode !== oldCode) {
      tr = tr.setNodeMarkup(pos, null, { ...this.node.attrs, code: newCode });
    }

    if (selection === 'before') {
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(pos), -1));
    } else if (selection === 'after') {
      const nextPos = Math.min(pos + this.node.nodeSize, tr.doc.content.size);
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(nextPos), 1));
    }

    if (tr.docChanged || selection !== 'preserve') {
      this.view.dispatch(tr);
    }
    if (!tr.docChanged) void this.renderMermaid();
    if (selection === 'before' || selection === 'after') {
      this.view.focus();
    }
  }

  private cleanupEdit(): void {
    unregisterActiveEdit(this.view, this.activeEditExitFn);
    this.disposeViewport();
    this.editing = false;
    this.clearPreviewDebounce();
    this.previewRenderId += 1;
    this.clearPendingRender();
    this.renderedCode = null;
    this.dom.classList.remove('is-editing');
    this.editSurfaceEl?.remove();
    this.textarea = null;
    this.previewEl = null;
    this.previewSnapshotEl = null;
    this.editSurfaceEl = null;
  }

  private takeRenderedPreviewSnapshot(className: string): HTMLElement | null {
    const renderedContent = this.dom.querySelector<HTMLElement>('.mermaid-block-rendered');
    if (!renderedContent?.hasChildNodes() && !this.dom.hasChildNodes()) return null;

    const previewEl = document.createElement('div');
    previewEl.className = className;
    previewEl.setAttribute('aria-hidden', 'true');

    if (renderedContent) {
      while (renderedContent.firstChild) {
        previewEl.appendChild(renderedContent.firstChild);
      }
      this.dom.replaceChildren();
      return previewEl;
    }

    while (this.dom.firstChild) {
      const child = this.dom.firstChild;
      if (
        child instanceof HTMLElement &&
        child.classList.contains('mermaid-block-fullscreen-button')
      ) {
        child.remove();
      } else {
        previewEl.appendChild(child);
      }
    }

    return previewEl;
  }

  private handleKeyDown(event: KeyboardEvent): void {
    if (!this.textarea || this.destroyed || this.view.isDestroyed) return;
    if (!this.view.editable) {
      event.preventDefault();
      this.exitEdit(false);
      return;
    }

    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && event.shiftKey) {
      event.preventDefault();
      const pos = this.getPos();
      this.exitEdit(true, 'preserve');
      const paragraph = this.view.state.schema.nodes.paragraph.create();
      const tr = this.view.state.tr.insert(pos, paragraph);
      this.view.dispatch(tr.setSelection(TextSelection.create(tr.doc, pos + 1)));
      this.view.focus();
      return;
    }

    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      const pos = this.getPos();
      this.exitEdit(true, 'preserve');
      const afterPos = pos + this.node.nodeSize;
      if (afterPos <= this.view.state.doc.content.size) {
        const paragraph = this.view.state.schema.nodes.paragraph.create();
        const tr = this.view.state.tr.insert(afterPos, paragraph);
        this.view.dispatch(tr.setSelection(TextSelection.create(tr.doc, afterPos + 1)));
        this.view.focus();
      }
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      this.exitEdit(false);
      return;
    }

    if (event.key === 'ArrowDown' && !event.shiftKey) {
      const { selectionStart, value } = this.textarea;
      if (!value.slice(selectionStart).includes('\n')) {
        event.preventDefault();
        this.exitEdit(true, 'after');
      }
      return;
    }

    if (event.key === 'ArrowUp' && !event.shiftKey) {
      const { selectionStart, value } = this.textarea;
      if (!value.slice(0, selectionStart).includes('\n')) {
        event.preventDefault();
        this.exitEdit(true, 'before');
      }
    }
  }

  private autoResizeTextarea(): void {
    if (!this.textarea) return;
    this.textarea.rows = Math.max(4, this.textarea.value.split('\n').length);
  }

  private updatePreview(): Promise<void> {
    if (
      this.destroyed || this.view.isDestroyed || !this.editing || !this.previewEl || !this.textarea
    ) return Promise.resolve();
    this.clearPendingRender();
    const id = ++this.previewRenderId;
    const code = this.textarea.value;
    const renderer = getDiagramRenderer();
    if (!code.trim()) {
      this.setPreviewContent('', { error: false, renderId: id });
      return Promise.resolve();
    }
    if (!renderer) {
      this.setPreviewContent('(diagram renderer unavailable)', { error: true, renderId: id });
      return Promise.resolve();
    }

    const controller = new AbortController();
    this.renderAbort = controller;
    return this.trackPendingRender((async () => {
      try {
        const result = await renderer.renderMermaid(code, { theme: this.getTheme(), signal: controller.signal });
        if (
          this.destroyed || this.view.isDestroyed || !this.editing ||
          !this.previewEl || id !== this.previewRenderId || controller.signal.aborted
        ) return;
        if (result.error) {
          this.setPreviewContent(result.error, { error: true, renderId: id });
        } else {
          this.setPreviewContent(normalizeMermaidSvgSize(result.svg), {
            error: false,
            html: true,
            renderId: id,
          });
        }
      } catch (error) {
        if (
          this.destroyed || this.view.isDestroyed || !this.editing ||
          !this.previewEl || id !== this.previewRenderId
        ) return;
        this.setPreviewContent(error instanceof Error ? error.message : t.mermaidRenderFailed(), {
          error: true,
          renderId: id,
        });
      }
    })());
  }

  private schedulePreviewUpdate(): void {
    this.clearPreviewDebounce();
    this.previewRenderId += 1;
    this.previewDebounceTimer = setTimeout(() => {
      this.previewDebounceTimer = null;
      void this.updatePreview();
    }, MermaidBlockNodeView.PREVIEW_DEBOUNCE_MS);
  }

  private clearPreviewDebounce(): void {
    if (this.previewDebounceTimer !== null) {
      clearTimeout(this.previewDebounceTimer);
      this.previewDebounceTimer = null;
    }
  }

  private setPreviewContent(
    content: string,
    options: { error: boolean; html?: boolean; renderId: number },
  ): void {
    if (this.destroyed || !this.editing || !this.previewEl || options.renderId !== this.previewRenderId) return;

    this.disposeViewport();
    this.previewEl.classList.toggle('is-error', options.error);
    const renderedEl = document.createElement('div');
    renderedEl.className = 'mermaid-block-preview-render';
    if (options.html) renderedEl.innerHTML = content;
    else renderedEl.textContent = content;
    this.previewEl.replaceChildren(renderedEl);
    this.previewSnapshotEl = null;
    if (options.html && renderedEl.querySelector('svg')) {
      normalizeRenderedMermaidViewport(renderedEl);
      this.bindPreviewViewport();
    }
  }

  private viewLabels() {
    return {
      mode: t.mermaidViewMode(),
      readable: t.mermaidReadable(),
      fit: t.mermaidFit(),
      original: t.mermaidOriginal(),
      custom: t.mermaidCustom(),
    };
  }

  private bindViewport(
    viewport: HTMLElement,
    toolbar: HTMLElement,
    kind: 'card' | 'preview',
  ): void {
    this.viewportBinding = bindMermaidViewport(viewport, toolbar, this.viewLabels(), {
      kind,
      mode: this.viewMode,
      position: this.viewPosition,
      onModeChange: (mode) => {
        this.viewMode = mode;
      },
    });
  }

  private bindPreviewViewport(): void {
    if (!this.previewEl) return;
    this.previewToolbar = document.createElement('div');
    this.previewToolbar.className = 'mermaid-view-toolbar';
    this.previewEl.before(this.previewToolbar);
    this.bindViewport(this.previewEl, this.previewToolbar, 'preview');
  }

  private disposeViewport(): void {
    this.viewPosition = this.viewportBinding?.position() ?? this.viewPosition;
    this.viewportBinding?.dispose();
    this.viewportBinding = null;
    this.previewToolbar?.remove();
    this.previewToolbar = null;
  }

  private renderError(error: string, code: string): void {
    this.disposeFullscreen();
    this.dom.textContent = '';
    const errorEl = document.createElement('div');
    errorEl.className = 'mermaid-block-error';
    errorEl.textContent = error;
    const sourceEl = document.createElement('pre');
    sourceEl.className = 'mermaid-block-source';
    sourceEl.textContent = `\`\`\`mermaid\n${code}\n\`\`\``;
    this.dom.append(errorEl, sourceEl);
  }

  private renderEmptyDiagram(): void {
    this.disposeFullscreen();
    this.dom.textContent = '';
    const emptyEl = document.createElement('div');
    emptyEl.className = 'mermaid-block-empty';
    this.dom.appendChild(emptyEl);
  }

  private renderDisplayDiagram(svg: string): void {
    this.disposeFullscreen();
    this.dom.textContent = '';

    const renderedEl = document.createElement('div');
    renderedEl.className = 'mermaid-block-rendered';
    renderedEl.innerHTML = svg;
    this.dom.appendChild(renderedEl);
    normalizeRenderedMermaidViewport(renderedEl);

    const toolbar = document.createElement('div');
    toolbar.className = 'mermaid-view-toolbar';
    renderedEl.before(toolbar);
    this.bindViewport(renderedEl, toolbar, 'card');
    this.fullscreenBinding = bindMermaidFullscreen(toolbar, renderedEl, {
      ...this.viewLabels(),
      open: t.fullscreenDiagram(),
      dialog: t.fullscreenDiagramPreview(),
      close: t.closeFullscreenDiagram(),
      zoomIn: t.increaseZoom(),
      zoomOut: t.decreaseZoom(),
      reset: t.reset(),
    });
  }

  private closeFullscreen(): void {
    this.fullscreenBinding?.close();
  }

  private disposeFullscreen(): void {
    this.disposeViewport();
    this.fullscreenBinding?.dispose();
    this.fullscreenBinding = null;
  }

  private getTheme(): MermaidThemeDefinition {
    return MermaidBlockNodeView.currentTheme;
  }
}
