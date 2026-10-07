/** Mermaid 图表放大查看器所需的界面文案。 */
export interface MermaidViewLabels {
  mode: string;
  readable: string;
  fit: string;
  original: string;
  custom: string;
}

export interface MermaidFullscreenLabels extends MermaidViewLabels {
  /** 图表卡片上“放大查看”按钮的无障碍名称。 */
  open: string;
  /** 全屏对话框的无障碍名称。 */
  dialog: string;
  /** 关闭全屏对话框按钮的无障碍名称。 */
  close: string;
  /** 放大一级按钮的无障碍名称。 */
  zoomIn: string;
  /** 缩小一级按钮的无障碍名称。 */
  zoomOut: string;
  /** 恢复默认比例按钮的无障碍名称。 */
  reset: string;
}

/** Mermaid 图表卡片与共享放大查看器之间的绑定。 */
export interface MermaidFullscreenBinding {
  /** 已插入图表卡片的放大查看按钮。 */
  button: HTMLButtonElement;
  /** 关闭当前绑定打开的全屏查看器；按钮和监听仍可继续使用。 */
  close(): void;
  /** 关闭查看器并移除按钮及监听；调用后该绑定不可复用。 */
  dispose(): void;
}

const FULLSCREEN_MIN_SCALE = 0.1;
const FULLSCREEN_MAX_SCALE = 3;
const FULLSCREEN_SCALE_STEP = 0.1;

export type MermaidViewMode = 'readable' | 'fit' | 'original' | 'custom';
type SvgSize = { width: number; height: number };
export interface MermaidViewPosition {
  x: number;
  y: number;
  startX: boolean;
  startY: boolean;
}

/** 只属于视图的缩放状态，不写入 Markdown 或编辑器事务。 */
export interface MermaidViewportBinding {
  update(): void;
  position(): MermaidViewPosition | undefined;
  zoomBy(delta: number, point?: { x: number; y: number }): void;
  reset(): void;
  dispose(): void;
}

/** 高度只影响全图概览；日常阅读最多缩小到 85%。 */
export function calculateMermaidScale(
  mode: Exclude<MermaidViewMode, 'custom'>,
  size: SvgSize,
  available: SvgSize,
): number {
  if (mode === 'original') return 1;
  if (mode === 'fit')
    return Math.min(1, available.width / size.width, available.height / size.height);
  return Math.max(0.85, Math.min(1, available.width / size.width));
}

