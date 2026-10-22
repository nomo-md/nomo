// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MarkdownApplicationSession } from './markdownApplicationSession';
import { createMarkdownTab } from './tabs';
import { ProgressiveMarkdownDocument } from '../../lib/markdown-document/runtime';
import { buildMarkdownIndex } from '../../lib/markdown-document/incrementalIndex';
import { MarkdownStructureScanner } from '../../lib/markdown-document/structure';
import { captureWordSnapshot } from './word/snapshot';
import { createPersistedWorkspaceState } from './workspacePersistence';
import { utf8Length } from '../../lib/markdown-document/utf8Text';
import { createMemoryMarkdownPort } from '../../lib/markdown-document/port';

const localBuild: typeof buildMarkdownIndex = (port, snapshot, options) =>
  buildMarkdownIndex(port, snapshot, {
    ...options,
    scan: async (port, snapshot, task) => {
      const scanner = new MarkdownStructureScanner(snapshot.revision, task.checkpoint);
      let from = task.checkpoint?.fromByte ?? 0;
      do {
        task.signal.throwIfAborted();
        const part = await port.readSnapshot(
          snapshot,
          from,
          Math.min(snapshot.byteLength, from + 16 * 1024),
        );
        from = part.toByte;
        if ((await task.onBatch(scanner.push(part.text, from === snapshot.byteLength))) === false)
          return;
      } while (from < snapshot.byteLength);
    },
  });
const applications: MarkdownApplicationSession[] = [];
afterEach(async () => {
  for (const app of applications.splice(0)) await app.close(true);
  vi.restoreAllMocks();
  document.body.replaceChildren();
});
async function setup(text: string, onRequestSource?: (isCurrent: () => boolean) => Promise<boolean>) {
  vi.spyOn(ProgressiveMarkdownDocument, 'open').mockImplementation(
    async (port, input) =>
      new ProgressiveMarkdownDocument(port, await port.openDocument(input), localBuild),
  );
  const tab = createMarkdownTab({ markdown: text, savedMarkdown: text });
  const failures: unknown[] = [];
  const application = new MarkdownApplicationSession(
    tab,
    false,
    { markdown: text, savedMarkdown: text },
    (error) => failures.push(error),
    () => {},
    undefined,
    onRequestSource,
  );
  applications.push(application);
  const doc = await application.ready;
  return { application, doc, tab, failures };
}

