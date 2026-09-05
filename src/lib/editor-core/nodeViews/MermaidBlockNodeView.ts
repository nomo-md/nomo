import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import { getDiagramRenderer } from '../renderers';
import type { MermaidThemeDefinition } from '../../theme/types';
import {
  bindMermaidFullscreen,
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
  private previewRenderId = 0;
  private previewDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  private editing = false;
  private originalCode = '';
  private textarea: HTMLTextAreaElement | null = null;
  private previewEl: HTMLElement | null = null;
  private previewSnapshotEl: HTMLElement | null = null;
  private editSurfaceEl: HTMLElement | null = null;
  private fullscreenBinding: MermaidFullscreenBinding | null = null;
  private unsubscribeLocale: () => void = () => undefined;
  private themeObserver: IntersectionObserver | null = null;
  private themeVisible = true;
  private themeDirty = false;
  private themeRender: Promise<void> | null = null;

  constructor(node: ProseMirrorNode, view: EditorView, getPos: () => number) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    MermaidBlockNodeView.instances.add(this);

    this.dom = document.createElement('div');
    this.dom.className = 'mermaid-block';
    this.dom.contentEditable = 'false';
    this.dom.setAttribute('data-code', node.attrs.code as string);

    // 切主题不能重排整篇文档的 SVG；提前 200px 更新将进入视口的图表。
    // 未提供 IntersectionObserver 的宿主保持即时更新，不让节点永久等待。
    if (typeof IntersectionObserver === 'function') {
      this.themeObserver = new IntersectionObserver(
        ([entry]) => {
          if (!entry) return;
          this.themeVisible = entry.isIntersecting;
          if (this.themeVisible) void this.applyPendingTheme();
        },
        { rootMargin: '200px' },
      );
      this.themeObserver.observe(this.dom);
    }

    this.dom.addEventListener('click', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      this.enterEdit();
    });

    this.unsubscribeLocale = onInterfaceLocaleChanged(() => {
      this.closeFullscreen();
      if (!this.editing) {
        void this.renderMermaid();
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
      instance.themeDirty = true;
      if (instance.themeVisible) void instance.applyPendingTheme();
    }
  }

  /** 导出读取 DOM 前补齐当前编辑器的屏幕外图表，避免导出混合主题。 */
  static async flushThemeUpdates(root: HTMLElement): Promise<void> {
    await Promise.all(
      [...MermaidBlockNodeView.instances]
        .filter((instance) => root.contains(instance.dom))
        .map((instance) => instance.applyPendingTheme()),
    );
  }

  /** 只读取最新主题；连续切换时屏幕外节点不积累中间主题的渲染任务。 */
  private applyPendingTheme(): Promise<void> | null {
    if (!this.themeDirty) return this.themeRender;
    this.themeDirty = false;
    const render = (this.editing ? this.updatePreview() : this.renderMermaid()).finally(() => {
      if (this.themeRender === render) this.themeRender = null;
    });
    this.themeRender = render;
    return render;
  }

  static enterEditAt(view: EditorView, pos: number, caret: 'start' | 'end' = 'start'): boolean {
    for (const instance of MermaidBlockNodeView.instances) {
      if (instance.view !== view) continue;
      if (instance.getPos() === pos) {
        instance.enterEdit(caret);
        return true;
      }
    }
    return false;
  }

  static enterClosestEditAt(
    view: EditorView,
    pos: number,
    caret: 'start' | 'end' = 'start',
  ): boolean {
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
    closestInstance.enterEdit(caret);
    return true;
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) return false;
    this.node = node;
    if (!this.editing) {
      this.renderMermaid();
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
    if (this.editing && this.dom.contains(event.target as Node)) return true;
    return false;
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.themeObserver?.disconnect();
    this.themeDirty = false;
    this.renderId += 1;
    this.previewRenderId += 1;
    this.disposeFullscreen();
    this.cleanupEdit();
    this.unsubscribeLocale();
    MermaidBlockNodeView.instances.delete(this);
  }

  private async renderMermaid(): Promise<void> {
    const id = ++this.renderId;
    const code = this.node.attrs.code as string;
    this.dom.setAttribute('data-code', code);

    if (this.editing) return;

    if (!code.trim()) {
      this.renderEmptyDiagram();
      return;
    }

    const renderer = getDiagramRenderer();
    if (!renderer) {
      this.disposeFullscreen();
      this.dom.textContent = `\`\`\`mermaid\n${code}\n\`\`\``;
      return;
    }

    try {
      const result = await renderer.renderMermaid(code, { theme: this.getTheme() });
      if (this.editing || id !== this.renderId) return;
      if (result.error) {
        this.renderError(result.error, code);
      } else {
        this.renderDisplayDiagram(normalizeMermaidSvgSize(result.svg));
      }
    } catch (error) {
      if (this.editing || id !== this.renderId) return;
      this.renderError(error instanceof Error ? error.message : t.mermaidRenderFailed(), code);
    }
  }

  private enterEdit(caret: 'start' | 'end' = 'start'): void {
    if (this.editing) return;
    this.editing = true;
    this.renderId += 1;
    this.previewRenderId += 1;
    this.disposeFullscreen();
    this.originalCode = this.node.attrs.code as string;
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
    }

    this.textarea.addEventListener('input', () => {
      this.autoResizeTextarea();
      this.schedulePreviewUpdate();
    });
    this.textarea.addEventListener('keydown', (event) => this.handleKeyDown(event));
    this.textarea.addEventListener('blur', () => {
      this.exitEdit(true);
    });

    requestAnimationFrame(() => {
      if (!this.textarea) return;
      this.textarea.focus({ preventScroll: true });
      const pos = caret === 'end' ? this.textarea.value.length : 0;
      this.textarea.setSelectionRange(pos, pos);
    });
  }

  private exitEdit(save: boolean, selection: 'before' | 'after' | 'preserve' = 'preserve'): void {
    if (!this.editing) return;

    const newCode = save && this.textarea ? this.textarea.value : this.originalCode;
    const oldCode = this.node.attrs.code as string;
    const pos = this.getPos();

    this.cleanupEdit();

    let tr = this.view.state.tr;
    if (save && newCode !== oldCode) {
      tr = tr.setNodeMarkup(pos, null, { code: newCode });
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
    if (selection === 'before' || selection === 'after') {
      this.view.focus();
    }
  }

  private cleanupEdit(): void {
    this.editing = false;
    this.clearPreviewDebounce();
    this.previewRenderId += 1;
    this.dom.classList.remove('is-editing');
    this.textarea = null;
    this.previewEl = null;
    this.previewSnapshotEl = null;
    this.editSurfaceEl = null;
    void this.renderMermaid();
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
    if (!this.textarea) return;

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

  private async updatePreview(): Promise<void> {
    if (!this.previewEl || !this.textarea) return;
    const id = ++this.previewRenderId;
    const code = this.textarea.value;
    const renderer = getDiagramRenderer();
    if (!code.trim()) {
      this.setPreviewContent('', { error: false, renderId: id });
      return;
    }
    if (!renderer) {
      this.setPreviewContent('(diagram renderer unavailable)', { error: true, renderId: id });
      return;
    }

    try {
      const result = await renderer.renderMermaid(code, { theme: this.getTheme() });
      if (!this.editing || !this.previewEl || id !== this.previewRenderId) return;
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
      if (!this.editing || !this.previewEl || id !== this.previewRenderId) return;
      this.setPreviewContent(error instanceof Error ? error.message : t.mermaidRenderFailed(), {
        error: true,
        renderId: id,
      });
    }
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
    if (!this.previewEl || options.renderId !== this.previewRenderId) return;

    const snapshotEl = this.previewSnapshotEl;
    this.previewEl.classList.toggle('is-error', options.error);

    if (!snapshotEl) {
      this.previewEl.textContent = '';
      if (options.html) {
        this.previewEl.innerHTML = content;
        normalizeRenderedMermaidViewport(this.previewEl);
      } else {
        this.previewEl.textContent = content;
      }
      return;
    }

    const renderedEl = document.createElement('div');
    renderedEl.className = 'mermaid-block-preview-render';
    if (options.html) {
      renderedEl.innerHTML = content;
    } else {
      renderedEl.textContent = content;
    }
    this.previewEl.replaceChildren(renderedEl);
    if (options.html) {
      normalizeRenderedMermaidViewport(renderedEl);
    }
    this.previewSnapshotEl = null;
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

    this.fullscreenBinding = bindMermaidFullscreen(this.dom, renderedEl, {
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
    this.fullscreenBinding?.dispose();
    this.fullscreenBinding = null;
  }

  private getTheme(): MermaidThemeDefinition {
    return MermaidBlockNodeView.currentTheme;
  }
}