/** 将视图控件和尺寸管理绑定到已有 SVG；调用方在替换 DOM 前保存 position。 */
export function bindMermaidViewport(
  viewport: HTMLElement,
  toolbar: HTMLElement,
  labels: MermaidViewLabels,
  options: {
    kind: 'card' | 'preview' | 'fullscreen';
    mode?: Exclude<MermaidViewMode, 'custom'>;
    position?: MermaidViewPosition;
    onModeChange?: (mode: Exclude<MermaidViewMode, 'custom'>) => void;
    badge?: HTMLElement;
  },
): MermaidViewportBinding {
  const select = document.createElement('select');
  select.className = 'mermaid-view-mode';
  select.setAttribute('aria-label', labels.mode);
  for (const mode of [
    'readable',
    'fit',
    'original',
    ...(options.kind === 'fullscreen' ? ['custom'] : []),
  ] as MermaidViewMode[]) {
    const option = document.createElement('option');
    option.value = mode;
    option.textContent = labels[mode];
    option.disabled = mode === 'custom';
    select.appendChild(option);
  }
  const badge = options.badge ?? document.createElement('span');
  if (!options.badge) badge.className = 'mermaid-view-scale';
  toolbar.prepend(select);
  if (!options.badge) toolbar.appendChild(badge);
  viewport.classList.add('mermaid-managed-viewport');
  viewport.dataset.mermaidView = options.kind;
  if (options.kind !== 'fullscreen') viewport.tabIndex = 0;
  let mode: MermaidViewMode = options.mode ?? 'readable';
  let scale = 1;
  let svg: SVGSVGElement | null = null;
  let size: SvgSize | null = null;
  let pendingPosition = options.position;
  let frame = 0;
  let disposed = false;
  let lastWidth = 0;
  let lastHeight = 0;
  let lastClientHeight = 0;
  const heightBudget = () =>
    options.kind === 'fullscreen'
      ? viewport.clientHeight
      : Math.max(160, Math.min(420, window.innerHeight * 0.52));

  const position = (point?: { x: number; y: number }): MermaidViewPosition | undefined => {
    if (!svg || !size || !viewport.clientWidth) return pendingPosition;
    const bounds = svg.getBoundingClientRect();
    const parent = viewport.getBoundingClientRect();
    return {
      x:
        (parent.left + viewport.clientLeft + (point?.x ?? viewport.clientWidth / 2) - bounds.left) /
        scale,
      y:
        (parent.top + viewport.clientTop + (point?.y ?? viewport.clientHeight / 2) - bounds.top) /
        scale,
      startX: !point && viewport.scrollLeft === 0,
      startY: !point && viewport.scrollTop === 0,
    };
  };
  const syncControls = () => {
    select.value = mode;
    select.disabled = !size;
    badge.textContent = size ? `${Number((scale * 100).toFixed(1))}%` : '—';
  };
  const apply = (reset = false, anchor = position(), point?: { x: number; y: number }) => {
    if (!svg || !size || !viewport.clientWidth) {
      syncControls();
      return;
    }
    const style = getComputedStyle(viewport);
    const horizontal = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
    const vertical = (parseFloat(style.paddingTop) || 0) + (parseFloat(style.paddingBottom) || 0);
    const budget = heightBudget();
    const available = {
      width: Math.max(1, viewport.clientWidth - horizontal),
      height: Math.max(1, budget - vertical),
    };
    if (mode !== 'custom') scale = calculateMermaidScale(mode, size, available);
    const width = Math.ceil(size.width * scale - 1e-8);
    const height = Math.ceil(size.height * scale - 1e-8);
    for (const [key, value] of [
      ['width', width],
      ['height', height],
    ] as const) {
      if (svg.getAttribute(key) !== String(value)) svg.setAttribute(key, String(value));
      if (svg.style[key] !== `${value}px`) svg.style[key] = `${value}px`;
    }
    svg.style.marginLeft = `${Math.max(0, (available.width - width) / 2)}px`;
    svg.style.marginTop = `${options.kind === 'fullscreen' ? Math.max(0, (available.height - height) / 2) : 0}px`;
    if (reset) {
      viewport.scrollLeft = 0;
      viewport.scrollTop = 0;
    } else if (anchor) {
      const bounds = svg.getBoundingClientRect();
      const parent = viewport.getBoundingClientRect();
      viewport.scrollLeft = anchor.startX
        ? 0
        : viewport.scrollLeft +
          bounds.left +
          anchor.x * scale -
          parent.left -
          viewport.clientLeft -
          (point?.x ?? viewport.clientWidth / 2);
      viewport.scrollTop = anchor.startY
        ? 0
        : viewport.scrollTop +
          bounds.top +
          anchor.y * scale -
          parent.top -
          viewport.clientTop -
          (point?.y ?? viewport.clientHeight / 2);
    }
    pendingPosition = undefined;
    lastWidth = viewport.clientWidth;
    lastHeight = budget;
    lastClientHeight = viewport.clientHeight;
    syncControls();
  };
  const update = () => {
    pendingPosition ??= position();
    const next = viewport.querySelector<SVGSVGElement>('svg');
    if (next !== svg) {
      svg = next;
      size = svg ? readSvgIntrinsicSize(svg) : null;
      if (svg && size) {
        svg.dataset.mermaidNaturalWidth = String(size.width);
        svg.dataset.mermaidNaturalHeight = String(size.height);
      }
    }
    apply(!pendingPosition, pendingPosition);
  };
  const resize = () => {
    if (disposed || frame) return;
    // ResizeObserver 通知时容器已变宽，用上次视口中心恢复原有阅读位置。
    const anchor = lastWidth ? position({ x: lastWidth / 2, y: lastClientHeight / 2 }) : position();
    if (anchor) {
      anchor.startX = viewport.scrollLeft === 0;
      anchor.startY = viewport.scrollTop === 0;
    }
    frame = requestAnimationFrame(() => {
      frame = 0;
      const height = heightBudget();
      if (viewport.clientWidth !== lastWidth || height !== lastHeight)
        apply(false, pendingPosition ?? anchor);
    });
  };
  const change = () => {
    mode = select.value as Exclude<MermaidViewMode, 'custom'>;
    options.onModeChange?.(mode);
    apply(true);
  };
  select.addEventListener('change', change);
  const observer = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(resize) : null;
  observer?.observe(viewport);
  window.addEventListener('resize', resize);
  update();
  return {
    update,
    position,
    zoomBy(delta, point) {
      if (!size) return;
      const pivot = point ?? { x: viewport.clientWidth / 2, y: viewport.clientHeight / 2 };
      const anchor = position(pivot);
      mode = 'custom';
      scale = Number(clampScale(scale + delta).toFixed(4));
      apply(false, anchor, pivot);
    },
    reset() {
      mode = 'readable';
      apply(true);
    },
    dispose() {
      disposed = true;
      observer?.disconnect();
      window.removeEventListener('resize', resize);
      if (frame) cancelAnimationFrame(frame);
      select.removeEventListener('change', change);
      select.remove();
      if (!options.badge) badge.remove();
    },
  };
}

