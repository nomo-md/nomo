import { describe, expect, it, vi, afterEach } from 'vitest';
import type { NativeDocument } from '../../lib/desktop/tauriStorage';
import type { MarkdownTabState, Tab } from '../types';
import { createEmptyExternalFileChange } from '../types';
import { createDocumentActionsController } from './documentActionsController';
import { confirmAction } from './confirmAction';
import { saveNativeMarkdownFile } from './documentFiles';
import { createMarkdownTab, createSegmentedTextTab, isMarkdownTab } from './tabs';

vi.mock('./confirmAction', () => ({
  confirmAction: vi.fn(),
}));

vi.mock('./documentFiles', () => ({
  exportMarkdownInBrowser: vi.fn(),
  findDroppedMarkdownPath: vi.fn(),
  getExternalFileChange: vi.fn().mockResolvedValue({
    type: 'none',
    path: null,
    modifiedAt: 0,
    dirtyAtDetection: false,
    message: '',
  }),
  loadRecentEntries: vi.fn().mockResolvedValue([]),
  openMarkdownFromDialog: vi.fn(),
  readMarkdownFromPath: vi.fn(),
  rememberNativeDocument: vi.fn(),
  saveNativeMarkdownFile: vi.fn(),
}));

function createTab(
  overrides: Omit<Partial<MarkdownTabState>, 'documentKind'> = {},
): MarkdownTabState {
  return createMarkdownTab({
    id: 'tab-1',
    fileName: 'old.md',
    filePath: 'C:/docs/old.md',
    nativePath: 'C:/docs/old.md',
    draftId: null,
    markdown: '# Old\n\nchanged',
    savedMarkdown: '# Old',
    dirty: true,
    lastKnownModifiedAt: 1,
    largeDocumentMode: false,
    readonlyDocumentMode: false,
    diskReadonly: false,
    externalFileChange: createEmptyExternalFileChange(),
    version: 1,
    ...overrides,
  });
}

function createOptions(initialTabs: Tab[]) {
  let tabs = initialTabs;
  const initialMarkdownTab = isMarkdownTab(tabs[0]) ? tabs[0] : null;
  let activeTabId = tabs[0]?.id ?? '';
  let statusMessage = '';
  let nativePath = tabs[0]?.nativePath ?? null;
  let fileName = tabs[0]?.fileName ?? 'old.md';
  let filePath = tabs[0]?.filePath ?? 'C:/docs/old.md';
  let savedMarkdown = initialMarkdownTab?.savedMarkdown ?? '';
  let lastKnownModifiedAt = tabs[0]?.lastKnownModifiedAt ?? 0;
  let largeDocumentMode = initialMarkdownTab?.largeDocumentMode ?? false;
  let readonlyDocumentMode = initialMarkdownTab?.readonlyDocumentMode ?? false;
  let diskReadonly = tabs[0]?.diskReadonly ?? false;
  let dirty = tabs[0]?.dirty ?? false;
  const getMarkdown = vi.fn(() => '# New Title\n\nchanged');

  return {
    options: {
      recoveryKey: 'test-recovery',
      getLargeDocumentLimit: () => 100_000,
      getAutoSaveDelayMs: () => 50,
      getCreateSnapshotBeforeSave: () => false,
      getDesktopEnabled: () => true,
      getDirty: () => dirty,
      getAutoSaveEnabled: () => true,
      getNativePath: () => nativePath,
      setMarkdown: vi.fn(),
      setSavedMarkdown: (value: string) => {
        savedMarkdown = value;
      },
      setNativePath: (value: string | null) => {
        nativePath = value;
      },
      getFileName: () => fileName,
      setFileName: (value: string) => {
        fileName = value;
      },
      getFilePath: () => filePath,
      setFilePath: (value: string) => {
        filePath = value;
      },
      getLastKnownModifiedAt: () => lastKnownModifiedAt,
      setLastKnownModifiedAt: (value: number) => {
        lastKnownModifiedAt = value;
      },
      getExternalFileChange: () => createEmptyExternalFileChange(),
      setExternalFileChange: vi.fn(),
      setDirty: (value: boolean) => {
        dirty = value;
      },
      setLargeDocumentMode: (value: boolean) => {
        largeDocumentMode = value;
      },
      setReadonlyDocumentMode: (value: boolean) => {
        readonlyDocumentMode = value;
      },
      setDiskReadonly: (value: boolean) => {
        diskReadonly = value;
      },
      getCurrentFolderPath: () => '',
      getFileInput: () => document.createElement('input'),
      getEditor: () => ({
        getMarkdown,
        flushMarkdown: getMarkdown,
        setDirty: vi.fn(),
      }),
      getTabs: () => tabs,
      setTabs: (value: Tab[]) => {
        tabs = value;
      },
      getActiveTabId: () => activeTabId,
      setActiveTabId: (value: string) => {
        activeTabId = value;
      },
      getPreviewTabId: () => null,
      setPreviewTabId: vi.fn(),
      setStatusMessage: (value: string) => {
        statusMessage = value;
      },
      setRecentFiles: vi.fn(),
      saveActiveTabState: vi.fn(),
      loadTabState: (tab: Tab) => {
        if (!isMarkdownTab(tab)) return;
        activeTabId = tab.id;
        nativePath = tab.nativePath;
        fileName = tab.fileName;
        filePath = tab.filePath;
        savedMarkdown = tab.savedMarkdown;
        lastKnownModifiedAt = tab.lastKnownModifiedAt;
        largeDocumentMode = tab.largeDocumentMode;
        readonlyDocumentMode = tab.readonlyDocumentMode;
        diskReadonly = tab.diskReadonly;
        dirty = tab.dirty;
      },
      switchTab: vi.fn(),
      writeRecoveryDraft: vi.fn(),
      updateWindowTitle: vi.fn(),
      loadFolder: vi.fn().mockResolvedValue(undefined),
      expandAncestors: vi.fn(),
    },
    getState: () => ({
      tabs,
      activeTabId,
      fileName,
      filePath,
      nativePath,
      savedMarkdown,
      lastKnownModifiedAt,
      largeDocumentMode,
      readonlyDocumentMode,
      diskReadonly,
      dirty,
      statusMessage,
    }),
    getMarkdown,
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  localStorage.clear();
});

