import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { getMathRenderer } from '../renderers';
import { registerActiveEdit, unregisterActiveEdit } from './activeEditRegistry';

export class MathInlineNodeView {
  private static nextKeyboardCursorSide: 'start' | 'end' | null = null;
  private static instantEditMode = false;
  private static keyboardEntries = new WeakMap<EditorView, 'start' | 'end'>();
  private static instantEditViews = new WeakSet<EditorView>();

  dom: HTMLElement;

  private node: ProseMirrorNode;
  private view: EditorView;
  private getPos: () => number;
  private editing = false;
  private destroyed = false;
  private suppressNextSelectAutoEdit = false;
  private originalTex = '';
  private renderId = 0;
  private previewRenderId = 0;
  private pendingPointerRatio: number | null = null;
  private activeEditExitFn: (() => void) | null = null;

  // 编辑态 DOM 引用
  private input: HTMLInputElement | null = null;
  private previewCard: HTMLElement | null = null;
  private previewContent: HTMLElement | null = null;

  // 滚动/窗口大小监听句柄
  private positionHandler: (() => void) | null = null;

  constructor(node: ProseMirrorNode, view: EditorView, getPos: () => number, suppressInitialEdit = false) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    this.suppressNextSelectAutoEdit = suppressInitialEdit;