/** 导出恢复完整 SVG，避免把当前屏幕缩放带到打印页面。 */
export function restoreMermaidExportSize(root: HTMLElement): void {
  root
    .querySelectorAll('.mermaid-view-toolbar, .mermaid-fullscreen-overlay')
    .forEach((el) => el.remove());
  root.querySelectorAll<SVGSVGElement>('.mermaid-block svg').forEach((svg) => {
    const size = readSvgIntrinsicSize(svg);
    if (size) {
      svg.setAttribute('width', String(size.width));
      svg.setAttribute('height', String(size.height));
    }
    for (const property of [
      'width',
      'height',
      'margin-left',
      'margin-top',
      'max-width',
      'max-height',
    ])
      svg.style.removeProperty(property);
    delete svg.dataset.mermaidNaturalWidth;
    delete svg.dataset.mermaidNaturalHeight;
  });
  root.querySelectorAll<HTMLElement>('.mermaid-managed-viewport').forEach((el) => {
    el.classList.remove('mermaid-managed-viewport');
    delete el.dataset.mermaidView;
    el.removeAttribute('tabindex');
  });
}

/**
 * 规范化 Mermaid SVG 的内在尺寸，使卡片和全屏查看器使用同一个几何基准。
 *
 * Mermaid 默认可能输出 `width="100%"`，导致图表继承整行宽度。本函数优先使用
 * `viewBox` 的宽高写回数值尺寸，并去掉内联 `max-width`，再交给各视图的 CSS 限制显示范围。
 *
 * @param svg Mermaid renderer 返回的完整 SVG 字符串；不要求一定包含有效 `viewBox`。
 * @returns 规范化后的 SVG 字符串；无法取得有效尺寸时返回结构不变的安全解析结果。
 */
export function normalizeMermaidSvgSize(svg: string): string {
  const template = document.createElement('template');
  template.innerHTML = svg.trim();

  const svgEl = template.content.querySelector('svg');
  const size = svgEl ? readSvgViewBoxSize(svgEl) : null;
  if (!svgEl || !size) return template.innerHTML;

  svgEl.setAttribute('width', String(Math.ceil(size.width)));
  svgEl.setAttribute('height', String(Math.ceil(size.height)));

  const inlineStyle = svgEl.getAttribute('style');
  if (inlineStyle) {
    const nextStyle = inlineStyle
      .split(';')
      .map((part) => part.trim())
      .filter((part) => part && !part.toLowerCase().startsWith('max-width'))
      .join('; ');
    if (nextStyle) {
      svgEl.setAttribute('style', nextStyle);
    } else {
      svgEl.removeAttribute('style');
    }
  }

  return template.innerHTML;
}

/**
 * 按 Mermaid 已布局根图形的真实边界修正 SVG 视口。
 *
 * Mermaid 11 在部分 WebView 中可能把 `foreignObject` 文本计入错误边界，使简单图表出现
 * 数千像素空白。此函数只在 SVG、`g.root` 与布局尺寸都有效时修正，否则保持原图不变。
 *
 * @param container 包含已挂载 Mermaid SVG 的元素；必须已进入文档并完成一次布局。
 * @returns 无返回值；成功时会原地更新 SVG 的 `viewBox`、`width` 和 `height`。
 */
