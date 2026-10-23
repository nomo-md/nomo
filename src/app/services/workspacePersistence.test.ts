import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  MarkdownTabState,
  PersistedMarkdownWorkspaceTab,
  PersistedSegmentedWorkspaceTab,
  PersistedWorkspaceState,
} from '../types';
import { createSegmentedTextTab } from './tabs';
import {
  createPersistedWorkspaceState,
  createRuntimeTabFromPersisted,
  migrateWorkspaceSetting,
  pendingCloseWorkspaceSetting,
  restorePendingCloseItems,
  partitionPersistedWorkspaceTabsForRestore,
  persistWorkspaceDrafts,
  WorkspacePersistenceQueue,
  type WorkspaceDraftPersistenceCache,
} from './workspacePersistence';

const writeWorkspaceDraft = vi.hoisted(() => vi.fn());
const deleteWorkspaceDraft = vi.hoisted(() => vi.fn());

vi.mock('../../lib/desktop/tauriStorage', () => ({
  writeWorkspaceDraft,
  deleteWorkspaceDraft,
}));

beforeEach(() => {
  writeWorkspaceDraft.mockReset();
  deleteWorkspaceDraft.mockReset();
  writeWorkspaceDraft.mockImplementation(async (_markdown: string, draftId?: string | null) => ({
    draftId: draftId ?? 'draft-created',
    markdown: _markdown,
    updatedAt: 1,
  }));
});

