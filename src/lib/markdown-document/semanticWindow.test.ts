// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { MarkdownSessionController } from './controller';
import { createMemoryMarkdownPort, readCompleteSnapshot } from './port';
import { MarkdownSemanticWindow } from './semanticWindow';
import { utf8Length } from './utf8Text';

describe('真实 ProseMirror 窗口接入会话', () => {
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
