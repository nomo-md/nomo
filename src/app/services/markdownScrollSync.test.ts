import { describe, expect, it } from 'vitest';
import type { MarkdownSyncAnchor } from '../../lib/editor-core/scrollSyncMapping';
import {
  createMarkdownScrollSync,
  interpolateSyncPosition,
  type SyncPane,
  type SyncPoint,
} from './markdownScrollSync';

const anchor = (line: number): MarkdownSyncAnchor => ({
  key: `p:${line}`,
  fromLine: line,
  toLine: line,
  pos: line,
  endPos: line + 1,
  kind: 'paragraph',
  edge: 'start',
  depth: 0,
});
const point = (source: number, semantic: number): SyncPoint => ({
  source,
  semantic,
  anchor: anchor(source),
});

describe('continuous content scroll mapping', () => {
  it('maps unequal distances continuously in both directions and clamps degenerate intervals', () => {
    const before = point(100, 200),
      after = point(200, 500);
    expect(interpolateSyncPosition(before, after, 'source', 150)).toBe(350);
    expect(interpolateSyncPosition(before, after, 'semantic', 350)).toBe(150);
    expect(interpolateSyncPosition(before, before, 'source', 500)).toBe(200);
    expect(interpolateSyncPosition(before, after, 'source', -100)).toBe(200);
    expect(interpolateSyncPosition(before, after, 'source', 1000)).toBe(500);
  });

  it('writes only to the follower, handles takeover and ignores its own scroll events', () => {
    const f = fixture();
    f.panes.source.scrollTop = 200;
    f.sync.userIntent('source', 'scroll');
    f.flush();
    expect(f.panes.source.scrollTop).toBe(200);
    expect(f.panes.semantic.scrollTop).toBe(960);
    f.sync.scroll('semantic');
    expect(f.frames.size).toBe(0);
    f.panes.semantic.scrollTop = 600;
    f.sync.userIntent('semantic', 'scroll');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(600);
    expect(f.panes.source.scrollTop).toBe(80);
    f.sync.destroy();
  });

  it('does not chase the old caret after manual scrolling and respects the visibility band', () => {
    const f = fixture();
    f.panes.semantic.scrollTop = 400;
    f.caret.source = 200;
    f.sync.userIntent('source', 'caret');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(400);
    f.caret.source = 600;
    f.sync.caretChanged('source');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(1620);
    f.panes.source.scrollTop = 200;
    f.sync.userIntent('source', 'scroll');
    f.flush();
    f.caret.source = 0;
    f.sync.caretChanged('source');
    expect(f.frames.size).toBe(0);
    f.sync.destroy();
  });

  it('holds during composition, selection drag and missing revisions, then uses only the latest intent', () => {
    const f = fixture();
    f.sync.composing('source', true);
    f.sync.userIntent('source', 'caret');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.sync.composing('source', false);
    f.sync.dragging('source', true);
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.sync.dragging('source', false);
    f.state.ready = false;
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.sync.userIntent('semantic', 'scroll');
    f.state.ready = true;
    f.state.revision = 'next';
    f.panes.semantic.scrollTop = 600;
    f.sync.layoutChanged();
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(600);
    expect(f.panes.source.scrollTop).toBe(80);
    f.sync.destroy();
  });

  it('lets local code interaction cancel stale following without disabling subsequent layout sync', () => {
    const f = fixture();
    f.panes.source.scrollTop = 200;
    f.panes.semantic.scrollTop = 600;
    f.sync.userIntent('source', 'scroll');
    f.sync.userIntent('semantic', 'local');
    f.sync.layoutChanged();
    f.sync.caretChanged('semantic');
    f.flush();
    expect(f.panes.source.scrollTop).toBe(200);
    expect(f.panes.semantic.scrollTop).toBe(600);
    f.sync.userIntent('semantic', 'scroll');
    f.flush();
    expect(f.panes.source.scrollTop).toBe(80);
    f.sync.destroy();
  });

  it('clamps document edges, skips invalid anchors and cancels disabled work', () => {
    const f = fixture();
    f.state.anchors.splice(1, 0, { ...anchor(2), key: 'missing' });
    f.sync.userIntent('source', 'scroll');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.panes.source.scrollTop = 1400;
    f.sync.userIntent('source', 'scroll');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(4400);
    f.sync.userIntent('source', 'scroll');
    f.sync.update({ enabled: false, documentId: 'other', activePane: 'source', paused: false });
    expect(f.frames.size).toBe(0);
    f.sync.destroy();
  });

  it('coalesces layout changes, preserves focus and selections, and pauses completely without a reliable map', () => {
    const f = fixture();
    document.body.append(f.panes.source);
    f.panes.source.tabIndex = 0;
    f.panes.source.focus();
    f.panes.source.scrollTop = 200;
    f.sync.userIntent('source', 'scroll');
    for (let i = 0; i < 20; i += 1) f.sync.layoutChanged();
    expect(f.frames.size).toBe(1);
    f.flush();
    expect(document.activeElement).toBe(f.panes.source);
    expect(f.panes.source.scrollTop).toBe(200);
    const following = f.panes.semantic.scrollTop;
    f.state.anchors = [{ ...anchor(1), key: 'missing' }];
    f.state.revision = 'unmapped';
    f.panes.source.scrollTop = 400;
    f.sync.layoutChanged();
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(following);
    f.sync.destroy();
    f.panes.source.remove();
  });

  it('merges duplicate starts, stays monotone, and resumes from the current leader after resizing', () => {
    const f = fixture();
    f.state.anchors.unshift({ ...anchor(1), key: 'missing', depth: 2 });
    f.sync.update({ enabled: true, documentId: 'doc', activePane: 'source', paused: true });
    f.panes.source.scrollTop = 300;
    f.sync.userIntent('source', 'scroll');
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.sync.update({ enabled: true, documentId: 'doc', activePane: 'source', paused: false });
    f.flush();
    let last = f.panes.semantic.scrollTop;
    for (let top = 310; top <= 600; top += 10) {
      f.panes.source.scrollTop = top;
      f.sync.scroll('source');
      f.flush();
      expect(f.panes.semantic.scrollTop).toBeGreaterThanOrEqual(last);
      last = f.panes.semantic.scrollTop;
    }
    f.sync.userIntent('semantic', 'scroll');
    f.sync.update({
      enabled: true,
      documentId: 'new-document',
      activePane: 'source',
      paused: false,
    });
    f.panes.source.scrollTop = 0;
    f.flush();
    expect(f.panes.semantic.scrollTop).toBe(0);
    f.sync.destroy();
  });
});

