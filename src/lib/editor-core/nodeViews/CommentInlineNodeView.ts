import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection } from 'prosemirror-state';
import type { EditorView } from 'prosemirror-view';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import { registerActiveEdit, unregisterActiveEdit } from './activeEditRegistry';

const COMMENT_INLINE_PREVIEW_LENGTH = 12;
const COMMENT_INLINE_MIN_EDIT_WIDTH_PX = 16;
const COMMENT_INLINE_MAX_EDIT_WIDTH_PX = 640;
const COMMENT_INLINE_EDIT_CARET_PADDING_PX = 6;

/** 行内 Markdown 注释 NodeView：展示为正文内的注释标签，点击后原位编辑注释内容。 */
export class CommentInlineNodeView {
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
  private originalContent = '';
  private input: HTMLInputElement | null = null;
  private activeEditExitFn: (() => void) | null = null;
  private unsubscribeLocale: () => void = () => undefined;

  constructor(node: ProseMirrorNode, view: EditorView, getPos: () => number, suppressInitialEdit = false) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    this.suppressNextSelectAutoEdit = suppressInitialEdit;

    this.dom = document.createElement('span');
    this.dom.className = 'comment-inline';
    this.dom.contentEditable = 'false';
    this.dom.tabIndex = 0;
    this.dom.setAttribute('role', 'button');
    this.syncAccessibleLabels();
    this.dom.addEventListener('mousedown', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      window.setTimeout(() => this.enterEdit(), 0);
    });
    this.dom.addEventListener('click', (event) => {
      if (this.editing) return;
      event.preventDefault();
      event.stopPropagation();
      this.enterEdit();
    });
    this.dom.addEventListener('keydown', (event) => {
      if (this.editing) return;
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        this.enterEdit();
      }
    });
    this.unsubscribeLocale = onInterfaceLocaleChanged(() => {
      this.syncAccessibleLabels();
      if (this.editing && this.input) {
        this.input.setAttribute('aria-label', t.editInlineComment());
      } else {
        this.renderDisplay();
      }
    });

    this.renderDisplay();

    if (CommentInlineNodeView.instantEditViews.has(view) || CommentInlineNodeView.instantEditMode) {
      CommentInlineNodeView.instantEditViews.delete(view);
      CommentInlineNodeView.instantEditMode = false;
      requestAnimationFrame(() => this.enterEdit());
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
    this.syncAccessibleLabels();
    if (!this.editing) this.renderDisplay();
    return true;
  }

  selectNode(): void {
    this.dom.classList.add('ProseMirror-selectednode');
    if (this.suppressNextSelectAutoEdit) {
      this.suppressNextSelectAutoEdit = false;
      return;
    }
    this.enterEdit();
  }

  deselectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
    if (this.editing) this.exitEdit(true);
  }

  stopEvent(event: Event): boolean {
    return this.editing && this.dom.contains(event.target as Node);
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.destroyed = true;
    this.unsubscribeLocale();
    this.cleanupEdit();
  }

  private renderDisplay(): void {
    this.dom.classList.remove('is-empty');
    const content = this.getContent();
    if (!content.trim()) this.dom.classList.add('is-empty');
    this.syncAccessibleLabels();

    this.dom.textContent = '';
    const label = document.createElement('span');
    label.className = 'comment-inline-label';
    label.textContent = createCommentPreviewText(content);
    this.dom.appendChild(label);
  }

  private enterEdit(): void {
    if (this.editing || this.destroyed || this.view.isDestroyed) return;

    this.editing = true;
    this.activeEditExitFn = () => this.exitEdit(true, 'preserve');
    registerActiveEdit(this.view, this.activeEditExitFn);

    this.originalContent = this.getContent();
    this.dom.classList.add('is-editing');
    this.dom.classList.remove('ProseMirror-selectednode');

    this.input = document.createElement('input');
    this.input.type = 'text';
    this.input.className = 'comment-inline-input';
    this.input.value = this.originalContent;
    this.input.setAttribute('aria-label', t.editInlineComment());
    this.updateInputWidth();

    this.dom.textContent = '';
    this.dom.appendChild(this.input);

    this.input.addEventListener('input', () => this.updateInputWidth());
    this.input.addEventListener('keydown', (event) => this.handleKeyDown(event));
    this.input.addEventListener('blur', () => this.exitEdit(true));

    requestAnimationFrame(() => {
      if (!this.input) return;
      this.input.focus({ preventScroll: true });
      const cursorSide = CommentInlineNodeView.keyboardEntries.get(this.view) ?? CommentInlineNodeView.nextKeyboardCursorSide;
      CommentInlineNodeView.keyboardEntries.delete(this.view);
      const cursorPos = cursorSide === 'start' ? 0 : this.input.value.length;
      CommentInlineNodeView.nextKeyboardCursorSide = null;
      this.input.setSelectionRange(cursorPos, cursorPos);
    });
  }

  private exitEdit(save: boolean, cursorSide: 'before' | 'after' | 'preserve' = 'after'): void {
    if (!this.editing) return;

    const nextContent =
      save && this.input ? sanitizeCommentContent(this.input.value) : this.originalContent;
    const previousContent = this.getContent();
    const pos = this.getPos();

    this.cleanupEdit();

    let tr = this.view.state.tr;
    if (save && nextContent !== previousContent) {
      tr = tr.setNodeMarkup(pos, null, { content: nextContent });
    }

    if (cursorSide !== 'preserve') {
      const cursorPos = cursorSide === 'before' ? pos : pos + 1;
      const bias = cursorSide === 'before' ? -1 : 1;
      tr = tr.setSelection(TextSelection.near(tr.doc.resolve(cursorPos), bias));
    }
    if (tr.docChanged || cursorSide !== 'preserve') this.view.dispatch(tr);
    if (cursorSide !== 'preserve') this.view.focus();
  }

  private cleanupEdit(): void {
    this.editing = false;
    if (this.activeEditExitFn) {
      unregisterActiveEdit(this.view, this.activeEditExitFn);
      this.activeEditExitFn = null;
    }
    this.dom.classList.remove('is-editing');
    this.input = null;
    this.renderDisplay();
  }

  private handleKeyDown(event: KeyboardEvent): void {
    if (!this.input) return;

    if (event.key === 'Enter') {
      event.preventDefault();
      this.exitEdit(true);
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      this.exitEdit(false);
      return;
    }

    if (
      event.key === 'ArrowLeft' &&
      this.input.selectionStart === 0 &&
      this.input.selectionEnd === 0
    ) {
      event.preventDefault();
      this.exitEdit(true, 'before');
      return;
    }

    if (
      event.key === 'ArrowRight' &&
      this.input.selectionStart === this.input.value.length &&
      this.input.selectionEnd === this.input.value.length
    ) {
      event.preventDefault();
      this.exitEdit(true, 'after');
    }
  }

  private updateInputWidth(): void {
    if (!this.input) return;
    const contentWidth = measureInputContentWidth(this.input);
    const nextWidth = Math.min(
      COMMENT_INLINE_MAX_EDIT_WIDTH_PX,
      Math.max(
        COMMENT_INLINE_MIN_EDIT_WIDTH_PX,
        contentWidth + COMMENT_INLINE_EDIT_CARET_PADDING_PX,
      ),
    );
    this.input.style.width = `${nextWidth}px`;
  }

  private getContent(): string {
    return String(this.node.attrs.content ?? '');
  }

  private createAriaLabel(): string {
    const content = this.getContent().trim();
    return content ? t.inlineCommentWithContent({ content }) : t.inlineComment();
  }

  private syncAccessibleLabels(): void {
    this.dom.setAttribute('aria-label', this.createAriaLabel());
    this.dom.title = this.getContent().trim() || t.emptyComment();
  }
}

function sanitizeCommentContent(content: string): string {
  return content.replace(/-->/g, '-- >');
}

function createCommentPreviewText(content: string): string {
  const normalized = content.trim();
  if (!normalized) return t.emptyComment();

  const chars = Array.from(normalized);
  const preview = chars.slice(0, COMMENT_INLINE_PREVIEW_LENGTH).join('');
  return `${preview}${chars.length > COMMENT_INLINE_PREVIEW_LENGTH ? '…' : ''}`;
}

function measureInputContentWidth(input: HTMLInputElement): number {
  const computed = window.getComputedStyle(input);
  const measure = document.createElement('span');
  measure.className = 'comment-inline-measure';
  measure.textContent = input.value || ' ';
  measure.style.position = 'absolute';
  measure.style.visibility = 'hidden';
  measure.style.pointerEvents = 'none';
  measure.style.whiteSpace = 'pre';
  measure.style.font = computed.font;
  measure.style.letterSpacing = computed.letterSpacing;
  measure.style.textTransform = computed.textTransform;
  document.body.appendChild(measure);
  const width = Math.ceil(measure.getBoundingClientRect().width);
  measure.remove();
  return width;
}