export function normalizeRenderedMermaidViewport(container: HTMLElement): void {
  const svgEl = container.querySelector<SVGSVGElement>('svg');
  const rootGroupEl = svgEl?.querySelector<SVGGElement>('g.root');
  const viewBox = svgEl?.getAttribute('viewBox');
  if (!svgEl || !rootGroupEl || !viewBox) return;

  const [viewBoxX, viewBoxY, viewBoxWidth, viewBoxHeight] = viewBox
    .trim()
    .split(/\s+/)
    .map((value) => Number.parseFloat(value));
  const svgBounds = svgEl.getBoundingClientRect();
  const rootBounds = rootGroupEl.getBoundingClientRect();
  if (
    ![viewBoxX, viewBoxY, viewBoxWidth, viewBoxHeight].every(Number.isFinite) ||
    viewBoxWidth <= 0 ||
    viewBoxHeight <= 0 ||
    svgBounds.width <= 0 ||
    svgBounds.height <= 0 ||
    rootBounds.width <= 0 ||
    rootBounds.height <= 0
  ) {
    return;
  }

  const scaleX = svgBounds.width / viewBoxWidth;
  const scaleY = svgBounds.height / viewBoxHeight;
  const contentX = viewBoxX + (rootBounds.left - svgBounds.left) / scaleX;
  const contentY = viewBoxY + (rootBounds.top - svgBounds.top) / scaleY;
  const contentWidth = rootBounds.width / scaleX;
  const contentHeight = rootBounds.height / scaleY;
  const padding = 8;
  const normalizedWidth = Math.ceil(contentWidth + padding * 2);
  const normalizedHeight = Math.ceil(contentHeight + padding * 2);

  svgEl.setAttribute(
    'viewBox',
    [
      Math.floor(contentX - padding),
      Math.floor(contentY - padding),
      normalizedWidth,
      normalizedHeight,
    ].join(' '),
  );
  svgEl.setAttribute('width', String(normalizedWidth));
  svgEl.setAttribute('height', String(normalizedHeight));
}

/**
 * 给一块已渲染的 Mermaid 图表绑定与主编辑器一致的放大查看器。
 *
 * 查看器支持按钮、`+`/`-`/`0`、Ctrl/Command + 滚轮缩放，鼠标或触控笔拖动画布，
 * 以及 Escape、关闭按钮和点击遮罩关闭。所有状态均由返回的绑定拥有，销毁时不会残留全局监听。
 *
 * @param host 接收右上角放大按钮的图表卡片；调用方负责保证其可定位按钮。
 * @param renderedContent 包含可克隆 SVG 的已渲染区域；源码和错误状态不应传入。
 * @param labels 主应用或 Quick Look 提供的界面文案，不能包含 HTML。
 * @returns 可关闭和销毁的绑定；创建时已把按钮插入 `host`。
 */
export function bindMermaidFullscreen(
  host: HTMLElement,
  renderedContent: HTMLElement,
  labels: MermaidFullscreenLabels,
): MermaidFullscreenBinding {
  const button = createIconButton(
    'mermaid-block-fullscreen-button',
    labels.open,
    labels.open,
    'maximize',
  );
  let viewer: { close(): void } | null = null;

  const open = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    viewer?.close();
    viewer = openMermaidFullscreen(renderedContent, button, labels, () => {
      viewer = null;
    });
  };
  button.addEventListener('click', open);
  host.appendChild(button);

  return {
    button,
    close() {
      viewer?.close();
      viewer = null;
    },
    dispose() {
      viewer?.close();
      viewer = null;
      button.removeEventListener('click', open);
      button.remove();
    },
  };
}

/**
 * 创建并挂载一份全屏 Mermaid 查看器。
 *
 * @param renderedContent 原图的已渲染容器；函数只克隆节点，不移动或修改原图。
 * @param triggerButton 打开查看器的按钮；关闭后焦点会尽量恢复到该按钮。
 * @param labels 查看器按钮及对话框的界面文案。
 * @param onClosed 查看器完成清理后的通知；每个实例至多调用一次。
 * @returns 只暴露幂等 `close()` 的查看器句柄。
 */