describe('应用 Markdown 会话入口', () => {
  it('打开尚未结束时销毁宿主，清理等待打开收口且只执行一次，不发布迟到元数据', async () => {
    const port = createMemoryMarkdownPort();
    let release!: () => void;
    const opening = new Promise<void>(resolve => { release = resolve; });
    let detached!: ReturnType<typeof vi.spyOn>;
    vi.spyOn(ProgressiveMarkdownDocument, 'open').mockImplementation(async () => {
      await opening;
      const doc = new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '恢复正文' }), localBuild);
      detached = vi.spyOn(doc, 'detach');
      return doc;
    });
    const metadata = vi.fn(), failure = vi.fn();
    const app = new MarkdownApplicationSession(createMarkdownTab(), false, { markdown: '' }, failure, metadata);
    const disposal = app.detach();
    expect(app.detach()).toBe(disposal);
    let done = false;
    void disposal.then(() => { done = true; });
    await Promise.resolve();
    expect(done).toBe(false);
    release();
    const doc = await app.ready;
    await disposal;
    expect(detached).toHaveBeenCalledOnce();
    expect(metadata).not.toHaveBeenCalled();
    expect(failure).not.toHaveBeenCalled();
    await port.close(doc.session.documentId, true);
  });

  it('已销毁宿主的迟到打开失败不再通知当前标签', async () => {
    let reject!: (error: Error) => void;
    vi.spyOn(ProgressiveMarkdownDocument, 'open').mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
    const failure = vi.fn();
    const app = new MarkdownApplicationSession(createMarkdownTab(), false, { markdown: '' }, failure, () => {});
    const disposal = app.detach();
    reject(new Error('Markdown 会话不存在或已关闭'));
    await disposal;
    expect(failure).not.toHaveBeenCalled();
  });
  it('关闭存储失败后恢复正文挂载，并能继续编辑、派生目录和保存快照', async () => {
    const { application, doc } = await setup('# 原标题\n\n<!-- toc -->\n- 手写目录\n<!-- /toc -->');
    const target = document.body.appendChild(document.createElement('div'));
    application.editor.mount(target);
    await vi.waitFor(() => expect(application.writable).toBe(true));
    vi.spyOn(doc.port, 'close').mockRejectedValueOnce(new Error('关闭存储失败'));
    await expect(application.close(true)).rejects.toThrow('关闭存储失败');
    await vi.waitFor(() => expect(application.writable).toBe(true));
    expect(target.querySelector('.ProseMirror')).not.toBeNull();
    await doc.session.applyEdits({ viewGeneration: doc.session.state.viewGeneration,
      historyGroupId: 'after-close-failed', edits: [{ fromByte: 0, toByte: 0, insertedText: '# 新标题\n\n' }] });
    expect(await application.readMarkdown()).toContain('[新标题](#新标题)');
    await doc.session.save('after-close-failed.md');
    expect(doc.state.status.dirty).toBe(false);
  });
  it('已保存文件的恢复目录缺失时从原文件重新建立会话并更新持久化身份', async () => {
    const port = createMemoryMarkdownPort();
    const open = vi.spyOn(ProgressiveMarkdownDocument, 'open').mockImplementation(async (_, input) => {
      if ('recoveryId' in input) throw Object.assign(new Error('恢复资料已不存在'), { code: 'recovery-missing' });
      expect(input).toEqual({ path: 'C:/docs/saved.md' });
      return new ProgressiveMarkdownDocument(port, await port.openDocument({ text: '磁盘原文' }), localBuild);
    });
    const tab = Object.assign(createMarkdownTab({ nativePath: 'C:/docs/saved.md', dirty: false }), { documentId: 'md-missing' });
    const failures = vi.fn();
    const app = new MarkdownApplicationSession(tab, true, { markdown: '' }, failures, () => {});
    applications.push(app);
    await app.ready;
    expect(await app.readMarkdown()).toBe('磁盘原文');
    expect(open).toHaveBeenCalledTimes(2);
    expect(tab.documentId).not.toBe('md-missing');
    expect(failures).not.toHaveBeenCalled();
  });

  it.each([
    { dirty: true, path: 'C:/docs/saved.md', code: 'recovery-missing' },
    { dirty: false, path: null, code: 'recovery-missing' },
    { dirty: false, path: 'C:/docs/saved.md', code: 'recovery-corrupt' },
    { dirty: false, path: 'C:/docs/saved.md', code: 'io-error' },
  ])('未保存修改、无原路径或恢复资料损坏时不自动替换正文：%j', async ({ dirty, path, code }) => {
    const failure = Object.assign(new Error('恢复失败'), { code });
    const open = vi.spyOn(ProgressiveMarkdownDocument, 'open').mockRejectedValue(failure);
    const tab = Object.assign(createMarkdownTab({ nativePath: path, dirty }), { documentId: 'md-missing' });
    const app = new MarkdownApplicationSession(tab, true, { markdown: '' }, () => {}, () => {});
    applications.push(app);
    await expect(app.ready).rejects.toBe(failure);
    expect(open).toHaveBeenCalledOnce();
    expect(tab.documentId).toBe('md-missing');
    expect(tab.dirty).toBe(dirty);
  });
  it('双栏的后台源码挂载不会接管语义栏的有效焦点目标', async () => {
    const { application } = await setup('正文');
    const semantic = document.body.appendChild(document.createElement('div'));
    application.editor.mount(semantic);
    await vi.waitFor(() => expect(application.writable).toBe(true));
    await application.setViewMode('split');
    const source = document.body.appendChild(document.createElement('div'));
    application.mountSource(source);
    await vi.waitFor(() => expect(source.querySelector('.cm-editor')).not.toBeNull());
    application.editor.focus();
    expect(semantic.contains(document.activeElement)).toBe(true);
    source.querySelector<HTMLElement>('.cm-content')!.focus();
    application.editor.focus();
    expect(source.contains(document.activeElement)).toBe(true);
  });

  it('阅读后首次进入双栏，迟到的源码宿主接续当前位置', async () => {
    const { application, tab } = await setup('巨'.repeat(20000));
    const semantic = document.body.appendChild(document.createElement('div'));
    application.editor.mount(semantic);
    await vi.waitFor(() => expect(semantic.querySelector('.markdown-session-restricted')).not.toBeNull());
    await application.reveal(3000, false);
    await application.setViewMode('split');
    const source = document.body.appendChild(document.createElement('div'));
    const unmount = application.mountSource(source);
    await vi.waitFor(() => expect(source.querySelector('.cm-editor')).not.toBeNull());
    expect(tab.globalScrollAnchor?.byteOffset).toBe(3000);
    // 挂载完成后由共享的字节导航定位，不能退回仍在文首的编辑选区。
    await vi.waitFor(() => expect((application as any).sourceHost.position.byteOffset).toBe(3000));
    unmount();
  });

  it('关闭后的元数据读取失败不会覆盖新标签的状态', async () => {
    const { application, doc, failures } = await setup('正文');
    let rejectRead!: (error: Error) => void;
    const read = vi.spyOn(doc.port, 'readWindow').mockImplementationOnce(() =>
      new Promise((_, reject) => { rejectRead = reject; }));
    await doc.session.applyEdits({
      viewGeneration: doc.session.state.viewGeneration,
      historyGroupId: 'metadata', edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }],
    });
    await vi.waitFor(() => expect(read).toHaveBeenCalled());
    await application.close(true);
    applications.splice(applications.indexOf(application), 1);
    rejectRead(new Error('文档会话已关闭。'));
    await Promise.resolve(); await Promise.resolve();
    expect(failures).toEqual([]);
  });

  it('等待中的源码请求在用户切换目标后失效，不再触发模式和焦点恢复', async () => {
    let release!: () => void;
    let current: (() => boolean) | undefined;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const { application } = await setup('正文', async isCurrent => { current = isCurrent; await gate; return isCurrent(); });
    const request = application.openSource(0);
    await vi.waitFor(() => expect(current).toBeDefined());
    await application.setViewMode('semantic');
    expect(current!()).toBe(false);
    release(); await request;
    const target = document.body.appendChild(document.createElement('div'));
    application.editor.mount(target);
    await vi.waitFor(() => expect(application.writable).toBe(true));
    expect(target.querySelector('.ProseMirror')).not.toBeNull();
    expect(target.contains(document.activeElement)).toBe(false);
  });

  it('双栏的源码入口只定位源码栏，语义提示保持原位', async () => {
    const request = vi.fn(async () => true);
    const { application } = await setup('巨'.repeat(20000), request);
    await application.setViewMode('split');
    const semantic = document.body.appendChild(document.createElement('div'));
    const source = document.body.appendChild(document.createElement('div'));
    application.editor.mount(semantic);
    const unmount = application.mountSource(source);
    await vi.waitFor(() => expect(semantic.querySelector('.markdown-session-restricted')).not.toBeNull());
    await vi.waitFor(() => expect(source.querySelector('.cm-editor')).not.toBeNull());
    // 源码栏也可能尚处于原有的未索引窗口，先完成其正常的结构窗口接管。
    await application.openSource(0);
    request.mockClear();
    const hint = semantic.querySelector('.markdown-session-restricted');
    await application.openSource(3000);
    expect(request).toHaveBeenCalledOnce();
    expect(application.captureSelection()?.headByte).toBe(3000);
    expect(semantic.querySelector('.markdown-session-restricted')).toBe(hint);
    expect(source.querySelector('.cm-editor')).not.toBeNull();
    unmount();
  });

  it('组合成员共用模式切换时，每篇文档仍保留独立会话及源码位置', async () => {
    let members: MarkdownApplicationSession[] = [];
    const request = async () => { await Promise.all(members.map(member => member.setViewMode('source'))); return true; };
    const left = await setup('左'.repeat(20000), request);
    const right = await setup('右'.repeat(20000), request);
    members = [left.application, right.application];
    const targets = members.map(() => document.body.appendChild(document.createElement('div')));
    members.forEach((member, i) => member.editor.mount(targets[i]));
    await vi.waitFor(() => expect(targets.every(target => target.querySelector('.markdown-session-restricted'))).toBe(true));
    await left.application.openSource(6000);
    const sourceTargets = members.map(() => document.body.appendChild(document.createElement('div')));
    const cleanups = members.map((member, i) => member.mountSource(sourceTargets[i]));
    await vi.waitFor(() => expect(sourceTargets.every(target => target.querySelector('.cm-editor'))).toBe(true));
    expect(left.application.captureSelection()?.headByte).toBe(6000);
    expect(right.application.captureSelection()?.headByte).toBe(0);
    expect(left.doc.session.documentId).not.toBe(right.doc.session.documentId);
    expect(left.doc.state.status.dirty || right.doc.state.status.dirty).toBe(false);
    cleanups.forEach(cleanup => cleanup());
  });

  it('源码宿主延迟创建期间转向其他控件，保留定位但不抢回焦点', async () => {
    const { application } = await setup('巨'.repeat(20000));
    const target = document.body.appendChild(document.createElement('div'));
    application.editor.mount(target);
    await vi.waitFor(() => expect(target.querySelector('.markdown-session-restricted')).not.toBeNull());
    await application.openSource(3000);
    const button = document.body.appendChild(document.createElement('button'));
    button.focus();
    const source = document.body.appendChild(document.createElement('div'));
    const unmount = application.mountSource(source);
    try {
      await vi.waitFor(() => expect(application.captureSelection()?.headByte).toBe(3000));
      expect(document.activeElement).toBe(button);
    } finally { unmount(); }
  });

  it('受限结构主动进入源码，等待源码宿主创建后仍定位原目标', async () => {
    const { application, doc } = await setup('巨'.repeat(20000));
    const pane = document.body.appendChild(document.createElement('div'));
    pane.className = 'semantic-pane';
    const target = pane.appendChild(document.createElement('div'));
    application.editor.mount(target);
    await vi.waitFor(() => expect(target.querySelector('.markdown-session-restricted')).not.toBeNull());
    await application.reveal(30000);
    target.querySelector<HTMLButtonElement>('button')!.click();
    await vi.waitFor(() => expect(target.querySelector('.markdown-session-restricted')).toBeNull());
    const sourcePane = document.body.appendChild(document.createElement('div'));
    sourcePane.className = 'source-pane';
    const sourceTarget = sourcePane.appendChild(document.createElement('div'));
    const unmountSource = application.mountSource(sourceTarget);
    await vi.waitFor(() => expect(sourceTarget.querySelector('.cm-editor')).not.toBeNull());
    await vi.waitFor(() => expect(application.captureSelection()?.headByte).toBe(30000));
    expect(application.writable).toBe(true);
    await application.setViewMode('semantic');
    await vi.waitFor(() => expect(target.querySelector('.markdown-session-restricted')).not.toBeNull());
    expect(application.readingByte).toBe(30000);
    expect(doc.state.status.revision).toBe(0);
    unmountSource();
  });

  it('预览的工具栏、异步插入均不可写，已有书签在应用只读后也不能提交', async () => {
    const { application, doc } = await setup('正文');
    const target = document.body.appendChild(document.createElement('div'));
    application.editor.mount(target);
    expect(application.editor.execute({ type: 'insertText', text: '误输入' })).toBe(false);
    expect(() => application.createInsertionBookmark()).toThrow('尚未开放');
    await vi.waitFor(() => expect(application.writable).toBe(true));
    const bookmark = application.createInsertionBookmark();
    application.editor.updateOptions({ readonly: true });
    await expect(bookmark.insert('误图片')).rejects.toThrow('失效');
    expect(application.editor.canExecute({ type: 'insertText', text: '' })).toBe(false);
    expect(await doc.readMarkdown()).toBe('正文');
    bookmark.dispose();
    application.editor.updateOptions({ readonly: false });
    expect(application.writable).toBe(true);
  });

  it('挂载前的聚焦请求在用户转向其他控件后取消', async () => {
    const { application } = await setup('正文');
    const target = document.body.appendChild(document.createElement('div'));
    const button = document.body.appendChild(document.createElement('button'));
    application.editor.mount(target);
    application.editor.focus();
    button.focus();
    await vi.waitFor(() => expect(application.writable).toBe(true));
    expect(document.activeElement).toBe(button);
  });

  it('标签退出全文热路径，Word 导出读取完整会话，保存旧快照后新输入仍为未保存', async () => {
    const text = '# 标题\r\n\r\n' + '正文\n\n'.repeat(3500) + '尾部';
    const { application, doc, tab, failures } = await setup(text);
    expect(tab.markdown).toBe('');
    expect(tab.savedMarkdown).toBe('');
    expect(() => application.editor.flushMarkdown()).toThrow('全文');
    const exported = await captureWordSnapshot(
      tab,
      { disposed: false, editor: application.editor, session: application },
      {
        signal: new AbortController().signal,
        isAvailable: () => true,
        flush: () => {},
      },
    );
    expect(exported?.markdown === text).toBe(true);
    await doc.session.withSnapshot(async (snapshot) => {
      await doc.session.applyEdits({
        viewGeneration: doc.session.state.viewGeneration,
        edits: [{ fromByte: 0, toByte: 0, insertedText: '新' }],
        historyGroupId: 'later',
      });
      await doc.session.saveCaptured(snapshot, 'test.md');
    });
    expect(tab.dirty).toBe(true);
    await doc.session.undo();
    expect(tab.dirty).toBe(false);
    expect(failures).toEqual([]);
  });

  it('新建焦点等真实窗口挂载，异步图片书签随前方编辑映射，隐藏后导入不丢内容', async () => {
    const { application, doc, failures } = await setup('正文');
    const pane = document.createElement('div');
    pane.className = 'semantic-pane';
    const target = document.createElement('div');
    pane.append(target);
    document.body.append(pane);
    application.editor.mount(target);
    application.editor.focus();
    await vi.waitFor(() => expect(target.contains(document.activeElement)).toBe(true));
    await application.reveal(doc.state.status.byteLength);
    await application.prepareTransfer();
    expect(() => application.editor.commitPendingEdits()).not.toThrow();
    application.resume();
    await vi.waitFor(() => expect(target.querySelector('.ProseMirror, .cm-editor')).not.toBeNull());
    await application.reveal(doc.state.status.byteLength);
    const bookmark = application.createInsertionBookmark();
    await application.unmount();
    await doc.session.applyEdits({
      viewGeneration: doc.session.state.viewGeneration,
      edits: [{ fromByte: 0, toByte: 0, insertedText: '前方' }],
      historyGroupId: 'prefix',
    });
    await bookmark.insert('![图](asset.png)');
    bookmark.dispose();
    expect(await application.readMarkdown()).toBe('前方正文![图](asset.png)');
    await doc.session.undo();
    expect(await application.readMarkdown()).toBe('前方正文');
    expect(failures).toEqual([]);
  });

  it('诊断导航越过长行的读取窗口后仍使用完整 UTF-8 字节位置', async () => {
    const longLine = '长'.repeat(30000);
    const { application, failures } = await setup(longLine + '\r\n尾😀部');
    const reveal = vi.spyOn(application, 'reveal').mockResolvedValue(undefined);
    await application.revealLine(1, 27001);
    expect(reveal).toHaveBeenLastCalledWith(81000);
    await application.revealLine(2, 4);
    expect(reveal).toHaveBeenLastCalledWith(utf8Length(longLine + '\r\n尾😀'));
    expect(failures).toEqual([]);
  });

  it('恢复和跨窗元数据只存会话身份及全局位置，保留旧草稿作为迁移回退资料', async () => {
    const { tab } = await setup('完整内容');
    tab.documentId = 'md-recovery-fixture';
    tab.draftId = 'legacy-draft';
    tab.globalSelection = { anchorByte: 0, headByte: 6 };
    tab.globalScrollAnchor = { byteOffset: 3, line: 1 };
    const persisted = await createPersistedWorkspaceState({
      tabs: [tab],
      activeTabId: tab.id,
      currentFolderPath: '',
      desktopEnabled: true,
    });
    expect(persisted.tabs[0]).toMatchObject({
      documentId: 'md-recovery-fixture',
      draftId: 'legacy-draft',
      globalSelection: tab.globalSelection,
      globalScrollAnchor: tab.globalScrollAnchor,
    });
    expect(persisted.tabs[0]).not.toHaveProperty('markdown');
    expect(persisted.tabs[0]).not.toHaveProperty('savedMarkdown');
  });
});