    this.dom = document.createElement('span');
    this.dom.className = 'math-inline';
    this.dom.contentEditable = 'false';
    this.dom.setAttribute('data-tex', node.attrs.tex as string);
    this.dom.addEventListener('mousedown', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      this.pendingPointerRatio = getPointerRatio(event.clientX, this.dom.getBoundingClientRect());
      window.setTimeout(() => this.enterEdit(), 0);
    });
    this.dom.addEventListener('click', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      this.pendingPointerRatio = getPointerRatio(event.clientX, this.dom.getBoundingClientRect());
      this.enterEdit();
    });
    this.dom.textContent = `$${node.attrs.tex}$`;
    this.renderKaTeX();

    // 检查是否需要立即进入编辑态
    if (MathInlineNodeView.instantEditViews.has(view) || MathInlineNodeView.instantEditMode) {
      MathInlineNodeView.instantEditViews.delete(view);
      MathInlineNodeView.instantEditMode = false;
      this.pendingPointerRatio = 1; // 光标在末尾
      // 延迟一帧确保 DOM 已渲染
      requestAnimationFrame(() => {
        this.enterEdit();
      });
    }
  }

  static requestKeyboardEntry(cursorSide: 'start' | 'end', view?: EditorView): void {
    if (view) this.keyboardEntries.set(view, cursorSide);
    else this.nextKeyboardCursorSide = cursorSide;
  }

  static requestInstantEdit(view?: EditorView): void {
    if (view) this.instantEditViews.add(view);
    else this.instantEditMode = true;
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) return false;
    this.node = node;
    if (!this.editing) {
      this.renderKaTeX();
    }
    return true;
  }

  selectNode(): void {
    this.dom.classList.add('ProseMirror-selectednode');
    if (this.suppressNextSelectAutoEdit) {
      this.suppressNextSelectAutoEdit = false;
      return;
    }
    const cursorSide = MathInlineNodeView.keyboardEntries.get(this.view) ?? MathInlineNodeView.nextKeyboardCursorSide;
    MathInlineNodeView.keyboardEntries.delete(this.view);
    if (cursorSide === 'start') {
      this.pendingPointerRatio = 0;
    } else if (cursorSide === 'end') {
      this.pendingPointerRatio = 1;
    }
    MathInlineNodeView.nextKeyboardCursorSide = null;
    this.enterEdit();
  }

  deselectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
    if (this.editing) {
      this.exitEdit(true);
    }
  }

  stopEvent(event: Event): boolean {
    if (this.editing) {
      if (this.dom.contains(event.target as Node)) return true;
      if (this.previewCard && this.previewCard.contains(event.target as Node)) return true;
    }
    return false;
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.destroyed = true;
    this.cleanupEdit();
  }

  // ---- KaTeX 渲染 ----

  private async renderKaTeX(): Promise<void> {
    const id = ++this.renderId;
    const tex = this.node.attrs.tex as string;
    this.dom.setAttribute('data-tex', tex);
    this.dom.textContent = tex ? `$${tex}$` : '';
    if (!tex) return;

    const mathRenderer = getMathRenderer();
    if (!mathRenderer) return;

    try {
      const result = await mathRenderer.render(tex, { displayMode: false });
      if (id !== this.renderId) return; // 放弃过期渲染
      if (result.error) {
        this.dom.textContent = `$${tex}$`;
      } else {
        this.dom.innerHTML = result.html;
      }
    } catch {
      if (id !== this.renderId) return;
      this.dom.textContent = `$${tex}$`;
    }
  }

  // ---- 编辑态管理 ----

  private enterEdit(): void {
    if (this.editing || this.destroyed || this.view.isDestroyed) return;

    this.editing = true;
    this.activeEditExitFn = () => this.exitEdit(true, 'preserve');
    registerActiveEdit(this.view, this.activeEditExitFn);

    this.originalTex = this.node.attrs.tex as string;
    this.dom.classList.add('is-editing');
    this.dom.classList.remove('ProseMirror-selectednode');

    // 步骤1：原位替换为 input
    this.input = document.createElement('input');
    this.input.type = 'text';
    this.input.className = 'math-inline-input';
    this.input.contentEditable = 'true';
    this.input.value = this.originalTex;
    this.updateInputWidth();
    this.dom.textContent = '';
    this.dom.appendChild(this.input);

    // 步骤2：创建浮动预览卡片（挂 document.body，fixed 定位）
    this.previewCard = document.createElement('div');
    this.previewCard.className = 'math-inline-preview';
    this.previewCard.style.position = 'fixed';
    this.previewCard.style.zIndex = '1000';
    this.previewCard.addEventListener('mousedown', (e) => {
      e.preventDefault(); // 阻止焦点离开 input
      this.input?.focus();
    });

    this.previewContent = document.createElement('div');
    this.previewContent.className = 'math-inline-preview-content';
    this.previewCard.appendChild(this.previewContent);
    this.previewCard.style.visibility = 'hidden'; // 定位前隐藏，避免闪烁
    document.body.appendChild(this.previewCard);

    // 步骤3：定位预览卡片（延迟一帧，等 DOM 布局稳定后再定位，
    // 避免从前一个编辑态退出时的 DOM 回流导致坐标偏移）
    this.positionHandler = () => this.updatePreviewPosition();
    window.addEventListener('scroll', this.positionHandler, true);
    window.addEventListener('resize', this.positionHandler);

    // 步骤4：input 事件监听
    this.input.addEventListener('input', () => {
      this.updateInputWidth();
      this.updatePreview();
      this.updatePreviewPosition();
    });
    this.input.addEventListener('keydown', (e) => this.handleKeyDown(e));
    this.input.addEventListener('blur', (e) => {
      if (
        this.previewCard &&
        e.relatedTarget instanceof Node &&
        this.previewCard.contains(e.relatedTarget)
      ) {
        return;
      }
      this.exitEdit(true);
    });

    // 步骤5：初始预览渲染
    this.updatePreview();

    // 步骤6：聚焦 input + 定位预览卡片
    requestAnimationFrame(() => {
      if (!this.input) return;
      this.input.focus({ preventScroll: true });
      const cursorPos = this.resolveInitialCursorPos();
      this.input.setSelectionRange(cursorPos, cursorPos);
      this.pendingPointerRatio = null;

      // DOM 布局已稳定，此时定位预览卡片坐标准确
      this.updatePreviewPosition();
      if (this.previewCard) {
        this.previewCard.style.visibility = '';
      }
    });
  }

  private exitEdit(save: boolean, cursorSide: 'before' | 'after' | 'preserve' = 'after'): void {
    if (!this.editing) return;

    const newTex = save && this.input ? this.input.value : this.originalTex;
    const oldTex = this.node.attrs.tex as string;
    const pos = this.getPos();

    this.cleanupEdit();

    let tr = this.view.state.tr;
    if (save && newTex !== oldTex) {
      // 一次性提交：修改 node attrs + 移动光标到节点后
      tr = tr.setNodeMarkup(pos, null, { tex: newTex });
    }

    if (cursorSide !== 'preserve') {
      const cursorPos = cursorSide === 'before' ? pos : pos + 1;
      const bias = cursorSide === 'before' ? -1 : 1;
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(cursorPos), bias));
    }
    if (tr.docChanged || cursorSide !== 'preserve') this.view.dispatch(tr);

    // 恢复编辑器焦点，否则光标不可见
    if (cursorSide !== 'preserve') this.view.focus();
  }

  private cleanupEdit(): void {
    this.editing = false;
    if (this.activeEditExitFn) {
      unregisterActiveEdit(this.view, this.activeEditExitFn);
      this.activeEditExitFn = null;
    }
    this.dom.classList.remove('is-editing');

    if (this.positionHandler) {
      window.removeEventListener('scroll', this.positionHandler, true);
      window.removeEventListener('resize', this.positionHandler);
      this.positionHandler = null;
    }

    if (this.previewCard) {
      this.previewCard.remove();
      this.previewCard = null;
      this.previewContent = null;
    }

    this.input = null;

    // 恢复 KaTeX 渲染（renderId 计数器保护，若 save 路径的 dispatch → update 触发新渲染则自动覆盖）
    this.renderKaTeX();
  }

  // ---- 键盘处理 ----

  private handleKeyDown(e: KeyboardEvent): void {
    if (!this.input) return;

    if (e.key === 'Enter') {
      e.preventDefault();
      this.exitEdit(true);
      return;
    }

    if (e.key === 'Escape') {
      e.preventDefault();
      this.exitEdit(false);
      return;
    }

    // 左右箭头在 input 内部正常移动字符；只在边界时退出编辑态
    if (e.key === 'ArrowLeft' && this.input.selectionStart === 0 && this.input.selectionEnd === 0) {
      e.preventDefault();
      this.exitEdit(true, 'before');
      return;
    }

    if (
      e.key === 'ArrowRight' &&
      this.input.selectionStart === this.input.value.length &&
      this.input.selectionEnd === this.input.value.length
    ) {
      e.preventDefault();
      this.exitEdit(true, 'after');
      return;
    }
  }

  // ---- 预览卡片 ----

  private updatePreviewPosition(): void {
    if (!this.previewCard || !this.input) return;

    const rect = this.input.getBoundingClientRect();
    const isVisible = rect.bottom >= 0 && rect.top < window.innerHeight;

    if (!isVisible) {
      this.previewCard.style.display = 'none';
      return;
    }

    this.previewCard.style.display = '';
    this.previewCard.style.top = `${rect.bottom + 4}px`;
    this.previewCard.style.left = `${rect.left}px`;
    this.previewCard.style.maxWidth = `${Math.min(window.innerWidth - rect.left - 16, 600)}px`;
  }

  private async updatePreview(): Promise<void> {
    if (!this.previewContent || !this.input) return;

    const id = ++this.previewRenderId;
    const tex = this.input.value.trim();
    if (!tex) {
      this.previewContent.textContent = '';
      return;
    }

    const mathRenderer = getMathRenderer();
    if (!mathRenderer) {
      this.previewContent.innerHTML =
        '<span class="math-inline-preview-error">Math renderer unavailable</span>';
      return;
    }

    try {
      const result = await mathRenderer.render(tex, { displayMode: false });
      if (id !== this.previewRenderId || !this.previewContent) return;
      if (result.error) {
        this.previewContent.innerHTML = `<span class="math-inline-preview-error">${escapeHtml(result.error)}</span>`;
      } else {
        this.previewContent.innerHTML = result.html;
      }
    } catch {
      this.previewContent.innerHTML = '<span class="math-inline-preview-error">KaTeX error</span>';
    }
  }

  private updateInputWidth(): void {
    if (!this.input) return;
    this.input.style.width = `${Math.max(4, this.input.value.length + 1)}ch`;
  }

  private resolveInitialCursorPos(): number {
    const ratio = this.pendingPointerRatio;
    const valueLength = this.input?.value.length ?? 0;
    if (ratio === null) return valueLength;
    return Math.max(0, Math.min(valueLength, Math.round(ratio * valueLength)));
  }
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function getPointerRatio(clientX: number, rect: DOMRect): number {
  if (rect.width <= 0) return 1;
  return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
}
