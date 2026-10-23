export type NavigationEnhancement = 'code' | 'typography' | 'diagram';
export interface NavigationRenderContext {
  revision: number;
  generation: number;
  deadline?: number;
  preparationOnly?: boolean;
  signal: AbortSignal;
  ready: boolean;
  targets: ReadonlySet<HTMLElement>;
  viewport(): { top: number; bottom: number };
  deferred(kind: NavigationEnhancement, element: HTMLElement): void;
  started(kind: NavigationEnhancement): () => void;
}
const navigationContexts = new WeakMap<HTMLElement, NavigationRenderContext>();
const navigationAnchors = new WeakSet<HTMLElement>();
const suspendedHosts = new WeakSet<HTMLElement>();
const NAVIGATION_RENDER_EVENT = 'nomo:navigation-render';
export function setRenderSuspended(host: HTMLElement, suspended: boolean) {
  if (suspendedHosts.has(host) === suspended) return;
  if (suspended) suspendedHosts.add(host); else suspendedHosts.delete(host);
  host.dispatchEvent(new Event(NAVIGATION_RENDER_EVENT, { bubbles: true }));
}

/** 仅宿主的临时视图调度；不进入文档状态、序列化或用户偏好。 */
export function setNavigationRenderContext(host: HTMLElement, context: NavigationRenderContext | undefined, keepAnchor = false) {
  if (context) { navigationContexts.set(host, context); navigationAnchors.add(host); }
  else navigationContexts.delete(host);
  if (!context && !keepAnchor) navigationAnchors.delete(host);
  host.dispatchEvent(new Event(NAVIGATION_RENDER_EVENT, { bubbles: true }));
}
function navigationContext(element: HTMLElement) {
  const host = element.closest<HTMLElement>('.markdown-session-host');
  const context = host && navigationContexts.get(host);
  return context && !context.signal.aborted ? context : undefined;
}
export function navigationRenderViewport(element: HTMLElement) {
  const context = navigationContext(element);
  return context && [...context.targets].some(target => target.contains(element)) ? context.viewport() : undefined;
}
export function navigationRenderSignal(element: HTMLElement) { return navigationContext(element)?.signal; }
export function navigationRenderAllowed(element: HTMLElement, kind: NavigationEnhancement): boolean | undefined {
  const context = navigationContext(element);
  if (!context) return undefined;
  if (context.preparationOnly) return false;
  const target = [...context.targets].some(target => target.contains(element));
  // 不启用即将到期的新增强任务；正文和用户编辑不走此门禁。
  const reserve = kind === 'diagram' ? 48 : kind === 'typography' ? 32 : 16;
  const remaining = context.deadline === undefined ? Infinity : context.deadline - performance.now();
  let allowed = context.ready && target && remaining > reserve;
  if (allowed && kind !== 'typography') {
    const rect = element.getBoundingClientRect(), viewport = context.viewport();
    allowed = rect.bottom >= viewport.top && rect.top <= viewport.bottom;
  }
  if (kind === 'typography' && context.ready && target && remaining <= reserve)
    context.deferred(kind, element);
  return allowed;
}
export function trackNavigationEnhancement(element: HTMLElement, kind: NavigationEnhancement, controller?: AbortController) {
  const context = navigationContext(element);
  if (!context) return () => {};
  const abort = () => controller?.abort();
  context.signal.addEventListener('abort', abort, { once: true });
  const finish = context.started(kind);
  return () => { context.signal.removeEventListener('abort', abort); finish(); };
}
export function observeNavigationRender(element: HTMLElement, notify: () => void) {
  const listener = (event: Event) => {
    if (event.target instanceof Element && event.target.contains(element)) notify();
  };
  element.ownerDocument.addEventListener(NAVIGATION_RENDER_EVENT, listener);
  return () => element.ownerDocument.removeEventListener(NAVIGATION_RENDER_EVENT, listener);
}

/** 不可见文档暂停增强；无导航上下文时保留导出／测试宿主的原行为。 */
export function observeRenderVisibility(element: HTMLElement, onChange: (visible: boolean) => void,
  kind: NavigationEnhancement = 'code', needsPreparation: () => boolean = () => true): () => void {
  let nearViewport = typeof IntersectionObserver !== 'function';
  const notify = () => {
    const allowed = navigationRenderAllowed(element, kind);
    const context = navigationContext(element);
    if (allowed === false && context?.ready && context.deadline !== undefined && needsPreparation() &&
      context.deadline - performance.now() <= (kind === 'diagram' ? 48 : 16) &&
      [...context.targets].some(target => target.contains(element))) context.deferred(kind, element);
    const host = element.closest<HTMLElement>('.markdown-session-host');
    onChange((allowed ?? nearViewport) && !element.ownerDocument.hidden && !(host && suspendedHosts.has(host)));
  };
  const stopNavigation = observeNavigationRender(element, notify);
  const observer = typeof IntersectionObserver === 'function'
    ? new IntersectionObserver(([entry]) => {
      if (!entry) return;
      nearViewport = entry.isIntersecting;
      notify();
    }, { rootMargin: '400px' }) : null;
  observer?.observe(element);
  element.ownerDocument.addEventListener('visibilitychange', notify);
  return () => {
    stopNavigation();
    observer?.disconnect();
    element.ownerDocument.removeEventListener('visibilitychange', notify);
  };
}

/** 屏幕上方的占位换成图表时保持阅读位置；仅在增强结果提交时测量一次。 */
export function preserveRenderScrollAnchor(element: HTMLElement, update: () => void): void {
  // 会话宿主统一在布局批次中保持标题锚点，避免图表另写 scrollTop 被误判为用户接管。
  const host = element.closest<HTMLElement>('.markdown-session-host');
  if (host && navigationAnchors.has(host)) { update(); return; }
  const pane = element.closest<HTMLElement>('.semantic-pane');
  const before = pane && element.getBoundingClientRect();
  const above = pane && before && before.bottom <= pane.getBoundingClientRect().top;
  const scrollTop = pane?.scrollTop ?? 0;
  update();
  if (above && pane && before) pane.scrollTop = scrollTop + element.getBoundingClientRect().height - before.height;
}