describe('workspacePersistence', () => {
  it('只合并相邻未执行元数据，关闭屏障前后的请求保持顺序', async () => {
    const queue = new WorkspacePersistenceQueue(), writes: string[] = [];
    let finish!: () => void;
    const running = queue.enqueue(() => new Promise<void>(resolve => { finish = resolve; }));
    await Promise.resolve();
    const old = queue.enqueue(async () => { writes.push('old'); }, true);
    const latest = queue.enqueue(async () => { writes.push('latest'); }, true);
    expect(latest).toBe(old);
    const close = queue.enqueue(async () => { writes.push('close'); });
    const after = queue.enqueue(async () => { writes.push('after'); }, true);
    expect(after).not.toBe(latest);
    finish(); await Promise.all([running, old, latest, close, after]);
    expect(writes).toEqual(['latest', 'close', 'after']);
  });
  it('已开始的写入不合并，错误返回调用方但不阻断之后的写入', async () => {
    const queue = new WorkspacePersistenceQueue(); let fail!: (error: Error) => void;
    const first = queue.enqueue(() => new Promise<void>((_, reject) => { fail = reject; }), true);
    await Promise.resolve();
    const next = vi.fn(async () => {}), second = queue.enqueue(next, true);
    expect(second).not.toBe(first);
    const rejected = expect(first).rejects.toThrow('disk failure');
    fail(Error('disk failure')); await rejected; await second; await queue.settled;
    expect(next).toHaveBeenCalledOnce();
  });
  it('失败后组合回到原外层位置，不移动等待期间新开的标签', () => {
    const tabs = ['a', 'b', 'c', 'new'].map(id => createTab({ id }));
    const item = (tabId: string) => ({ kind: 'single' as const, id: `item-${tabId}`, tabId });
    const comparison = { kind: 'comparison' as const, id: 'pair', leftTabId: 'b', rightTabId: 'c', focusedTabId: 'c', mode: 'semantic' as const, leftPercent: 40 };
    const restored = restorePendingCloseItems(tabs, [item('a'), item('new')], { items: [comparison], itemPositions: [1], tabs: [], positions: [] });
    expect(restored.map(item => item.id)).toEqual(['item-a', 'pair', 'item-new']);
    expect(restored[1]).toEqual(comparison);
  });
  it('恢复当前窗口的关闭检查点，旧启动快照和其它窗口不覆盖它', () => {
    const setting = (key: string, value: unknown) => ({ key, valueJson: JSON.stringify(value), updatedAt: 1 });
    const interrupted = setting('workspaceTabs:main', { version: 5, pendingCloses: [{ id: 'interrupted' }] });
    const settings = [setting('startupWorkspace', { version: 4, tabs: [] }), interrupted,
      setting('workspaceTabs:window-other', { version: 5, pendingCloses: [{ id: 'other' }] })];
    expect(pendingCloseWorkspaceSetting(settings, 'main')).toBe(interrupted);
    expect(pendingCloseWorkspaceSetting(settings, 'window-missing')).toBeUndefined();
    expect(pendingCloseWorkspaceSetting([setting('workspaceTabs:main', { version: 5, pendingCloses: [] })], 'main')).toBeUndefined();
  });
  it('v5 将隐藏关闭记录与可见文档分开持久化，不复制正文，迁移保留恢复身份', async () => {
    const tab = createTab({ id: 'closing', markdown: 'private body', dirty: true });
    tab.documentId = 'md-closing-1';
    const closed = await createPersistedWorkspaceState({ tabs: [tab], activeTabId: tab.id, currentFolderPath: '', desktopEnabled: true, draftWritePolicy: 'skip' });
    const state = await createPersistedWorkspaceState({ tabs: [], activeTabId: '', currentFolderPath: '', desktopEnabled: true,
      pendingCloses: [{ id: 'close-1', phase: 'pending', tabs: closed.tabs as PersistedMarkdownWorkspaceTab[], positions: [2], items: [], decisions: { closing: 'save' }, revisions: { closing: tab.version } }],
    });
    expect(state.tabs).toEqual([]);
    expect(state.pendingCloses?.[0].tabs[0].documentId).toBe('md-closing-1');
    expect(JSON.stringify(state)).not.toContain('private body');
    const restored = await migrateWorkspaceSetting({ valueJson: JSON.stringify(state) });
    expect(restored?.migrated).toBe(false);
    expect(restored?.state.pendingCloses?.[0]).toMatchObject({ phase: 'pending', positions: [2], decisions: { closing: 'save' } });
  });
  it('persists workspace tabs without markdown bodies', async () => {
    const tab = createTab({
      nativePath: 'D:\\docs\\a.md',
      markdown: '# changed',
      savedMarkdown: '# saved',
      dirty: false,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [tab],
      activeTabId: tab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
    });

    expect(JSON.stringify(state)).not.toContain('changed');
    expect(JSON.stringify(state)).not.toContain('saved');
    expect(state).toMatchObject({
      version: 5,
      activeTabId: tab.id,
      currentFolderPath: 'D:\\docs',
      tabs: [{ nativePath: 'D:\\docs\\a.md', draftId: null, dirty: false }],
    });
  });

  it('persists segmented tabs as path and view metadata without Markdown drafts', async () => {
    const tab = createSegmentedTextTab({
      id: 'segmented-json',
      documentKind: 'json',
      fileName: 'large.json',
      filePath: 'D:\\docs\\large.json',
      nativePath: 'D:\\docs\\large.json',
      sessionId: 'session-runtime-only',
      revision: 7,
      persistedRevision: 6,
      indexProgress: 0.5,
      dirty: true,
      selection: { anchorByte: 12, headByte: 20 },
      scrollAnchor: { byteOffset: 1024, line: 41 },
      recoveryConflictPath: 'D:\\recovery\\large-json.journal',
    });

    const state = await createPersistedWorkspaceState({
      tabs: [tab],
      activeTabId: tab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
    });

    expect(writeWorkspaceDraft).not.toHaveBeenCalled();
    expect(deleteWorkspaceDraft).not.toHaveBeenCalled();
    expect(state.tabs[0]).toEqual({
      id: 'segmented-json',
      documentKind: 'json',
      fileName: 'large.json',
      filePath: 'D:\\docs\\large.json',
      nativePath: 'D:\\docs\\large.json',
      dirty: true,
      lastKnownModifiedAt: 0,
      diskReadonly: false,
      selection: { anchorByte: 12, headByte: 20 },
      scrollAnchor: { byteOffset: 1024, line: 41 },
      recoveryConflictPath: 'D:\\recovery\\large-json.journal',
    });
    expect(JSON.stringify(state)).not.toContain('session-runtime-only');
    expect(JSON.stringify(state)).not.toContain('draftId');
    expect(JSON.stringify(state)).not.toContain('markdown');
  });

  it('writes drafts only for dirty or pathless tabs', async () => {
    const dirtyNative = createTab({
      id: 'dirty-native',
      nativePath: 'D:\\docs\\a.md',
      markdown: '# dirty',
      dirty: true,
    });
    const cleanNative = createTab({
      id: 'clean-native',
      nativePath: 'D:\\docs\\b.md',
      markdown: '# clean',
      dirty: false,
    });
    const pathless = createTab({
      id: 'pathless',
      nativePath: null,
      markdown: '# local',
      dirty: false,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [dirtyNative, cleanNative, pathless],
      activeTabId: dirtyNative.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
    });

    expect(writeWorkspaceDraft).toHaveBeenCalledTimes(2);
    expect(getPersistedMarkdownTab(state, 'dirty-native').draftId).toBe('draft-created');
    expect(getPersistedMarkdownTab(state, 'clean-native').draftId).toBeNull();
    expect(getPersistedMarkdownTab(state, 'pathless').draftId).toBe('draft-created');
  });

  it('can ensure missing draft ids without rewriting existing drafts', async () => {
    const existingDirty = createTab({
      id: 'existing-dirty',
      nativePath: 'D:\\docs\\existing.md',
      draftId: 'draft-existing',
      markdown: '# existing',
      dirty: true,
    });
    const missingDirty = createTab({
      id: 'missing-dirty',
      nativePath: 'D:\\docs\\missing.md',
      draftId: null,
      markdown: '# missing',
      dirty: true,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [existingDirty, missingDirty],
      activeTabId: existingDirty.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
      draftWritePolicy: 'missing-only',
    });

    expect(writeWorkspaceDraft).toHaveBeenCalledTimes(1);
    expect(writeWorkspaceDraft).toHaveBeenCalledWith('# missing', null);
    expect(getPersistedMarkdownTab(state, 'existing-dirty').draftId).toBe('draft-existing');
    expect(getPersistedMarkdownTab(state, 'missing-dirty').draftId).toBe('draft-created');
  });

  it('can build workspace metadata without touching draft files', async () => {
    const dirtyTab = createTab({
      id: 'dirty-native',
      draftId: 'draft-existing',
      markdown: '# dirty',
      dirty: true,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [dirtyTab],
      activeTabId: dirtyTab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
      draftWritePolicy: 'skip',
    });

    expect(writeWorkspaceDraft).not.toHaveBeenCalled();
    expect(deleteWorkspaceDraft).not.toHaveBeenCalled();
    expect(getPersistedMarkdownTab(state, 'dirty-native').draftId).toBe('draft-existing');
    expect(JSON.stringify(state)).not.toContain('# dirty');
  });

  it('persists changed drafts once per content signature', async () => {
    const cache: WorkspaceDraftPersistenceCache = new Map();
    const dirtyTab = createTab({
      id: 'dirty-native',
      nativePath: 'D:\\docs\\a.md',
      draftId: null,
      markdown: '# dirty',
      dirty: true,
      version: 1,
    });

    const first = await persistWorkspaceDrafts({
      tabs: [dirtyTab],
      desktopEnabled: true,
      policy: 'changed',
      cache,
    });
    const second = await persistWorkspaceDrafts({
      tabs: [dirtyTab],
      desktopEnabled: true,
      policy: 'changed',
      cache,
    });
    dirtyTab.markdown = '# dirty changed';
    dirtyTab.version = 2;
    const third = await persistWorkspaceDrafts({
      tabs: [dirtyTab],
      desktopEnabled: true,
      policy: 'changed',
      cache,
    });

    expect(first).toEqual({ changed: true, changedDraftIds: true });
    expect(second).toEqual({ changed: false, changedDraftIds: false });
    expect(third).toEqual({ changed: true, changedDraftIds: false });
    expect(writeWorkspaceDraft).toHaveBeenCalledTimes(2);
    expect(writeWorkspaceDraft).toHaveBeenNthCalledWith(1, '# dirty', null);
    expect(writeWorkspaceDraft).toHaveBeenNthCalledWith(2, '# dirty changed', 'draft-created');
  });

  it('persists diskReadonly and defaults old workspace tabs to writable disk state', async () => {
    const readonlyTab = createTab({
      nativePath: 'D:\\docs\\readonly.md',
      diskReadonly: true,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [readonlyTab],
      activeTabId: readonlyTab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
    });

    expect(state.tabs[0].diskReadonly).toBe(true);

    const legacyPersistedTab = {
      ...getPersistedMarkdownTab(state, readonlyTab.id),
    } as Record<string, unknown>;
    delete legacyPersistedTab.diskReadonly;

    const runtimeTab = createRuntimeTabFromPersisted(
      legacyPersistedTab as unknown as PersistedMarkdownWorkspaceTab,
      '# restored',
    );

    expect(runtimeTab.diskReadonly).toBe(false);
  });

  it('preserves pending conflict drafts without rewriting or deleting them', async () => {
    const conflictTab = createTab({
      id: 'conflict-native',
      nativePath: 'D:\\docs\\conflict.md',
      draftId: 'draft-conflict',
      markdown: '# disk',
      savedMarkdown: '# disk',
      dirty: false,
    });

    const state = await createPersistedWorkspaceState({
      tabs: [conflictTab],
      activeTabId: conflictTab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
      preservedDraftIds: new Set(['draft-conflict']),
    });

    expect(writeWorkspaceDraft).not.toHaveBeenCalled();
    expect(deleteWorkspaceDraft).not.toHaveBeenCalled();
    expect(state.tabs[0]).toMatchObject({
      id: 'conflict-native',
      draftId: 'draft-conflict',
      dirty: false,
    });
  });

  it('migrates legacy workspace markdown into drafts only when needed', async () => {
    writeWorkspaceDraft.mockImplementation(async (markdown: string) => ({
      draftId: `draft-${markdown.length}`,
      markdown,
      updatedAt: 1,
    }));

    const result = await migrateWorkspaceSetting({
      valueJson: JSON.stringify({
        tabs: [
          createTab({
            id: 'clean-native',
            nativePath: 'D:\\docs\\clean.md',
            markdown: '# clean',
            dirty: false,
          }),
          createTab({
            id: 'dirty-native',
            nativePath: 'D:\\docs\\dirty.md',
            markdown: '# dirty',
            dirty: true,
          }),
          createTab({
            id: 'pathless',
            nativePath: null,
            markdown: '# local',
            dirty: false,
          }),
        ],
        activeTabId: 'dirty-native',
        currentFolderPath: 'D:\\docs',
      }),
    });

    expect(result?.migrated).toBe(true);
    expect(writeWorkspaceDraft).toHaveBeenCalledTimes(2);
    if (!result) throw new Error('Expected migrated workspace state');
    expect(getPersistedMarkdownTab(result.state, 'clean-native').draftId).toBeNull();
    expect(getPersistedMarkdownTab(result.state, 'dirty-native').draftId).toBe('draft-7');
    expect(getPersistedMarkdownTab(result.state, 'pathless').draftId).toBe('draft-7');
    expect(result?.state.tabs.every((tab) => !('markdown' in tab))).toBe(true);
    expect(JSON.stringify(result?.state)).not.toContain('# clean');
  });

  it('routes version 2 workspace metadata by file extension', async () => {
    const result = await migrateWorkspaceSetting({
      valueJson: JSON.stringify({
        version: 2,
        tabs: [
          {
            id: 'v2-tab',
            fileName: 'legacy.md',
            filePath: 'D:\\docs\\legacy.md',
            nativePath: 'D:\\docs\\legacy.md',
            draftId: 'draft-v2',
            dirty: true,
            lastKnownModifiedAt: 100,
            largeDocumentMode: false,
            readonlyDocumentMode: false,
            diskReadonly: true,
            version: 8,
          },
          {
            id: 'v2-text',
            fileName: 'legacy.txt',
            filePath: 'D:\\docs\\legacy.txt',
            nativePath: 'D:\\docs\\legacy.txt',
            draftId: 'obsolete-text-draft',
            dirty: true,
          },
          {
            id: 'v2-json',
            fileName: 'legacy.json',
            filePath: 'D:\\docs\\legacy.json',
            nativePath: 'D:\\docs\\legacy.json',
            dirty: false,
          },
        ],
        activeTabId: 'v2-tab',
      }),
    });

    expect(result?.migrated).toBe(true);
    expect(result?.state.version).toBe(5);
    expect(result?.state.tabs[0]).toMatchObject({
      id: 'v2-tab',
      documentKind: 'markdown',
      draftId: 'draft-v2',
      diskReadonly: true,
      version: 8,
    });
    expect(result?.state.tabs[1]).toMatchObject({
      id: 'v2-text',
      documentKind: 'text',
      selection: null,
      scrollAnchor: null,
    });
    expect(result?.state.tabs[1]).not.toHaveProperty('draftId');
    expect(result?.state.tabs[2]).toMatchObject({ id: 'v2-json', documentKind: 'json' });
    expect(writeWorkspaceDraft).not.toHaveBeenCalled();
  });

  it('does not recreate legacy TXT or JSON bodies as Markdown drafts', async () => {
    const result = await migrateWorkspaceSetting({
      valueJson: JSON.stringify({
        tabs: [
          {
            id: 'legacy-text',
            fileName: 'large.txt',
            filePath: 'D:\\docs\\large.txt',
            nativePath: 'D:\\docs\\large.txt',
            markdown: 'legacy full text body',
            dirty: true,
          },
          {
            id: 'legacy-json',
            fileName: 'large.json',
            filePath: 'D:\\docs\\large.json',
            nativePath: 'D:\\docs\\large.json',
            markdown: '{"full":"body"}',
            dirty: false,
          },
        ],
        activeTabId: 'legacy-text',
      }),
    });

    expect(result?.state.tabs.map((tab) => tab.documentKind)).toEqual(['text', 'json']);
    expect(JSON.stringify(result?.state)).not.toContain('legacy full text body');
    expect(JSON.stringify(result?.state)).not.toContain('"full"');
    expect(writeWorkspaceDraft).not.toHaveBeenCalled();
  });

  it('corrects stale version 3 document kinds from the native extension', async () => {
    const result = await migrateWorkspaceSetting({
      valueJson: JSON.stringify({
        version: 3,
        tabs: [
          {
            id: 'wrong-text',
            documentKind: 'markdown',
            fileName: 'large.txt',
            filePath: 'D:\\docs\\large.txt',
            nativePath: 'D:\\docs\\large.txt',
            dirty: false,
            lastKnownModifiedAt: 0,
            diskReadonly: false,
          },
          {
            id: 'wrong-markdown',
            documentKind: 'text',
            fileName: 'note.md',
            filePath: 'D:\\docs\\note.md',
            nativePath: 'D:\\docs\\note.md',
            dirty: false,
            lastKnownModifiedAt: 0,
            diskReadonly: false,
          },
        ],
        activeTabId: 'wrong-text',
      }),
    });

    expect(result?.migrated).toBe(true);
    expect(result?.state.tabs.map((tab) => tab.documentKind)).toEqual(['text', 'markdown']);
  });

  it('restores only the active tab on the immediate path', () => {
    const tabs: PersistedWorkspaceState['tabs'] = [
      createPersistedMarkdownTab('first', 'first.md'),
      createPersistedMarkdownTab('second', 'second.md'),
      {
        id: 'active-text',
        documentKind: 'text',
        fileName: 'active.txt',
        filePath: 'D:\\docs\\active.txt',
        nativePath: 'D:\\docs\\active.txt',
        dirty: false,
        lastKnownModifiedAt: 0,
        diskReadonly: false,
        recoveryConflictPath: null,
        selection: null,
        scrollAnchor: null,
      },
    ];

    const partition = partitionPersistedWorkspaceTabsForRestore(tabs, 'active-text');

    expect(partition.immediateTabs.map((tab) => tab.id)).toEqual(['active-text']);
    expect(partition.deferredTabs.map((tab) => tab.id)).toEqual(['first', 'second']);
  });

  it('restores segmented metadata with the newly opened Rust session', async () => {
    const tab = createSegmentedTextTab({
      id: 'segmented-text',
      documentKind: 'text',
      fileName: 'large.txt',
      filePath: 'D:\\docs\\large.txt',
      nativePath: 'D:\\docs\\large.txt',
      sessionId: 'session-before-restart',
      revision: 20,
      persistedRevision: 19,
      indexProgress: 1,
      selection: { anchorByte: 4, headByte: 9 },
      scrollAnchor: { byteOffset: 400, line: 21 },
      recoveryConflictPath: 'D:\\recovery\\before-restart.journal',
    });
    const state = await createPersistedWorkspaceState({
      tabs: [tab],
      activeTabId: tab.id,
      currentFolderPath: 'D:\\docs',
      desktopEnabled: true,
    });

    const runtimeTab = createRuntimeTabFromPersisted(getPersistedSegmentedTab(state, tab.id), {
      sessionId: 'session-after-restart',
      revision: 2,
      persistedRevision: 2,
      indexProgress: 0.1,
      readonly: false,
      recoveryConflictPath: 'D:\\recovery\\after-restart.journal',
    });

    expect(runtimeTab).toMatchObject({
      documentKind: 'text',
      sessionId: 'session-after-restart',
      revision: 2,
      persistedRevision: 2,
      indexProgress: 0.1,
      selection: { anchorByte: 4, headByte: 9 },
      scrollAnchor: { byteOffset: 400, line: 21 },
      recoveryConflictPath: 'D:\\recovery\\after-restart.journal',
    });
    expect('markdown' in runtimeTab).toBe(false);
  });

  it('normalizes old version 3 segmented metadata without a recovery conflict path to null', async () => {
    const result = await migrateWorkspaceSetting({
      valueJson: JSON.stringify({
        version: 3,
        tabs: [
          {
            id: 'old-v3-segmented',
            documentKind: 'text',
            fileName: 'old.txt',
            filePath: 'D:\\docs\\old.txt',
            nativePath: 'D:\\docs\\old.txt',
            dirty: false,
            lastKnownModifiedAt: 0,
            diskReadonly: false,
            selection: null,
            scrollAnchor: null,
          },
        ],
        activeTabId: 'old-v3-segmented',
      }),
    });

    expect(result?.migrated).toBe(true);
    expect(result?.state.tabs[0]).toMatchObject({
      documentKind: 'text',
      recoveryConflictPath: null,
    });
  });
});