function fixture() {
  const panes = { source: document.createElement('div'), semantic: document.createElement('div') };
  Object.defineProperties(panes.source, {
    clientHeight: { value: 600 },
    scrollHeight: { value: 2000 },
  });
  Object.defineProperties(panes.semantic, {
    clientHeight: { value: 600 },
    scrollHeight: { value: 5000 },
  });
  const frames = new Map<number, FrameRequestCallback>();
  let id = 0;
  const state = { ready: true, revision: 'one', anchors: [anchor(1), anchor(1000)] };
  const caret = { source: 0, semantic: 0 };
  const sync = createMarkdownScrollSync({
    snapshot: () => state,
    pane: (pane: SyncPane) => ({
      element: panes[pane],
      caret: () => caret[pane],
      position: (item) =>
        item.key === 'missing'
          ? null
          : (item.fromLine === 1 ? 0 : 1000) * (pane === 'source' ? 1 : 3),
    }),
    requestFrame: (callback) => {
      frames.set(++id, callback);
      return id;
    },
    cancelFrame: (frame) => {
      frames.delete(frame);
    },
  });
  sync.update({ enabled: true, documentId: 'doc', activePane: 'source', paused: false });
  const flush = () => {
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach((callback) => callback(0));
  };
  return { panes, frames, state, caret, sync, flush };
}
