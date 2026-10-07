import type { Node as ProseMirrorNode } from 'prosemirror-model';
import type { EditorView } from 'prosemirror-view';
import type { ImageContext, ImageResolveResult } from '../../services/render';
import { onInterfaceLocaleChanged, t } from '../../../app/i18n';
import { getImageLoader } from '../renderers';
import { registerActiveEdit, unregisterActiveEdit } from './activeEditRegistry';
import {
  mountContextMenuFactory,
  type ContextMenuItem,
} from '../plugins/contextMenu';

/**
 * Markdown 图片 NodeView。
 *
 * 职责：
 * 1. 将 Markdown image 节点解析为本地或远程预览；
 * 2. 缺失图片时显示明确占位；
 * 3. 提供与 Mermaid 图表一致的全屏查看体验。
 */
export class ImageNodeView {
  private static readonly FULLSCREEN_DEFAULT_SCALE = 1;
  private static readonly FULLSCREEN_MIN_SCALE = 0.25;
  private static readonly FULLSCREEN_MAX_SCALE = 4;
  private static readonly FULLSCREEN_SCALE_STEP = 0.1;

  dom: HTMLElement;

  private node: ProseMirrorNode;
  private renderId = 0;
  private resolved: ImageResolveResult | null = null;
  private fullscreenOverlayEl: HTMLElement | null = null;
  private fullscreenViewportEl: HTMLElement | null = null;
  private fullscreenZoomSurfaceEl: HTMLElement | null = null;
  private fullscreenZoomBadgeEl: HTMLElement | null = null;
  private fullscreenScale = ImageNodeView.FULLSCREEN_DEFAULT_SCALE;
  private fullscreenKeydown: ((event: KeyboardEvent) => void) | null = null;
  private fullscreenDrag:
    | {
        pointerId: number;
        startX: number;
        startY: number;
        scrollLeft: number;
        scrollTop: number;
      }
    | null = null;
  private sizeEditorEl: HTMLElement | null = null;
  private sizeEditorCommitFn: (() => void) | null = null;
  /** 最近一次右键菜单的鼠标位置，用于尺寸编辑器定位 */
  private lastContextMenuX = 0;
  private lastContextMenuY = 0;
  private unsubscribeLocale: () => void = () => undefined;

  constructor(
    node: ProseMirrorNode,
    private readonly view: EditorView,
    private readonly getImageContext: () => ImageContext,
  ) {
    this.node = node;
    this.dom = document.createElement('span');
    this.dom.className = 'image-node';
    this.dom.contentEditable = 'false';
    this.dom.addEventListener('mousedown', (event) => this.handleMouseDown(event));
    this.dom.addEventListener('dblclick', (event) => this.handleDblClick(event));
    this.dom.addEventListener('contextmenu', (event) => this.handleContextMenu(event));
    mountContextMenuFactory(this.dom, () => this.getContextMenuItems());
    this.unsubscribeLocale = onInterfaceLocaleChanged(() => {
      this.closeFullscreen();
      this.closeSizeEditor();
      this.render();
    });
    this.render();
  }

  update(node: ProseMirrorNode): boolean {
    if (node.type !== this.node.type) {
      return false;
    }
    const changed =
      node.attrs.src !== this.node.attrs.src ||
      node.attrs.alt !== this.node.attrs.alt ||
      node.attrs.title !== this.node.attrs.title ||
      node.attrs.align !== this.node.attrs.align ||
      node.attrs.width !== this.node.attrs.width;
    this.node = node;
    if (changed) {
      this.render();
    }
    return true;
  }

  selectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
  }

  deselectNode(): void {
    this.dom.classList.remove('ProseMirror-selectednode');
  }

  stopEvent(event: Event): boolean {
    return event.target instanceof Node && this.dom.contains(event.target);
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.closeFullscreen();
    this.closeSizeEditor();
    this.unsubscribeLocale();
    this.renderId += 1;
  }

  private render(): void {
    const renderId = ++this.renderId;
    const src = String(this.node.attrs.src ?? '');
    const alt = String(this.node.attrs.alt ?? '');
    this.dom.dataset.src = src;
    this.dom.classList.toggle('is-badge', isBadgeImageSrc(src));
    this.dom.dataset.loadingLabel = t.imageLoading();
    this.dom.replaceChildren();
    this.dom.classList.add('is-loading');

    const loader = getImageLoader();
    if (!loader) {
      this.renderImage({ src, displaySrc: src, exists: true }, renderId);
      return;
    }

    loader
      .resolve(src, this.getImageContext())
      .then((result) => this.renderImage(result, renderId))
      .catch((error) => {
        if (renderId !== this.renderId) return;
        this.renderError(error instanceof Error ? error.message : t.imageParseFailed(), alt);
      });
  }

  private renderImage(result: ImageResolveResult, renderId: number): void {
    if (renderId !== this.renderId) {
      return;
    }

    this.resolved = result;
    this.dom.classList.remove('is-loading');
    this.dom.replaceChildren();

    if (!result.exists) {
      this.renderError(result.error ?? t.imageFileMissing(), String(this.node.attrs.alt ?? ''));
      return;
    }

    const img = document.createElement('img');
    if (/^https?:/i.test(result.src.trim())) {
      // 避免应用来源触发远程图床的防盗链限制，须在设置 src 前生效。
      img.referrerPolicy = 'no-referrer';
    }
    img.src = result.displaySrc;
    img.alt = String(this.node.attrs.alt ?? '');
    if (this.node.attrs.title) {
      img.title = String(this.node.attrs.title);
    }
    // 应用对齐样式：block + margin auto 实现段落内图片对齐
    const align = this.node.attrs.align as string | null;
    if (align) {
      this.dom.style.display = 'block';
      this.dom.style.width = 'fit-content';
      this.dom.style.marginLeft = align === 'right' ? 'auto' : align === 'center' ? 'auto' : '0';
      this.dom.style.marginRight = align === 'left' ? 'auto' : align === 'center' ? 'auto' : '0';
    } else {
      this.dom.style.display = '';
      this.dom.style.width = '';
      this.dom.style.marginLeft = '';
      this.dom.style.marginRight = '';
    }
    // 应用宽度
    const width = this.node.attrs.width as string | null;
    if (width) {
      img.style.width = /^\d+$/.test(width) ? `${width}px` : width;
      img.style.height = 'auto';
    } else {
      img.style.width = '';
      img.style.height = '';
    }
    img.loading = 'lazy';
    img.decoding = 'async';
    img.addEventListener('error', () => {
      if (renderId !== this.renderId) return;
      this.renderError(t.imageLoadFailed(), String(this.node.attrs.alt ?? ''));
    });

    const button = this.createIconButton(
      'image-node-fullscreen-button',
      t.fullscreenImage(),
      t.enlarge(),
      'maximize',
    );
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      this.openFullscreen();
    });

    this.dom.append(img, button);
  }

  private handleMouseDown(event: MouseEvent): void {
    if ((event.target as HTMLElement | null)?.closest('button')) {
      return;
    }
    event.preventDefault();
  }

  /** 双击图片打开全屏查看 */
  private handleDblClick(event: MouseEvent): void {
    if ((event.target as HTMLElement | null)?.closest('button')) {
      return;
    }
    event.preventDefault();
    this.openFullscreen();
  }

  /**
   * 处理右键菜单事件。
   * 由于 stopEvent 拦截了 contextmenu，需要在 NodeView 内部处理。
   * 通过自定义 DOM 事件将菜单数据传递给应用层。
   */
  private handleContextMenu(event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.lastContextMenuX = event.clientX;
    this.lastContextMenuY = event.clientY;
    const items = this.getContextMenuItems();
    this.dom.dispatchEvent(
      new CustomEvent('image-context-menu', {
        bubbles: true,
        detail: { x: event.clientX, y: event.clientY, items },
      }),
    );
  }

  /**
   * 构建图片右键菜单项。
   * 由 contextMenuPlugin 通过 mountContextMenuFactory 调用。
   */
  private getContextMenuItems(): ContextMenuItem[] {
    const align = this.node.attrs.align as string | null;
    const width = this.node.attrs.width as string | null;
    const src = String(this.node.attrs.src ?? '');
    // absolutePath 是 resolve 后的磁盘绝对路径，用于文件操作
    const absolutePath = this.resolved?.absolutePath ?? src;
    const displaySrc = this.resolved?.displaySrc ?? src;
    const disabled = !this.view.editable;

    const setNodeAttr = (name: string, value: unknown) => {
      const pos = this.findOwnPos();
      if (pos < 0) return;
      const attrs = { ...this.node.attrs, [name]: value };
      this.view.dispatch(this.view.state.tr.setNodeMarkup(pos, undefined, attrs));
    };

    const removeNode = () => {
      const pos = this.findOwnPos();
      if (pos < 0) return;
      this.view.dispatch(this.view.state.tr.delete(pos, pos + this.node.nodeSize));
    };

    return [
      // 对齐组
      {
        label: t.alignLeft(),
        icon: 'align-left',
        disabled,
        action: () => setNodeAttr('align', 'left'),
        active: align === 'left',
      },
      {
        label: t.alignCenter(),
        icon: 'align-center',
        disabled,
        action: () => setNodeAttr('align', 'center'),
        active: align === 'center',
      },
      {
        label: t.alignRight(),
        icon: 'align-right',
        disabled,
        action: () => setNodeAttr('align', 'right'),
        active: align === 'right',
      },
      { label: '', separator: true },
      {
        label: t.originalSize(),
        icon: 'image',
        disabled,
        action: () => setNodeAttr('width', null),
        active: !width,
      },
      {
        label: t.setSize(),
        icon: 'edit',
        disabled,
        action: () => this.openSizeEditor(),
      },
      {
        label: t.editImageAlt(),
        icon: 'edit',
        disabled,
        action: () => this.openAltEditor(),
      },
      // 文件操作组
      { label: '', separator: true },
      {
        label: t.openImageLocation(),
        icon: 'open',
        action: () => this.openImageLocation(absolutePath),
      },
      {
        label: t.copyImage(),
        icon: 'copy',
        action: () => this.copyImageToClipboard(displaySrc),
      },
      {
        label: t.copyImagePath(),
        icon: 'copy',
        action: () => this.copyImagePath(absolutePath),
      },
      // 危险操作
      { label: '', separator: true },
      {
        label: t.deleteAction(),
        icon: 'delete',
        disabled,
        action: removeNode,
        danger: true,
      },
    ];
  }

  /**
   * 查找当前图片节点在 ProseMirror 文档中的位置。
   * 通过遍历文档匹配节点引用来定位。
   */
  private findOwnPos(): number {
    let foundPos = -1;
    this.view.state.doc.descendants((node, pos) => {
      if (node === this.node) {
        foundPos = pos;
        return false;
      }
      return true;
    });
    // 回退：通过 DOM 定位
    if (foundPos < 0) {
      foundPos = this.view.posAtDOM(this.dom, 0);
    }
    return foundPos;
  }

  /** 打开尺寸编辑内联浮层 */
  private openSizeEditor(): void {
    // 关闭已有的浮层
    this.closeSizeEditor();

    const currentWidth = this.node.attrs.width as string | null;
    const img = this.dom.querySelector('img');
    const naturalWidth = img?.naturalWidth ?? 0;

    // 解析当前宽度值和单位
    let value = '';
    let unit: 'px' | '%' = 'px';
    if (currentWidth) {
      if (currentWidth.endsWith('%')) {
        value = currentWidth.slice(0, -1);
        unit = '%';
      } else {
        value = currentWidth.replace(/px$/, '');
        unit = 'px';
      }
    } else if (naturalWidth > 0) {
      value = String(naturalWidth);
    }

    // 构建浮层 DOM
    const overlay = document.createElement('div');
    overlay.className = 'image-size-editor-overlay';

    const popover = document.createElement('div');
    popover.className = 'image-size-editor';

    // 输入行
    const row = document.createElement('div');
    row.className = 'image-size-editor-row';

    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'image-size-editor-input';
    input.value = value;
    input.placeholder = t.width();
    input.setAttribute('aria-label', t.imageWidth());

    const unitBtn = document.createElement('button');
    unitBtn.type = 'button';
    unitBtn.className = 'image-size-editor-unit';
    unitBtn.textContent = unit;
    unitBtn.title = t.switchUnit();
    unitBtn.addEventListener('click', () => {
      unit = unit === 'px' ? '%' : 'px';
      unitBtn.textContent = unit;
      // 切换单位时转换数值
      const num = parseFloat(input.value);
      if (!isNaN(num) && naturalWidth > 0) {
        if (unit === '%' && input.value === String(naturalWidth)) {
          input.value = '100';
        } else if (unit === 'px' && input.value === '100') {
          input.value = String(naturalWidth);
        }
      }
      input.focus();
    });

    row.append(input, unitBtn);

    // 按钮行
    const actions = document.createElement('div');
    actions.className = 'image-size-editor-actions';

    const resetBtn = document.createElement('button');
    resetBtn.type = 'button';
    resetBtn.className = 'image-size-editor-reset';
    resetBtn.textContent = t.reset();
    resetBtn.addEventListener('click', () => {
      this.setImageWidth(null);
      this.closeSizeEditor();
    });

    const confirmBtn = document.createElement('button');
    confirmBtn.type = 'button';
    confirmBtn.className = 'image-size-editor-confirm';
    confirmBtn.textContent = t.ok();
    confirmBtn.addEventListener('click', () => {
      const num = parseFloat(input.value);
      if (isNaN(num) || num <= 0) {
        this.setImageWidth(null);
      } else {
        this.setImageWidth(`${num}${unit}`);
      }
      this.closeSizeEditor();
    });

    actions.append(resetBtn, confirmBtn);
    popover.append(row, actions);

    // 点击遮罩关闭
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) this.closeSizeEditor();
    });

    // 键盘事件
    const handleKeydown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        this.closeSizeEditor();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        confirmBtn.click();
      }
    };
    input.addEventListener('keydown', handleKeydown);

    overlay.appendChild(popover);
    document.body.appendChild(overlay);
    this.sizeEditorEl = overlay;

    // 定位浮层到鼠标位置
    this.sizeEditorCommitFn = () => confirmBtn.click();
    registerActiveEdit(this.view, this.sizeEditorCommitFn);
    popover.style.position = 'fixed';
    popover.style.left = `${this.lastContextMenuX}px`;
    popover.style.top = `${this.lastContextMenuY}px`;
    popover.style.transform = 'translateX(-50%)';

    // 聚焦输入框并选中文本
    requestAnimationFrame(() => {
      input.focus();
      input.select();
    });
  }

  private closeSizeEditor(): void {
    if (this.sizeEditorCommitFn) {
      unregisterActiveEdit(this.view, this.sizeEditorCommitFn);
      this.sizeEditorCommitFn = null;
    }
    this.sizeEditorEl?.remove();
    this.sizeEditorEl = null;
  }

  private openAltEditor(): void {
    this.closeSizeEditor();
    const overlay = document.createElement('div');
    overlay.className = 'image-size-editor-overlay';
    const popover = document.createElement('div');
    popover.className = 'image-size-editor';
    const row = document.createElement('div');
    row.className = 'image-size-editor-row';
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'image-size-editor-input';
    input.value = String(this.node.attrs.alt ?? '');
    input.placeholder = t.imageAlt();
    input.setAttribute('aria-label', t.imageAlt());
    row.appendChild(input);

    const actions = document.createElement('div');
    actions.className = 'image-size-editor-actions';
    const cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.className = 'image-size-editor-reset';
    cancelBtn.textContent = t.cancel();
    cancelBtn.addEventListener('click', () => this.closeSizeEditor());
    const confirmBtn = document.createElement('button');
    confirmBtn.type = 'button';
    confirmBtn.className = 'image-size-editor-confirm';
    confirmBtn.textContent = t.ok();
    confirmBtn.addEventListener('click', () => {
      const pos = this.findOwnPos();
      if (pos >= 0) {
        this.view.dispatch(
          this.view.state.tr.setNodeMarkup(pos, undefined, {
            ...this.node.attrs,
            alt: input.value.trim(),
          }),
        );
      }
      this.closeSizeEditor();
    });
    actions.append(cancelBtn, confirmBtn);
    popover.append(row, actions);
    overlay.appendChild(popover);
    overlay.addEventListener('click', (event) => {
      if (event.target === overlay) this.closeSizeEditor();
    });
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        this.closeSizeEditor();
      } else if (event.key === 'Enter') {
        event.preventDefault();
        confirmBtn.click();
      }
    });
    document.body.appendChild(overlay);
    this.sizeEditorEl = overlay;
    popover.style.position = 'fixed';
    this.sizeEditorCommitFn = () => confirmBtn.click();
    registerActiveEdit(this.view, this.sizeEditorCommitFn);
    popover.style.left = `${this.lastContextMenuX}px`;
    popover.style.top = `${this.lastContextMenuY}px`;
    popover.style.transform = 'translateX(-50%)';
    requestAnimationFrame(() => {
      input.focus();
      input.select();
    });
  }

  private setImageWidth(width: string | null): void {
    const pos = this.findOwnPos();
    if (pos < 0) return;
    const attrs = { ...this.node.attrs, width };
    this.view.dispatch(this.view.state.tr.setNodeMarkup(pos, undefined, attrs));
  }

  /** 打开图片所在文件夹并选中文件 */
  private async openImageLocation(resolvedSrc: string): Promise<void> {
    try {
      if (resolvedSrc.startsWith('http://') || resolvedSrc.startsWith('https://')) {
        return;
      }
      // 尝试使用 Tauri 命令在文件管理器中定位
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('reveal_in_explorer', { path: resolvedSrc });
    } catch {
      // 静默失败
    }
  }

  /** 复制图片二进制到系统剪贴板 */
  private async copyImageToClipboard(resolvedSrc: string): Promise<void> {
    try {
      const response = await fetch(resolvedSrc);
      const blob = await response.blob();
      try {
        const { writeImage } = await import('@tauri-apps/plugin-clipboard-manager');
        await writeImage(new Uint8Array(await blob.arrayBuffer()));
      } catch {
        await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
      }
    } catch {
      // 静默失败
    }
  }

  /** 复制图片 src 路径到剪贴板 */
  private async copyImagePath(src: string): Promise<void> {
    try {
      try {
        const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
        await writeText(src);
      } catch {
        await navigator.clipboard.writeText(src);
      }
    } catch {
      // 静默失败
    }
  }

  private renderError(message: string, alt: string): void {
    this.dom.classList.remove('is-loading');
    this.dom.replaceChildren();
    const placeholder = document.createElement('span');
    placeholder.className = 'image-node-placeholder';
    placeholder.setAttribute('role', 'img');
    placeholder.setAttribute(
      'aria-label',
      alt ? `${t.imageLoadFailed()}: ${alt}` : t.imageLoadFailed(),
    );

    const title = document.createElement('strong');
    title.textContent = alt || t.image();
    const detail = document.createElement('span');
    detail.textContent = message;
    placeholder.append(title, detail);
    this.dom.appendChild(placeholder);
  }

  private openFullscreen(): void {
    if (this.fullscreenOverlayEl || !this.resolved?.exists) {
      return;
    }

    const img = this.dom.querySelector<HTMLImageElement>('img');
    if (!img) {
      return;
    }

    const overlayEl = document.createElement('div');
    overlayEl.className = 'image-fullscreen-overlay';
    overlayEl.setAttribute('role', 'dialog');
    overlayEl.setAttribute('aria-modal', 'true');
    overlayEl.setAttribute('aria-label', t.fullscreenImagePreview());

    const panelEl = document.createElement('div');
    panelEl.className = 'image-fullscreen-panel';

    const closeButton = this.createIconButton(
      'image-fullscreen-close-button',
      t.closeFullscreenImage(),
      t.close(),
      'close',
    );
    closeButton.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      this.closeFullscreen();
    });

    const viewportEl = document.createElement('div');
    viewportEl.className = 'image-fullscreen-viewport';

    const zoomSurfaceEl = document.createElement('div');
    zoomSurfaceEl.className = 'image-fullscreen-zoom-surface';
    const fullscreenImg = img.cloneNode(false) as HTMLImageElement;
    fullscreenImg.removeAttribute('loading');
    zoomSurfaceEl.appendChild(fullscreenImg);

    const zoomBadgeEl = document.createElement('div');
    zoomBadgeEl.className = 'image-fullscreen-zoom-badge';

    viewportEl.appendChild(zoomSurfaceEl);
    viewportEl.addEventListener('wheel', (event) => this.handleFullscreenWheel(event), {
      passive: false,
    });
    viewportEl.addEventListener('pointerdown', (event) => this.handleFullscreenPointerDown(event));
    viewportEl.addEventListener('pointermove', (event) => this.handleFullscreenPointerMove(event));
    viewportEl.addEventListener('pointerup', (event) => this.finishFullscreenDrag(event));
    viewportEl.addEventListener('pointercancel', (event) => this.finishFullscreenDrag(event));

    panelEl.append(closeButton, viewportEl, zoomBadgeEl);
    overlayEl.appendChild(panelEl);
    overlayEl.addEventListener('click', (event) => {
      if (event.target === overlayEl) {
        this.closeFullscreen();
      }
    });

    this.fullscreenKeydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        this.closeFullscreen();
      }
    };
    document.addEventListener('keydown', this.fullscreenKeydown);
    document.body.appendChild(overlayEl);
    document.body.classList.add('has-image-fullscreen');
    this.fullscreenOverlayEl = overlayEl;
    this.fullscreenViewportEl = viewportEl;
    this.fullscreenZoomSurfaceEl = zoomSurfaceEl;
    this.fullscreenZoomBadgeEl = zoomBadgeEl;
    this.setFullscreenScale(ImageNodeView.FULLSCREEN_DEFAULT_SCALE);

    requestAnimationFrame(() => {
      this.centerFullscreenContent();
      closeButton.focus({ preventScroll: true });
    });
  }

  private closeFullscreen(): void {
    if (this.fullscreenKeydown) {
      document.removeEventListener('keydown', this.fullscreenKeydown);
      this.fullscreenKeydown = null;
    }
    this.fullscreenOverlayEl?.remove();
    this.fullscreenOverlayEl = null;
    this.fullscreenViewportEl = null;
    this.fullscreenZoomSurfaceEl = null;
    this.fullscreenZoomBadgeEl = null;
    this.fullscreenDrag = null;
    document.body.classList.remove('has-image-fullscreen');
  }

  private handleFullscreenWheel(event: WheelEvent): void {
    if (!event.ctrlKey) return;
    event.preventDefault();

    const viewportEl = this.fullscreenViewportEl;
    if (!viewportEl) return;

    const oldScale = this.fullscreenScale;
    const direction = event.deltaY < 0 ? 1 : -1;
    const nextScale = this.clampFullscreenScale(
      oldScale + direction * ImageNodeView.FULLSCREEN_SCALE_STEP,
    );
    if (nextScale === oldScale) return;

    const viewportRect = viewportEl.getBoundingClientRect();
    const pointerX = event.clientX - viewportRect.left;
    const pointerY = event.clientY - viewportRect.top;
    const contentX = viewportEl.scrollLeft + pointerX;
    const contentY = viewportEl.scrollTop + pointerY;
    const scaleRatio = nextScale / oldScale;

    this.setFullscreenScale(nextScale);
    viewportEl.scrollLeft = contentX * scaleRatio - pointerX;
    viewportEl.scrollTop = contentY * scaleRatio - pointerY;
  }

  private handleFullscreenPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || !this.fullscreenViewportEl) return;
    event.preventDefault();
    this.fullscreenDrag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      scrollLeft: this.fullscreenViewportEl.scrollLeft,
      scrollTop: this.fullscreenViewportEl.scrollTop,
    };
    this.fullscreenViewportEl.classList.add('is-dragging');
    this.fullscreenViewportEl.setPointerCapture?.(event.pointerId);
  }

  private handleFullscreenPointerMove(event: PointerEvent): void {
    if (!this.fullscreenViewportEl || !this.fullscreenDrag) return;
    if (event.pointerId !== this.fullscreenDrag.pointerId) return;
    event.preventDefault();
    this.fullscreenViewportEl.scrollLeft =
      this.fullscreenDrag.scrollLeft - (event.clientX - this.fullscreenDrag.startX);
    this.fullscreenViewportEl.scrollTop =
      this.fullscreenDrag.scrollTop - (event.clientY - this.fullscreenDrag.startY);
  }

  private finishFullscreenDrag(event: PointerEvent): void {
    if (!this.fullscreenViewportEl || !this.fullscreenDrag) return;
    if (event.pointerId !== this.fullscreenDrag.pointerId) return;
    if (this.fullscreenViewportEl.hasPointerCapture?.(event.pointerId)) {
      this.fullscreenViewportEl.releasePointerCapture(event.pointerId);
    }
    this.fullscreenViewportEl.classList.remove('is-dragging');
    this.fullscreenDrag = null;
  }

  private centerFullscreenContent(): void {
    const viewportEl = this.fullscreenViewportEl;
    if (!viewportEl) return;
    viewportEl.scrollLeft = Math.max(0, (viewportEl.scrollWidth - viewportEl.clientWidth) / 2);
    viewportEl.scrollTop = Math.max(0, (viewportEl.scrollHeight - viewportEl.clientHeight) / 2);
  }

  private clampFullscreenScale(scale: number): number {
    return Math.min(
      ImageNodeView.FULLSCREEN_MAX_SCALE,
      Math.max(ImageNodeView.FULLSCREEN_MIN_SCALE, scale),
    );
  }

  private setFullscreenScale(scale: number): void {
    this.fullscreenScale = this.clampFullscreenScale(scale);
    const roundedScale = Number(this.fullscreenScale.toFixed(2));
    this.fullscreenZoomSurfaceEl?.style.setProperty('--image-fullscreen-scale', `${roundedScale}`);
    if (this.fullscreenZoomBadgeEl) {
      this.fullscreenZoomBadgeEl.textContent = `${Math.round(roundedScale * 100)}%`;
    }
  }

  private createIconButton(
    className: string,
    ariaLabel: string,
    title: string,
    icon: 'maximize' | 'close',
  ): HTMLButtonElement {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = className;
    button.setAttribute('aria-label', ariaLabel);
    button.title = title;
    button.innerHTML =
      icon === 'maximize'
        ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 3h6v6"/><path d="M21 3l-7 7"/><path d="M9 21H3v-6"/><path d="M3 21l7-7"/></svg>'
        : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';
    return button;
  }
}

function isBadgeImageSrc(src: string): boolean {
  try {
    const url = new URL(src);
    const host = url.hostname.toLowerCase();
    return host === 'img.shields.io' || host === 'badgen.net';
  } catch {
    return false;
  }
}
