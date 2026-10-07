import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { NodeSelection, TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { getMathRenderer } from '../renderers';
import { registerActiveEdit, unregisterActiveEdit } from './activeEditRegistry';

/**
 * math_block 节点的 NodeView —— 渲染态
 *
 * 职责：
 * 1. 将 math_block 节点渲染为 KaTeX 公式（displayMode: true）
 * 2. 点击公式块进入编辑态（textarea 多行编辑）
 * 3. Ctrl+Enter 保存退出 / Esc 放弃退出
 */
export class MathBlockNodeView {
  private static instances = new Set<MathBlockNodeView>();
  private static pendingKeyboardEntry: { caret: 'start' | 'end'; expiresAt: number } | null = null;
  private static keyboardEntries = new WeakMap<EditorView, { caret: 'start' | 'end'; expiresAt: number }>();

  dom: HTMLElement;

  private node: ProseMirrorNode;
  private view: EditorView;
  private getPos: () => number;
  private renderId = 0;

  // 编辑态相关
  private editing = false;
  private destroyed = false;
  private readonly activeEditExitFn = () => this.exitEdit(true, 'preserve');
  private originalTex = '';
  private textarea: HTMLTextAreaElement | null = null;
  private previewEl: HTMLElement | null = null;
  private suppressNextSelectAutoEdit = false;
  // 首次创建时自动进入编辑态（如 InputRule 从 $$ 创建的空公式块）
  private needsAutoEdit = false;

  constructor(node: ProseMirrorNode, view: EditorView, getPos: () => number, suppressInitialEdit = false) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    this.suppressNextSelectAutoEdit = suppressInitialEdit;
    MathBlockNodeView.instances.add(this);

    this.dom = document.createElement('div');
    this.dom.className = 'math-block';
    this.dom.contentEditable = 'false';
    this.dom.setAttribute('data-tex', node.attrs.tex as string);

    // 初始 fallback 文本
    this.dom.textContent = `$$\n${node.attrs.tex}\n$$`;

    // 点击进入编辑态
    this.dom.addEventListener('click', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      this.enterEdit();
    });

    // 空 tex 说明是 InputRule 新建的公式块，标记首次选中时自动进入编辑态
    if (!(node.attrs.tex as string).trim()) {
      this.needsAutoEdit = true;
    }

    this.renderKaTeX();
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
    if (this.editing) {
      this.dom.classList.remove('ProseMirror-selectednode');
      return;
    }
    if (this.suppressNextSelectAutoEdit) {
      this.suppressNextSelectAutoEdit = false;
      return;
    }

    // 公式块没有有意义的“只选中”中间态：键盘导航到达后立即进入编辑态，避免需要再按第二下。
    this.needsAutoEdit = false;
    const keyboardEntry = MathBlockNodeView.consumeKeyboardEntry(this.view);
    this.enterEdit(keyboardEntry ?? 'start');
  }

  deselectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
    if (this.editing) {
      this.exitEdit(true, 'preserve');
    }
  }

  stopEvent(event: Event): boolean {
    // 编辑态内拦截所有事件，防止 ProseMirror 处理
    if (this.editing) {
      if (this.dom.contains(event.target as Node)) return true;
    }
    return false;
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.destroyed = true;
    this.cleanupEdit();
    MathBlockNodeView.instances.delete(this);
  }

  // ---- KaTeX 渲染 ----

  private async renderKaTeX(): Promise<void> {
    const id = ++this.renderId;
    const tex = this.node.attrs.tex as string;
    this.dom.setAttribute('data-tex', tex);

    if (this.editing) return;

    if (!tex) {
      this.dom.textContent = '';
      return;
    }

    const mathRenderer = getMathRenderer();
    if (!mathRenderer) {
      this.dom.textContent = `$$\n${tex}\n$$`;
      return;
    }

    try {
      const result = await mathRenderer.render(tex, { displayMode: true });
      if (id !== this.renderId || this.editing) return; // 放弃过期渲染
      if (result.error) {
        this.dom.textContent = `$$\n${tex}\n$$`;
        this.dom.style.color = 'var(--md-editor-warning)';
      } else {
        this.dom.innerHTML = result.html;
        this.dom.style.color = '';
      }
    } catch {
      if (id !== this.renderId || this.editing) return;
      this.dom.textContent = `$$\n${tex}\n$$`;
    }
  }

  // ---- 编辑态管理 ----

  private enterEdit(caret: 'start' | 'end' = 'start'): void {
    if (this.editing || this.destroyed || this.view.isDestroyed) return;

    registerActiveEdit(this.view, this.activeEditExitFn);

    this.editing = true;
    this.renderId += 1;
    this.originalTex = this.node.attrs.tex as string;
    this.dom.classList.add('is-editing');
    this.dom.classList.remove('ProseMirror-selectednode');

    // 步骤1：清空 dom，构建编辑态布局（textarea + 预览区）
    this.dom.textContent = '';

    // textarea 编辑区
    this.textarea = document.createElement('textarea');
    this.textarea.className = 'math-block-textarea';
    this.textarea.value = this.originalTex;
    this.textarea.rows = Math.max(2, this.originalTex.split('\n').length);
    this.textarea.spellcheck = false;
    this.dom.appendChild(this.textarea);

    // 预览区（在公式块内部，不用 fixed 浮层）
    this.previewEl = document.createElement('div');
    this.previewEl.className = 'math-block-preview';
    this.dom.appendChild(this.previewEl);

    // 步骤2：textarea 事件监听
    this.textarea.addEventListener('input', () => {
      this.autoResizeTextarea();
      this.updatePreview();
    });
    this.textarea.addEventListener('keydown', (e) => this.handleKeyDown(e));
    this.textarea.addEventListener('blur', () => {
      // 点击外部保存退出
      this.exitEdit(true);
    });

    // 步骤3：初始预览渲染
    this.updatePreview();

    // 步骤4：聚焦 textarea
    requestAnimationFrame(() => {
      if (!this.textarea) return;
      this.textarea.focus({ preventScroll: true });
      const pos = caret === 'end' ? this.textarea.value.length : 0;
      this.textarea.setSelectionRange(pos, pos);
    });
  }

  static enterEditAt(view: EditorView, pos: number, caret: 'start' | 'end' = 'start'): boolean {
    for (const instance of MathBlockNodeView.instances) {
      if (instance.view !== view) continue;
      if (instance.getPos() === pos) {
        instance.enterEdit(caret);
        return true;
      }
    }
    return false;
  }

  static scheduleEnterEditAt(
    view: EditorView,
    pos: number,
    caret: 'start' | 'end' = 'start',
  ): void {
    const schedule =
      typeof requestAnimationFrame === 'function'
        ? requestAnimationFrame
        : (callback: FrameRequestCallback) => window.setTimeout(callback, 0);

    const attempt = (remaining: number) => {
      schedule(() => {
        const node = view.state.doc.nodeAt(pos);
        if (!node || node.type.name !== 'math_block') return;
        if (MathBlockNodeView.enterEditAt(view, pos, caret)) return;
        if (remaining > 0) {
          attempt(remaining - 1);
        }
      });
    };

    attempt(4);
  }

  static prepareKeyboardEntry(caret: 'start' | 'end', view?: EditorView): void {
    const entry = {
      caret,
      expiresAt: Date.now() + 600,
    };
    if (view) this.keyboardEntries.set(view, entry);
    else this.pendingKeyboardEntry = entry;
  }

  private static consumeKeyboardEntry(view: EditorView): 'start' | 'end' | null {
    const entry = this.keyboardEntries.get(view) ?? this.pendingKeyboardEntry;
    this.keyboardEntries.delete(view);
    this.pendingKeyboardEntry = null;
    if (!entry || entry.expiresAt < Date.now()) return null;
    return entry.caret;
  }

  private exitEdit(
    save: boolean,
    selection: 'node' | 'before' | 'after' | 'preserve' = 'node',
  ): void {
    if (!this.editing) return;

    const newTex = save && this.textarea ? this.textarea.value : this.originalTex;
    const oldTex = this.node.attrs.tex as string;
    const pos = this.getPos();

    this.cleanupEdit();

    let tr = this.view.state.tr;
    if (save && newTex !== oldTex) {
      // 一次性提交：修改 node attrs
      tr = tr.setNodeMarkup(pos, null, { tex: newTex });
    }

    if (selection === 'before') {
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(pos), -1));
    } else if (selection === 'after') {
      const nextPos = Math.min(pos + this.node.nodeSize, tr.doc.content.size);
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(nextPos), 1));
    } else if (selection === 'node') {
      // 恢复选中状态
      this.suppressNextSelectAutoEdit = true;
      tr = tr.setSelection(NodeSelection.create(tr.doc, pos));
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
    unregisterActiveEdit(this.view, this.activeEditExitFn);
    this.dom.classList.remove('is-editing');
    this.textarea = null;
    this.previewEl = null;

    // 恢复 KaTeX 渲染
    this.renderKaTeX();
  }

  // ---- 键盘处理 ----

  private handleKeyDown(e: KeyboardEvent): void {
    if (!this.textarea) return;

    // Shift+Ctrl+Enter：保存退出编辑态，并在上方插入新段落
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && e.shiftKey) {
      e.preventDefault();
      const pos = this.getPos();
      this.exitEdit(true, 'preserve');
      // 在公式块上方插入空段落，光标移入
      const paragraph = this.view.state.schema.nodes.paragraph.create();
      const tr = this.view.state.tr.insert(pos, paragraph);
      this.view.dispatch(tr.setSelection(TextSelection.create(tr.doc, pos + 1)));
      this.view.focus();
      return;
    }

    // Ctrl+Enter：保存退出编辑态，并在下方插入新段落
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      const pos = this.getPos();
      this.exitEdit(true, 'preserve');
      // 在公式块下方插入空段落，光标移入
      const afterPos = pos + this.node.nodeSize;
      if (afterPos <= this.view.state.doc.content.size) {
        const paragraph = this.view.state.schema.nodes.paragraph.create();
        const tr = this.view.state.tr.insert(afterPos, paragraph);
        this.view.dispatch(tr.setSelection(TextSelection.create(tr.doc, afterPos + 1)));
        this.view.focus();
      }
      return;
    }

    // Escape：放弃修改退出编辑态
    if (e.key === 'Escape') {
      e.preventDefault();
      this.exitEdit(false);
      return;
    }

    // Shift+Ctrl+M：公式块开关——取消公式块，恢复为段落
    if (e.key === 'M' && (e.ctrlKey || e.metaKey) && e.shiftKey) {
      e.preventDefault();
      const mbPos = this.getPos();
      const tex = this.textarea.value;
      const view = this.view;
      this.exitEdit(true);

      const doc = view.state.doc;
      const mbNode = doc.nodeAt(mbPos);
      if (!mbNode || mbNode.type.name !== 'math_block') return;

      const lines = tex.split('\n');
      const paragraphs: ProseMirrorNode[] = [];
      const paraType = view.state.schema.nodes.paragraph;
      for (const line of lines) {
        paragraphs.push(
          line ? paraType.create({}, view.state.schema.text(line)) : paraType.create(),
        );
      }

      const mbEnd = mbPos + mbNode.nodeSize;
      const tr = view.state.tr.replaceWith(mbPos, mbEnd, paragraphs);
      if (tex) {
        const fragSize = paragraphs.reduce((s, p) => s + p.nodeSize, 0);
        tr.setSelection(TextSelection.create(tr.doc, mbPos, mbPos + fragSize));
      } else {
        tr.setSelection(TextSelection.near(tr.doc.resolve(mbPos), 1));
      }
      view.dispatch(tr.scrollIntoView());
      view.focus();
      return;
    }

    // ArrowDown：光标在最后一行时保存并跳出到公式块之后
    if (e.key === 'ArrowDown' && !e.shiftKey) {
      const { selectionStart, value } = this.textarea;
      if (!value.slice(selectionStart).includes('\n')) {
        e.preventDefault();
        this.exitEdit(true, 'after');
        return;
      }
    }

    // ArrowUp：光标在第一行时保存并跳出到公式块之前
    if (e.key === 'ArrowUp' && !e.shiftKey) {
      const { selectionStart, value } = this.textarea;
      if (!value.slice(0, selectionStart).includes('\n')) {
        e.preventDefault();
        this.exitEdit(true, 'before');
        return;
      }
    }

    // Enter 在 textarea 内正常换行（不退出编辑态）
    // 其他按键也正常传递给 textarea
  }

  // ---- 辅助方法 ----

  private autoResizeTextarea(): void {
    if (!this.textarea) return;
    this.textarea.rows = Math.max(2, this.textarea.value.split('\n').length);
  }

  private async updatePreview(): Promise<void> {
    if (!this.previewEl || !this.textarea) return;

    const tex = this.textarea.value.trim();
    if (!tex) {
      this.previewEl.textContent = '';
      return;
    }

    const mathRenderer = getMathRenderer();
    if (!mathRenderer) {
      this.previewEl.textContent = '(math renderer unavailable)';
      return;
    }

    try {
      const result = await mathRenderer.render(tex, { displayMode: true });
      if (!this.editing || !this.previewEl) return; // 编辑态已退出
      if (result.error) {
        this.previewEl.textContent = result.error;
        this.previewEl.style.color = 'var(--md-editor-warning)';
      } else {
        this.previewEl.innerHTML = result.html;
        this.previewEl.style.color = '';
      }
    } catch {
      if (!this.editing || !this.previewEl) return;
      this.previewEl.textContent = 'KaTeX error';
    }
  }
}