describe('documentActionsController', () => {
  it('打开同目录新文件增加标签，保留旧标签内容和脏状态，重复打开不增加标签', async () => {
    const oldTab = createTab();
    const before = { ...oldTab };
    const { options, getState } = createOptions([oldTab]);
    const controller = createDocumentActionsController(options as any);
    const document: NativeDocument = {
      path: 'C:/docs/second.md',
      fileName: 'second.md',
      markdown: '# Second',
      modifiedAt: 1,
      sizeBytes: 8,
      readonly: false,
    };
    await controller.applyNativeDocument(document, 'opened');
    expect(getState().tabs).toHaveLength(2);
    expect(getState().tabs[0]).toEqual(before);
    expect(getState().tabs[0].dirty).toBe(true);
    expect(getState().tabs[1].nativePath).toBe(document.path);
    expect(getState().activeTabId).toBe(getState().tabs[1].id);
    await controller.applyNativeDocument(document, 'opened again');
    expect(getState().tabs).toHaveLength(2);
    expect(getState().tabs[0]).toEqual(before);
  });

  it('关闭脏标签时，如果保存失败则保留标签页', async () => {
    const tab = createTab();
    const { options, getState } = createOptions([tab]);
    vi.mocked(confirmAction).mockResolvedValue('save');
    vi.mocked(saveNativeMarkdownFile).mockResolvedValue({
      document: null,
      error: 'disk full',
    });

    const controller = createDocumentActionsController(options as any);
    await controller.closeTab(tab.id);

    expect(getState().tabs).toHaveLength(1);
    expect(getState().tabs[0].id).toBe(tab.id);
    expect(getState().statusMessage).toBe('disk full');
  });

  it('自动保存只写回当前路径，不根据 H1 隐式重命名文件', async () => {
    vi.useFakeTimers();
    const tab = createTab({ markdown: '# New Title\n\nchanged' });
    const { options, getState } = createOptions([tab]);
    const document: NativeDocument = {
      path: 'C:/docs/old.md',
      fileName: 'old.md',
      markdown: '# New Title\n\nchanged',
      modifiedAt: 2,
      sizeBytes: 21,
      readonly: false,
    };
    vi.mocked(saveNativeMarkdownFile).mockResolvedValue({ document, error: '' });

    const controller = createDocumentActionsController(options as any);
    controller.debouncedAutoSave(tab.id);
    await vi.advanceTimersByTimeAsync(50);

    expect(saveNativeMarkdownFile).toHaveBeenCalledWith(
      'C:/docs/old.md',
      '# New Title\n\nchanged\n\n',
      'old.md',
      null,
    );
    expect(getState().tabs[0].nativePath).toBe('C:/docs/old.md');
    expect(getState().tabs[0].fileName).toBe('old.md');
    expect(getState().nativePath).toBe('C:/docs/old.md');
    expect(getState().fileName).toBe('old.md');
  });

  it('标签恢复保存基线后取消尚未执行的自动保存', async () => {
    vi.useFakeTimers();
    const tab = createTab();
    const { options } = createOptions([tab]);
    const controller = createDocumentActionsController(options as any);

    controller.debouncedAutoSave(tab.id);
    tab.markdown = tab.savedMarkdown;
    tab.dirty = false;
    controller.cancelPendingAutoSave(tab.id);
    await vi.advanceTimersByTimeAsync(50);

    expect(saveNativeMarkdownFile).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('打开小型只读磁盘文件时保持编辑器可编辑并记录磁盘只读', async () => {
    const tab = createTab({
      dirty: false,
      nativePath: null,
      fileName: 'untitled.md',
      filePath: '未命名 Markdown',
    });
    const { options, getState } = createOptions([tab]);
    const document: NativeDocument = {
      path: 'C:/docs/readonly.md',
      fileName: 'readonly.md',
      markdown: '# Readonly\n\ncontent',
      modifiedAt: 2,
      sizeBytes: 20,
      readonly: true,
    };

    const controller = createDocumentActionsController(options as any);
    await controller.applyNativeDocument(document, 'opened');

    const activeTab = getState().tabs.find((tab) => tab.id === getState().activeTabId);
    expect(activeTab).toMatchObject({
      nativePath: 'C:/docs/readonly.md',
      diskReadonly: true,
      largeDocumentMode: false,
      readonlyDocumentMode: false,
    });
    expect(getState().diskReadonly).toBe(true);
    expect(getState().readonlyDocumentMode).toBe(false);
  });

  it('普通保存只读源文件时走另存为，不写回原路径', async () => {
    const tab = createTab({ diskReadonly: true });
    const { options, getState } = createOptions([tab]);
    const document: NativeDocument = {
      path: 'C:/docs/new.md',
      fileName: 'new.md',
      markdown: '# New Title\n\nchanged\n\n',
      modifiedAt: 2,
      sizeBytes: 23,
      readonly: false,
    };
    vi.mocked(saveNativeMarkdownFile).mockResolvedValue({ document, error: '' });

    const controller = createDocumentActionsController(options as any);
    const saved = await controller.saveMarkdownFile(false);

    expect(saved).toBe(true);
    expect(saveNativeMarkdownFile).toHaveBeenCalledWith(
      null,
      '# New Title\n\nchanged\n\n',
      'New Title.md',
      null,
    );
    expect(getState().nativePath).toBe('C:/docs/new.md');
    expect(getState().fileName).toBe('new.md');
    expect(getState().dirty).toBe(false);
    expect(getState().diskReadonly).toBe(false);
    expect(getState().tabs[0].diskReadonly).toBe(false);
  });

  it('只读源文件另存为取消后保留 dirty、原路径和 diskReadonly', async () => {
    const tab = createTab({ diskReadonly: true });
    const { options, getState } = createOptions([tab]);
    vi.mocked(saveNativeMarkdownFile).mockResolvedValue({ document: null, error: '' });

    const controller = createDocumentActionsController(options as any);
    const saved = await controller.saveMarkdownFile(false);

    expect(saved).toBe(false);
    expect(saveNativeMarkdownFile).toHaveBeenCalledWith(
      null,
      '# New Title\n\nchanged\n\n',
      'New Title.md',
      null,
    );
    expect(getState().nativePath).toBe('C:/docs/old.md');
    expect(getState().fileName).toBe('old.md');
    expect(getState().dirty).toBe(true);
    expect(getState().diskReadonly).toBe(true);
    expect(getState().tabs[0].diskReadonly).toBe(true);
  });

  it('自动保存只读源文件时不写磁盘且保持 dirty', async () => {
    vi.useFakeTimers();
    const tab = createTab({ diskReadonly: true });
    const { options, getState } = createOptions([tab]);

    const controller = createDocumentActionsController(options as any);
    controller.debouncedAutoSave(tab.id);
    await vi.advanceTimersByTimeAsync(50);

    expect(saveNativeMarkdownFile).not.toHaveBeenCalled();
    expect(getState().dirty).toBe(true);
    expect(getState().tabs[0].dirty).toBe(true);
    expect(getState().statusMessage).toBe(
      'Auto-save is paused because the source file is read-only. Use Save as to keep the current content.',
    );
  });

  it('segmented 标签误入手动保存时不读取或保存 Markdown', async () => {
    const tab = createSegmentedTextTab({
      id: 'segmented-json',
      documentKind: 'json',
      fileName: 'large.json',
      filePath: 'C:/docs/large.json',
      nativePath: 'C:/docs/large.json',
      sessionId: 'session-1',
      revision: 2,
      persistedRevision: 1,
      indexProgress: 1,
    });
    const { options, getMarkdown } = createOptions([tab]);
    const controller = createDocumentActionsController(options as any);

    await expect(controller.saveMarkdownFile(false)).resolves.toBe(false);

    expect(getMarkdown).not.toHaveBeenCalled();
    expect(saveNativeMarkdownFile).not.toHaveBeenCalled();
  });

  it('浏览器模式拒绝 TXT/JSON 且不调用 File.text 全量读取', async () => {
    const tab = createTab({ dirty: false });
    const { options } = createOptions([tab]);
    const text = vi.fn().mockResolvedValue('must-not-enter-webview');
    const input = document.createElement('input');
    Object.defineProperty(input, 'files', {
      configurable: true,
      value: [{ name: 'large.json', text }],
    });
    const controller = createDocumentActionsController(options as any);

    await controller.openMarkdownFile({ currentTarget: input } as unknown as Event);

    expect(text).not.toHaveBeenCalled();
  });

  it('segmented 标签误入自动保存时不调度 Markdown 保存', async () => {
    vi.useFakeTimers();
    const tab = createSegmentedTextTab({
      id: 'segmented-text',
      documentKind: 'text',
      fileName: 'large.txt',
      filePath: 'C:/docs/large.txt',
      nativePath: 'C:/docs/large.txt',
      sessionId: 'session-1',
      revision: 2,
      persistedRevision: 1,
      indexProgress: 1,
    });
    const { options } = createOptions([tab]);
    const controller = createDocumentActionsController(options as any);

    controller.debouncedAutoSave(tab.id);
    await vi.advanceTimersByTimeAsync(100);

    expect(saveNativeMarkdownFile).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('自动保存只以标签最新内容收敛且不把在途旧内容标记为已保存', async () => {
    vi.useFakeTimers();
    const originalMarkdown = '# Old\n\n';
    const editedMarkdown = '# Old\n\nchanged';
    const tab = createTab({
      markdown: editedMarkdown,
      savedMarkdown: originalMarkdown,
      dirty: true,
    });
    const { options, getState } = createOptions([tab]);
    let resolveFirstSave!: (value: { document: NativeDocument | null; error: string }) => void;
    const firstSave = new Promise<{ document: NativeDocument | null; error: string }>((resolve) => {
      resolveFirstSave = resolve;
    });
    vi.mocked(saveNativeMarkdownFile)
      .mockImplementationOnce(() => firstSave)
      .mockResolvedValueOnce({
        document: {
          path: tab.nativePath!,
          fileName: tab.fileName,
          markdown: originalMarkdown,
          modifiedAt: 3,
          sizeBytes: originalMarkdown.length,
          readonly: false,
        },
        error: '',
      });

    const controller = createDocumentActionsController(options as any);
    controller.debouncedAutoSave(tab.id);
    vi.advanceTimersByTime(50);
    await Promise.resolve();
    expect(saveNativeMarkdownFile).toHaveBeenCalledTimes(1);

    tab.markdown = originalMarkdown;
    tab.dirty = false;
    controller.cancelPendingAutoSave(tab.id);
    resolveFirstSave({
      document: {
        path: tab.nativePath!,
        fileName: tab.fileName,
        markdown: `${editedMarkdown}\n\n`,
        modifiedAt: 2,
        sizeBytes: editedMarkdown.length + 2,
        readonly: false,
      },
      error: '',
    });
    for (let index = 0; index < 5; index += 1) {
      await Promise.resolve();
    }

    expect(saveNativeMarkdownFile).toHaveBeenNthCalledWith(
      1,
      tab.nativePath,
      `${editedMarkdown}\n\n`,
      tab.fileName,
      null,
    );
    expect(saveNativeMarkdownFile).toHaveBeenNthCalledWith(
      2,
      tab.nativePath,
      originalMarkdown,
      tab.fileName,
      null,
    );
    expect(getState().tabs[0]).toMatchObject({
      markdown: originalMarkdown,
      savedMarkdown: originalMarkdown,
      dirty: false,
      lastKnownModifiedAt: 3,
    });
  });

  it('segmented 标签关闭不由 Markdown controller 处理', async () => {
    const tab = createSegmentedTextTab({
      id: 'segmented-text',
      documentKind: 'text',
      fileName: 'large.txt',
      filePath: 'C:/docs/large.txt',
      nativePath: 'C:/docs/large.txt',
      sessionId: 'session-1',
      revision: 2,
      persistedRevision: 1,
      indexProgress: 1,
    });
    const { options, getState } = createOptions([tab]);
    const controller = createDocumentActionsController(options as any);

    await controller.closeTab(tab.id);

    expect(getState().tabs).toEqual([tab]);
    expect(confirmAction).not.toHaveBeenCalled();
    expect(saveNativeMarkdownFile).not.toHaveBeenCalled();
  });
});
