import type { MarkdownSyncAnchor } from '../../lib/editor-core/scrollSyncMapping';

export const SYNC_REFERENCE_RATIO = 0.3;
export type SyncPane = 'source' | 'semantic';
export interface SyncPoint {
  source: number;
  semantic: number;
  anchor: MarkdownSyncAnchor;
}
export interface SyncPaneAdapter {
  element: HTMLElement;
  position(anchor: MarkdownSyncAnchor): number | null;
  caret(anchors: readonly MarkdownSyncAnchor[]): number | null;
}
export interface ScrollSyncOptions {
  snapshot(): { revision: string; ready: boolean; anchors: readonly MarkdownSyncAnchor[] };
  pane(pane: SyncPane): SyncPaneAdapter | null;
  status?(status: string, leader: SyncPane, revision: string): void;
  requestFrame?(callback: FrameRequestCallback): number;
  cancelFrame?(frame: number): void;
}

/** 两个有效内容边界之间的连续换算，不比较两栏的绝对高度。 */
export function interpolateSyncPosition(
  before: SyncPoint,
  after: SyncPoint,
  pane: SyncPane,
  position: number,
) {
  const target = pane === 'source' ? 'semantic' : 'source';
  const distance = after[pane] - before[pane];
  if (distance <= 0) return before[target];
  const progress = Math.min(1, Math.max(0, (position - before[pane]) / distance));
  return before[target] + progress * Math.max(0, after[target] - before[target]);
}

/**
 * 唯一的跨栏滚动写入入口。布局只使测量失效；用户意图决定谁跟随谁。
 * 不保存并恢复主栏的旧 scrollTop，不修改内容、选区或正文高度。
 */