function openMermaidFullscreen(
  renderedContent: HTMLElement,
  triggerButton: HTMLButtonElement,
  labels: MermaidFullscreenLabels,
  onClosed: () => void,
): { close(): void } {
  const overlayEl = document.createElement('div');
  overlayEl.className = 'mermaid-fullscreen-overlay';
  overlayEl.setAttribute('role', 'dialog');
  overlayEl.setAttribute('aria-modal', 'true');
  overlayEl.setAttribute('aria-label', labels.dialog);

  const panelEl = document.createElement('div');
  panelEl.className = 'mermaid-fullscreen-panel';

  const closeButton = createIconButton(
    'mermaid-fullscreen-close-button',
    labels.close,
    labels.close,
    'close',
  );
  const viewportEl = document.createElement('div');
  viewportEl.className = 'mermaid-fullscreen-viewport';
  const zoomSurfaceEl = document.createElement('div');
  zoomSurfaceEl.className = 'mermaid-fullscreen-zoom-surface';
  const fullContent = document.createElement('div');
  fullContent.className = 'mermaid-block-rendered';
  const sourceSvg = renderedContent.querySelector('svg');
  if (sourceSvg) fullContent.appendChild(sourceSvg.cloneNode(true));
  zoomSurfaceEl.appendChild(fullContent);
  viewportEl.appendChild(zoomSurfaceEl);

  const controlsEl = document.createElement('div');
  controlsEl.className = 'mermaid-fullscreen-controls';
  const zoomOutButton = createIconButton(
    'mermaid-fullscreen-control-button',
    labels.zoomOut,
    labels.zoomOut,
    'minus',
  );
  const resetButton = createIconButton(
    'mermaid-fullscreen-control-button',
    labels.reset,
    labels.reset,
    'reset',
  );
  const zoomInButton = createIconButton(
    'mermaid-fullscreen-control-button',
    labels.zoomIn,
    labels.zoomIn,
    'plus',
  );
  controlsEl.append(zoomOutButton, resetButton, zoomInButton);

  const zoomBadgeEl = document.createElement('div');
  zoomBadgeEl.className = 'mermaid-fullscreen-zoom-badge';
  panelEl.append(closeButton, viewportEl, controlsEl, zoomBadgeEl);
  overlayEl.appendChild(panelEl);

  let viewportBinding: MermaidViewportBinding;
  let focusFrame = 0;
  let closed = false;
  let drag: {
    pointerId: number;
    startX: number;
    startY: number;
    scrollLeft: number;
    scrollTop: number;
  } | null = null;

  const reset = () => viewportBinding.reset();
  const onWheel = (event: WheelEvent) => {
    if (!event.ctrlKey && !event.metaKey) return;
    event.preventDefault();
    const viewportRect = viewportEl.getBoundingClientRect();
    viewportBinding.zoomBy(event.deltaY < 0 ? FULLSCREEN_SCALE_STEP : -FULLSCREEN_SCALE_STEP, {
      x: event.clientX - viewportRect.left - viewportEl.clientLeft,
      y: event.clientY - viewportRect.top - viewportEl.clientTop,
    });
  };
  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    drag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      scrollLeft: viewportEl.scrollLeft,
      scrollTop: viewportEl.scrollTop,
    };
    viewportEl.classList.add('is-dragging');
    viewportEl.setPointerCapture?.(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    event.preventDefault();
    viewportEl.scrollLeft = drag.scrollLeft - (event.clientX - drag.startX);
    viewportEl.scrollTop = drag.scrollTop - (event.clientY - drag.startY);
  };
  const finishDrag = (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    if (viewportEl.hasPointerCapture?.(event.pointerId)) {
      viewportEl.releasePointerCapture(event.pointerId);
    }
    viewportEl.classList.remove('is-dragging');
    drag = null;
  };

  const viewer = {
    close() {
      if (closed) return;
      closed = true;
      document.removeEventListener('keydown', onKeydown);
      viewportBinding.dispose();
      cancelAnimationFrame(focusFrame);
      overlayEl.remove();
      document.body.classList.remove('has-mermaid-fullscreen');
      if (triggerButton.isConnected) triggerButton.focus({ preventScroll: true });
      onClosed();
    },
  };
  const onKeydown = (event: KeyboardEvent) => {
    if (event.key === 'Tab') {
      const controls = Array.from(
        panelEl.querySelectorAll<HTMLElement>('button:not(:disabled), select:not(:disabled)'),
      );
      const index = controls.indexOf(document.activeElement as HTMLElement);
      if (
        index < 0 ||
        (event.shiftKey && index === 0) ||
        (!event.shiftKey && index === controls.length - 1)
      ) {
        event.preventDefault();
        controls[event.shiftKey ? controls.length - 1 : 0]?.focus();
      }
    } else if (event.key === 'Escape') {
      event.preventDefault();
      viewer.close();
    } else if (event.key === '+' || event.key === '=') {
      event.preventDefault();
      viewportBinding.zoomBy(FULLSCREEN_SCALE_STEP);
    } else if (event.key === '-') {
      event.preventDefault();
      viewportBinding.zoomBy(-FULLSCREEN_SCALE_STEP);
    } else if (event.key === '0') {
      event.preventDefault();
      reset();
    }
  };

  closeButton.addEventListener('click', viewer.close);
  zoomOutButton.addEventListener('click', () => viewportBinding.zoomBy(-FULLSCREEN_SCALE_STEP));
  resetButton.addEventListener('click', reset);
  zoomInButton.addEventListener('click', () => viewportBinding.zoomBy(FULLSCREEN_SCALE_STEP));
  overlayEl.addEventListener('click', (event) => {
    if (event.target === overlayEl) viewer.close();
  });
  viewportEl.addEventListener('wheel', onWheel, { passive: false });
  viewportEl.addEventListener('pointerdown', onPointerDown);
  viewportEl.addEventListener('pointermove', onPointerMove);
  viewportEl.addEventListener('pointerup', finishDrag);
  viewportEl.addEventListener('pointercancel', finishDrag);
  document.addEventListener('keydown', onKeydown);
  document.body.appendChild(overlayEl);
  document.body.classList.add('has-mermaid-fullscreen');
  viewportBinding = bindMermaidViewport(viewportEl, controlsEl, labels, {
    kind: 'fullscreen',
    badge: zoomBadgeEl,
  });

  focusFrame = requestAnimationFrame(() => {
    closeButton.focus({ preventScroll: true });
  });
  return viewer;
}

