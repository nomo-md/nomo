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
async function setup(text: string) {
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
  );
  applications.push(application);
  const doc = await application.ready;
  return { application, doc, tab, failures };
}

describe('应用 Markdown 会话入口', () => {
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