export function createMarkdownScrollSync(options: ScrollSyncOptions) {
  const requestFrame = options.requestFrame ?? requestAnimationFrame;
  const cancelFrame = options.cancelFrame ?? cancelAnimationFrame;
  let enabled = false;
  let paused = false;
  let documentId = '';
  let leader: SyncPane = 'semantic';
  let intent: 'scroll' | 'caret' | 'local' = 'scroll';
  let frame = 0;
  let revision = '';
  let groups: MarkdownSyncAnchor[][] = [];
  const geometry = new Map<number, SyncPoint | null>();
  const unavailableCode = new Set<number>();
  const expectedScroll = new Map<SyncPane, number>();
  const composing = new Set<SyncPane>();
  const dragging = new Set<SyncPane>();
  let lastStatus = '';

  function report(status: string) {
    const value = `${status}:${leader}:${revision}`;
    if (value === lastStatus) return;
    lastStatus = value;
    options.status?.(status, leader, revision);
  }

  function invalidateGeometry() {
    geometry.clear();
    unavailableCode.clear();
  }

  function schedule() {
    if (!enabled || paused || frame || composing.has(leader) || dragging.has(leader)) return;
    frame = requestFrame(run);
  }

  function readPoint(index: number): SyncPoint | null {
    if (geometry.has(index)) return geometry.get(index)!;
    const source = options.pane('source');
    const semantic = options.pane('semantic');
    if (!source || !semantic) return null;
    for (const anchor of groups[index] ?? []) {
      if (anchor.edge === 'line' && unavailableCode.has(anchor.pos)) continue;
      const semanticTop = semantic.position(anchor);
      if (semanticTop == null && anchor.edge === 'line') unavailableCode.add(anchor.pos);
      const sourceTop = semanticTop == null ? null : source.position(anchor);
      if (
        sourceTop != null &&
        semanticTop != null &&
        Number.isFinite(sourceTop) &&
        Number.isFinite(semanticTop)
      ) {
        const point = { source: sourceTop, semantic: semanticTop, anchor };
        geometry.set(index, point);
        return point;
      }
    }
    geometry.set(index, null);
    return null;
  }

  function findPoint(start: number, direction: 1 | -1) {
    for (let index = start; index >= 0 && index < groups.length; index += direction) {
      const point = readPoint(index);
      if (point) return { index, point };
    }
    return null;
  }

  function mapPosition(position: number) {
    let low = 0;
    let high = groups.length - 1;
    let before: SyncPoint | null = null;
    let after: SyncPoint | null = null;
    while (low <= high) {
      const mid = (low + high) >>> 1;
      const found = findPoint(mid, 1);
      if (!found || found.index > high) {
        high = mid - 1;
        continue;
      }
      if (found.point[leader] <= position) {
        before = found.point;
        low = found.index + 1;
      } else {
        after = found.point;
        high = mid - 1;
      }
    }
    before ??= findPoint(0, 1)?.point ?? null;
    after ??= findPoint(groups.length - 1, -1)?.point ?? null;
    if (!before || !after) return null;
    return interpolateSyncPosition(before, after, leader, position);
  }

  function run() {
    frame = 0;
    if (!enabled || paused || composing.has(leader) || dragging.has(leader)) return;
    // 内嵌代码编辑器没有可靠的跨栏光标坐标；布局仍正常失效，但不能沿用旧意图。
    if (intent === 'local') return;
    const snapshot = options.snapshot();
    if (!snapshot.ready) {
      report('waiting-for-content');
      return;
    }
    const nextRevision = `${documentId}:${snapshot.revision}`;
    if (revision !== nextRevision) {
      revision = nextRevision;
      invalidateGeometry();
      const ordered = [...snapshot.anchors].sort(
        (a, b) =>
          a.fromLine - b.fromLine ||
          (a.edge === 'end' ? 1 : 0) - (b.edge === 'end' ? 1 : 0) ||
          b.depth - a.depth,
      );
      groups = [];
      for (const anchor of ordered) {
        const last = groups.at(-1);
        if (last?.[0].fromLine === anchor.fromLine) last.push(anchor);
        else groups.push([anchor]);
      }
    }
    const follower: SyncPane = leader === 'source' ? 'semantic' : 'source';
    const active = options.pane(leader);
    const passive = options.pane(follower);
    if (
      !active ||
      !passive ||
      active.element.clientHeight <= 0 ||
      passive.element.clientHeight <= 0
    )
      return;
    const activeMax = Math.max(0, active.element.scrollHeight - active.element.clientHeight);
    const passiveMax = Math.max(0, passive.element.scrollHeight - passive.element.clientHeight);
    const position =
      intent === 'caret'
        ? active.caret(snapshot.anchors)
        : active.element.scrollTop + active.element.clientHeight * SYNC_REFERENCE_RATIO;
    if (position == null) {
      report('unmapped-caret');
      return;
    }
    const mapped = mapPosition(position);
    if (mapped == null) {
      report('unmapped-content');
      return;
    }
    let nextTop = mapped - passive.element.clientHeight * SYNC_REFERENCE_RATIO;
    if (intent === 'scroll') {
      if (activeMax <= 1) {
        report('short-document');
        return;
      }
      if (active.element.scrollTop <= 1) nextTop = 0;
      else if (active.element.scrollTop >= activeMax - 1) nextTop = passiveMax;
    } else {
      const visiblePosition = mapped - passive.element.scrollTop;
      if (
        visiblePosition >= passive.element.clientHeight * 0.15 &&
        visiblePosition <= passive.element.clientHeight * 0.85
      ) {
        report('caret-visible');
        return;
      }
    }
    nextTop = Math.max(0, Math.min(passiveMax, nextTop));
    if (Math.abs(passive.element.scrollTop - nextTop) > 1) {
      expectedScroll.set(follower, nextTop);
      passive.element.dataset.nomoSyncScroll = 'true';
      passive.element.scrollTop = nextTop;
    }
    report('following');
  }

  return {
    update(next: { enabled: boolean; documentId: string; activePane: SyncPane; paused: boolean }) {
      if (documentId !== next.documentId || enabled !== next.enabled) {
        if (frame) cancelFrame(frame);
        frame = 0;
        revision = '';
        invalidateGeometry();
        composing.clear();
        dragging.clear();
        leader = next.activePane;
        intent = 'scroll';
        expectedScroll.clear();
        for (const pane of ['source', 'semantic'] as const) {
          const element = options.pane(pane)?.element;
          if (element) delete element.dataset.nomoSyncScroll;
        }
      }
      documentId = next.documentId;
      enabled = next.enabled;
      paused = next.paused;
      schedule();
    },
    userIntent(pane: SyncPane, nextIntent: 'scroll' | 'caret' | 'local') {
      if (!enabled) return;
      if (frame) cancelFrame(frame);
      frame = 0;
      leader = pane;
      intent = nextIntent;
      expectedScroll.delete(pane);
      const element = options.pane(pane)?.element;
      if (element) delete element.dataset.nomoSyncScroll;
      schedule();
    },
    scroll(pane: SyncPane) {
      const element = options.pane(pane)?.element;
      const expected = expectedScroll.get(pane);
      if (element && expected != null && Math.abs(element.scrollTop - expected) <= 1) return;
      if (pane === leader && intent === 'scroll') schedule();
    },
    caretChanged(pane: SyncPane) {
      if (pane === leader && intent === 'caret') schedule();
    },
    layoutChanged() {
      invalidateGeometry();
      schedule();
    },
    composing(pane: SyncPane, value: boolean) {
      if (value) composing.add(pane);
      else {
        composing.delete(pane);
        schedule();
      }
    },
    dragging(pane: SyncPane, value: boolean) {
      if (value) dragging.add(pane);
      else {
        dragging.delete(pane);
        schedule();
      }
    },
    destroy() {
      enabled = false;
      if (frame) cancelFrame(frame);
      for (const pane of ['source', 'semantic'] as const) {
        const element = options.pane(pane)?.element;
        if (element) delete element.dataset.nomoSyncScroll;
      }
    },
  };
}