function getPersistedMarkdownTab(
  state: PersistedWorkspaceState,
  id: string,
): PersistedMarkdownWorkspaceTab {
  const tab = state.tabs.find((candidate) => candidate.id === id);
  if (!tab || tab.documentKind !== 'markdown') {
    throw new Error(`Expected persisted Markdown tab: ${id}`);
  }
  return tab;
}

function getPersistedSegmentedTab(
  state: PersistedWorkspaceState,
  id: string,
): PersistedSegmentedWorkspaceTab {
  const tab = state.tabs.find((candidate) => candidate.id === id);
  if (!tab || tab.documentKind === 'markdown') {
    throw new Error(`Expected persisted segmented tab: ${id}`);
  }
  return tab;
}

function createPersistedMarkdownTab(id: string, fileName: string): PersistedMarkdownWorkspaceTab {
  const nativePath = `D:\\docs\\${fileName}`;
  return {
    id,
    documentKind: 'markdown',
    fileName,
    filePath: nativePath,
    nativePath,
    draftId: null,
    dirty: false,
    lastKnownModifiedAt: 0,
    largeDocumentMode: false,
    readonlyDocumentMode: false,
    diskReadonly: false,
    version: 0,
  };
}

function createTab(overrides: Partial<MarkdownTabState> = {}): MarkdownTabState {
  return {
    id: 'tab-1',
    documentKind: 'markdown',
    fileName: 'a.md',
    filePath: 'D:\\docs\\a.md',
    nativePath: 'D:\\docs\\a.md',
    draftId: null,
    markdown: '# a',
    savedMarkdown: '# a',
    dirty: false,
    lastKnownModifiedAt: 100,
    largeDocumentMode: false,
    readonlyDocumentMode: false,
    diskReadonly: false,
    externalFileChange: {
      type: 'none',
      path: null,
      modifiedAt: 0,
      dirtyAtDetection: false,
      message: '',
    },
    version: 1,
    ...overrides,
  };
}
