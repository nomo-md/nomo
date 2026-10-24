/** 显式验收构建的有界探针；普通构建常量折叠后不安装监听器、不持有缓存。 */
type AcceptanceEvent = { kind: string; at: number; [key: string]: unknown };
type AcceptanceBuffer = { origin: number; events: AcceptanceEvent[]; dropped: number };
declare global { interface Window { __nomoAcceptance?: AcceptanceBuffer } }
const enabled = import.meta.env.VITE_NOMO_ACCEPTANCE === '1';
const indexStates = new Map<string, string>();
export function recordAcceptance(kind: string, fields: Record<string, unknown>) {
  if (!enabled || typeof window === 'undefined') return;
  if (kind === 'index') {
    const key = String(fields.documentId);
    const signature = JSON.stringify([fields.revision, fields.state, fields.failure,
      (fields.timings as { indexed?: number } | undefined)?.indexed]);
    if (indexStates.get(key) === signature) return;
    indexStates.set(key, signature);
    if (indexStates.size > 16) indexStates.delete(indexStates.keys().next().value!);
  }
  const buffer = window.__nomoAcceptance ??= { origin: performance.timeOrigin, events: [], dropped: 0 };
  if (buffer.events.length >= 1024) { buffer.events.splice(0, 256); buffer.dropped += 256; }
  buffer.events.push({ kind, at: performance.now(), ...fields });
}
export function installAcceptanceProbe() {
  if (!enabled) return;
  recordAcceptance('entry', {});
  let serial = 0;
  let pending: { id: number; at: number; root: Element } | undefined;
  const input = (event: Event) => {
    const root = event.target instanceof Element ? event.target.closest('.ProseMirror, .cm-editor, textarea') : null;
    // 焦点转移/视图销毁不保证成对发出 compositionend，不能用全局布尔值永久屏蔽后续输入。
    if (!root || (event instanceof InputEvent && event.isComposing)) return;
    const id = ++serial;
    pending = { id, at: performance.now(), root };
    recordAcceptance('input', { id, event: event.type });
  };
  const compositionStart = () => { pending = undefined; };
  const compositionEnd = (event: Event) => { input(event); };
  const tabIntent = (event: Event) => {
    const tab = event.target instanceof Element ? event.target.closest('[data-workspace-item-id]') : null;
    if (tab) recordAcceptance('tab-intent', { item: tab.getAttribute('data-workspace-item-id') });
  };
  document.addEventListener('pointerdown', tabIntent, true);
  document.addEventListener('beforeinput', input, true);
  document.addEventListener('compositionstart', compositionStart, true);
  document.addEventListener('compositionend', compositionEnd, true);
  const mutation = new MutationObserver(records => {
    const current = pending;
    if (!current || !records.some(record => current.root.contains(record.target))) return;
    pending = undefined;
    recordAcceptance('input-dom', { id: current.id, duration: performance.now() - current.at });
    requestAnimationFrame(() => setTimeout(() => recordAcceptance('input-frame', {
      id: current.id, duration: performance.now() - current.at,
    }), 0));
  });
  mutation.observe(document, { subtree: true, childList: true, characterData: true });
  let timing: PerformanceObserver | undefined;
  if (PerformanceObserver.supportedEntryTypes.includes('event')) {
    timing = new PerformanceObserver(list => {
      for (const entry of list.getEntries()) recordAcceptance('event-timing', {
        event: entry.name, start: entry.startTime, duration: entry.duration,
      });
    });
    timing.observe({ type: 'event', buffered: true, durationThreshold: 16 } as PerformanceObserverInit);
  }
  window.addEventListener('beforeunload', () => {
    mutation.disconnect(); timing?.disconnect(); pending = undefined;
    document.removeEventListener('beforeinput', input, true);
    document.removeEventListener('compositionstart', compositionStart, true);
    document.removeEventListener('compositionend', compositionEnd, true);
    document.removeEventListener('pointerdown', tabIntent, true);
  }, { once: true });
}