/**
 * 创建查看器使用的纯图标按钮。
 *
 * @param className 按钮样式类名。
 * @param ariaLabel 面向辅助技术的完整操作名称。
 * @param title 指针悬停提示；允许与 ariaLabel 相同。
 * @param icon 内置线性图标名称。
 * @returns 尚未挂载、未绑定业务事件的 `button` 元素。
 */
function createIconButton(
  className: string,
  ariaLabel: string,
  title: string,
  icon: 'maximize' | 'close' | 'plus' | 'minus' | 'reset',
): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = className;
  button.setAttribute('aria-label', ariaLabel);
  button.title = title;
  const paths: Record<typeof icon, string> = {
    maximize:
      '<path d="M15 3h6v6"/><path d="M21 3l-7 7"/><path d="M9 21H3v-6"/><path d="M3 21l7-7"/>',
    close: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    plus: '<path d="M12 5v14"/><path d="M5 12h14"/>',
    minus: '<path d="M5 12h14"/>',
    reset: '<path d="M4 12a8 8 0 1 0 2.34-5.66"/><path d="M4 4v6h6"/>',
  };
  button.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[icon]}</svg>`;
  return button;
}

/** 将任意缩放值限制到查看器支持的安全范围。 */
function clampScale(scale: number): number {
  return Math.min(FULLSCREEN_MAX_SCALE, Math.max(FULLSCREEN_MIN_SCALE, scale));
}

/** 从 SVG 的 `viewBox` 或数值宽高读取稳定的缩放基准。 */
function readSvgIntrinsicSize(svgEl: SVGElement): { width: number; height: number } | null {
  const cached = {
    width: Number(svgEl.dataset.mermaidNaturalWidth),
    height: Number(svgEl.dataset.mermaidNaturalHeight),
  };
  return (
    readSvgViewBoxSize(svgEl) ??
    (Number.isFinite(cached.width) &&
    Number.isFinite(cached.height) &&
    cached.width > 0 &&
    cached.height > 0
      ? cached
      : readSvgAttributeSize(svgEl))
  );
}

/** 从 SVG `viewBox` 读取正数宽高；格式无效时返回 `null`。 */
function readSvgViewBoxSize(svgEl: Element): { width: number; height: number } | null {
  const viewBox = svgEl.getAttribute('viewBox');
  if (!viewBox) return null;
  const values = viewBox.trim().split(/\s+/).map(Number);
  if (values.length !== 4 || !values.every(Number.isFinite)) return null;
  const [, , width, height] = values;
  return Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0
    ? { width, height }
    : null;
}

/** 从 SVG 数值 `width`/`height` 属性读取正数宽高；格式无效时返回 `null`。 */
function readSvgAttributeSize(svgEl: Element): { width: number; height: number } | null {
  const width = Number(svgEl.getAttribute('width'));
  const height = Number(svgEl.getAttribute('height'));
  return Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0
    ? { width, height }
    : null;
}
