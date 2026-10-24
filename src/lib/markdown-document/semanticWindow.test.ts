// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import * as markdownParser from '../editor-core/markdown';
import { DecorationSet, type EditorView } from 'prosemirror-view';
import { MarkdownSessionController } from './controller';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownSemanticWindow } from './semanticWindow';
import { utf8Length } from './utf8Text';

describe('真实 ProseMirror 窗口接入会话', () => {
  it('原生目录范围通知保留未触及的 PM 实例并重定位源文坐标', async () => {
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text: 'toc\n\nBody' }));
    const target = document.body.appendChild(document.createElement('div'));
    const view = new MarkdownSemanticWindow(session, await port.readWindow(session.documentId,0,5,9),
      {target,onStale(){},onHistory(){},onFailure(error){throw error;}});
    view.activate(); await view.whenReady;
    const dom=target.querySelector('.ProseMirror');
    await session.commitNativeMutation({revision:0,generation:0},new AbortController().signal,async()=>({
      status: await port.applyTransaction(session.documentId,{baseRevision:0,operationId:'toc',historyGroupId:'toc',derived:true,edits:[{fromByte:0,toByte:3,insertedText:'long toc'}]}),
      nativeEdits:[{fromByte:0,toByte:3,insertedBytes:8}],
    }));
    expect(view.bridge.isStale).toBe(false); expect(view.bridge.window.fromByte).toBe(10);
    expect(target.querySelector('.ProseMirror')).toBe(dom); expect(target.textContent).toContain('Body');
    view.dispose(); target.remove(); await session.close(true);
  });
  it('相同源文选区不更新视图，实际选区及待输入格式变化仍完整恢复', async () => {
    const text = '# Header\n\nBody';
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text }));
    const target = document.createElement('div');
    const window = new MarkdownSemanticWindow(session, await port.readWindow(session.documentId, 0, 0, utf8Length(text)),
      { target, onStale() {}, onHistory() {}, onFailure(error) { throw error; } });
    try {
      window.prepareAttachment(); document.body.append(target); window.activate();
      const view = (window.core as unknown as { view: EditorView }).view;
      const dispatch = vi.spyOn(view, 'dispatch');
      expect(window.core.restoreSourceSelection({ anchorByte: 0, headByte: 0 })).toBe(true);
      expect(dispatch).not.toHaveBeenCalled();
      view.dispatch(view.state.tr.addStoredMark(view.state.schema.marks.strong.create()));
      dispatch.mockClear();
      window.core.restoreSourceSelection({ anchorByte: 0, headByte: 0 });
      expect(dispatch).toHaveBeenCalledOnce(); expect(view.state.storedMarks).toBeNull();
      dispatch.mockClear();
      window.core.restoreSourceSelection({ anchorByte: 2, headByte: 8 });
      expect(view.state.doc.textBetween(view.state.selection.from, view.state.selection.to)).toBe('Header');
      expect(dispatch).toHaveBeenCalledOnce();
      expect(session.state.committed).toMatchObject({ revision: 0, dirty: false, canUndo: false });
    } finally { window.dispose(); target.remove(); await session.close(true); }
  });
  it('窗口构建与结构测量共享解析，纯排版只刷新几何版本，编辑后重新建立来源', async () => {
    const text = '# 标题\n\n| 列 | 内容 |\n| --- | --- |\n| a | [引用][ref] |\n\n末段';
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text }));
    const parse = vi.spyOn(markdownParser, 'parseMarkdownWithSyncAnchors');
    const target = document.createElement('div');
    const view = new MarkdownSemanticWindow(session, await port.readWindow(session.documentId, 0, 0, utf8Length(text)),
      { target, references: { REF: { href: 'https://example.com', title: '' } },
        onStale() {}, onHistory() {}, onFailure(error) { throw error; } });
    try {
      view.prepareAttachment(); document.body.append(target); view.activate();
      const before = view.core.getScrollSyncSnapshot();
      view.core.measureStructureBlocks();
      expect(parse).toHaveBeenCalledTimes(1);
      expect(before.ready).toBe(true);
      expect(target.querySelector('a')?.getAttribute('href')).toBe('https://example.com');
      const editorView = (view.core as unknown as { view: EditorView }).view;
      editorView.dispatch(editorView.state.tr.setMeta('nomo:typography', DecorationSet.empty));
      const layout = view.core.getScrollSyncSnapshot();
      expect(layout.anchors).toBe(before.anchors);
      expect(layout.renderRevision).toBeGreaterThan(before.renderRevision);
      expect(parse).toHaveBeenCalledTimes(1);
      view.core.restoreSelectionSnapshot({ anchor: 1, head: 1 });
      view.core.execute({ type: 'insertText', text: '新' });
      await session.flushEdits();
      const after = view.core.getScrollSyncSnapshot();
      expect(after.revision).toBeGreaterThan(before.revision);
      expect(after.markdown).toContain('新标题');
      expect(after.anchors).not.toBe(before.anchors);
      expect(after.anchors.some(anchor => anchor.kind === 'table')).toBe(true);
      expect(await session.withSnapshot(snapshot => readCompleteSnapshot(port, snapshot))).toBe(text.replace('标题', '新标题'));
    } finally {
      parse.mockRestore(); view.dispose(); target.remove(); await session.close(true);
    }
  });
  it.each(['ready', 'readonly', 'stale', 'failure'] as const)('导航接入前准备保留正文就绪和编辑门禁：%s', async (mode) => {
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text: '正文' }));
    const target = document.createElement('div');
    const view = new MarkdownSemanticWindow(session, await port.readWindow(session.documentId, 0, 0, 6),
      { target, readonly: mode === 'readonly', onStale() {}, onHistory() {}, onFailure(error) { throw error; } });
    if (mode === 'failure') view.setFailure(true);
    if (mode === 'stale') await session.applyEdits({ viewGeneration: 0, historyGroupId: 'other',
      edits: [{ fromByte: 0, toByte: 6, insertedText: '新版' }] });
    const dom = target.querySelector('.ProseMirror');
    view.prepareAttachment();
    expect(view.bodyReady).toBe(false);
    expect(view.editable).toBe(false);
    expect(dom?.getAttribute('contenteditable')).toBe(mode === 'ready' ? 'true' : 'false');
    document.body.append(target); view.activate(); await view.whenReady;
    expect(view.bodyReady).toBe(mode !== 'stale');
    expect(view.editable).toBe(mode === 'ready');
    expect(target.querySelector('.ProseMirror')).toBe(dom);
    view.core.restoreSelectionSnapshot({ anchor: 1, head: 1 });
    expect(view.core.execute({ type: 'insertText', text: '新' })).toBe(mode === 'ready');
    await session.flushEdits();
    expect(await session.withSnapshot(snapshot => readCompleteSnapshot(port, snapshot)))
      .toBe(mode === 'ready' ? '新正文' : mode === 'stale' ? '新版' : '正文');
    view.dispose(); target.remove(); await session.close(true);
  });
  it('粘贴与之前的连续键入分组撤销，视图回收不制造历史', async () => {
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text: '正文' }));
    const target = document.createElement('div');
    document.body.append(target);
    const failures: unknown[] = [];
    const view = new MarkdownSemanticWindow(
      session,
      await port.readWindow(session.documentId, 0, 0, 6),
      { target, onStale() {}, onHistory() {}, onFailure: (error) => failures.push(error) },
    );
    view.activate();
    await view.whenReady;
    view.core.restoreSelectionSnapshot({ anchor: 1, head: 1 });
    view.core.execute({ type: 'insertText', text: 'A' });
    view.core.execute({ type: 'insertText', text: 'B' });
    view.core.pasteClipboard({ text: 'C' });
    await session.flushEdits();
    expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      'ABC正文',
    );
    view.dispose();
    target.remove();
    await session.undo();
    expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      'AB正文',
    );
    await session.undo();
    expect(session.state.committed.dirty).toBe(false);
    expect(failures).toEqual([]);
    await session.close();
  });
  it('另一窗口之前的插入只重定位当前窗口，接续输入写入新全局位置', async () => {
    const prefix = '前文\n\n';
    const tail = '后文\n';
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(
      port,
      await port.openDocument({ text: prefix + tail }),
    );
    const failures: unknown[] = [];
    let stale = 0;
    const targets = [document.createElement('div'), document.createElement('div')];
    document.body.append(...targets);
    const first = new MarkdownSemanticWindow(
      session,
      await port.readWindow(session.documentId, 0, 0, utf8Length(prefix)),
      {
        target: targets[0],
        onStale() {},
        onHistory() {},
        onFailure: (error) => failures.push(error),
      },
    );
    const second = new MarkdownSemanticWindow(
      session,
      await port.readWindow(session.documentId, 0, utf8Length(prefix), utf8Length(prefix + tail)),
      {
        target: targets[1],
        onStale: () => {
          stale++;
        },
        onHistory() {},
        onFailure: (error) => failures.push(error),
      },
    );
    first.activate(); second.activate();
    await Promise.all([first.whenReady, second.whenReady]);
    first.core.restoreSelectionSnapshot({ anchor: 1, head: 1 });
    first.core.execute({ type: 'insertText', text: 'A' });
    first.core.commitPendingEdits();
    second.core.restoreSelectionSnapshot({ anchor: 1, head: 1 });
    second.core.execute({ type: 'insertText', text: 'B' });
    await session.flushEdits();
    expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      'A' + prefix + 'B' + tail,
    );
    expect(stale).toBe(0);
    expect(failures).toEqual([]);
    first.dispose();
    second.dispose();
    targets.forEach((target) => target.remove());
    await session.close(true);
  });
  it('只挂载中间窗口，编辑后保存及撤销仍覆盖完整首尾', async () => {
    const prefix = '# 前文\r\n\r\n';
    const middle = '+ 正文\r\n+ 保留  \r\n';
    const tail = '\r\n未挂载尾部😀  \n';
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(
      port,
      await port.openDocument({ text: prefix + middle + tail }),
    );
    const window = await port.readWindow(
      session.documentId,
      0,
      utf8Length(prefix),
      utf8Length(prefix + middle),
    );
    const target = document.createElement('div');
    document.body.append(target);
    const failures: unknown[] = [];
    const view = new MarkdownSemanticWindow(session, window, {
      target,
      onStale() {},
      onHistory() {},
      onFailure: (error) => failures.push(error),
    });
    expect(target.textContent).not.toContain('未挂载');
    view.activate();
    await view.whenReady;
    view.core.restoreSelectionSnapshot({ anchor: 3, head: 3 });
    expect(view.core.execute({ type: 'insertText', text: '新' })).toBe(true);
    await session.flushEdits();
    expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      prefix + middle.replace('正文', '新正文') + tail,
    );
    expect(failures).toEqual([]);
    view.dispose();
    target.remove();
    await session.undo();
    expect(session.state.committed.dirty).toBe(false);
    expect(await session.withSnapshot((snapshot) => readCompleteSnapshot(port, snapshot))).toBe(
      prefix + middle + tail,
    );
    await session.close();
  });
  it('预览期间修订过期，生命周期就绪或选项更新都不能重新开放写入', async () => {
    const port = createMemoryMarkdownPort();
    const session = new MarkdownSessionController(port, await port.openDocument({ text: '正文' }));
    const target = document.body.appendChild(document.createElement('div'));
    const view = new MarkdownSemanticWindow(session, await port.readWindow(session.documentId, 0, 0, 6),
      { target, onStale() {}, onHistory() {}, onFailure(error) { throw error; } });
    const dom = target.querySelector('.ProseMirror');
    view.updateOptions({ readonly: false });
    expect(view.core.execute({ type: 'insertText', text: '误' })).toBe(false);
    await session.applyEdits({ viewGeneration: 0, historyGroupId: 'other', edits: [{ fromByte: 0, toByte: 6, insertedText: '新版' }] });
    view.activate(); await view.whenReady;
    view.updateOptions({ readonly: false });
    expect(view.editable).toBe(false);
    expect(target.querySelector('.ProseMirror')).toBe(dom);
    expect(await session.withSnapshot(snapshot => readCompleteSnapshot(port, snapshot))).toBe('新版');
    view.dispose(); target.remove(); await session.close(true);
  });
});
