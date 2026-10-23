import {
  deleteWorkspaceDraft,
  writeWorkspaceDraft,
  type SettingRecord,
} from '../../lib/desktop/tauriStorage';
import {
  createEmptyExternalFileChange,
  normalizeRecoveryConflictPath,
  type ExternalFileChangeState,
  type GlobalScrollAnchor,
  type GlobalSelection,
  type MarkdownTabState,
  type PersistedMarkdownWorkspaceTab,
  type PersistedSegmentedWorkspaceTab,
  type PersistedWorkspaceState,
  type PersistedPendingClose,
  type PersistedWorkspaceTab,
  type SegmentedSessionOpenData,
  type SegmentedTextTabState,
  type Tab,
  type WorkspaceItem,
} from '../types';
import { getDocumentKindFromPath, isMarkdownTab, isSegmentedTextTab } from './tabs';
import {
  findWorkspaceItemForTab,
  getWorkspaceItemFocusedTabId,
  getWorkspaceItemTabIds,
  normalizeWorkspaceItems,
} from './workspaceItems';
import {
  DEFAULT_MARKDOWN_ENCODING,
  normalizeMarkdownEncoding,
  type MarkdownEncoding,
} from '../../lib/services/storage';

interface PersistedWorkspaceTabV2 {
  closeFailure?: string;
  documentId?: string;
  globalSelection?: GlobalSelection | null;
  globalScrollAnchor?: GlobalScrollAnchor | null;
  id?: string;
  fileName?: string;
  filePath?: string;
  nativePath?: string | null;
  draftId?: string | null;
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  largeDocumentMode?: boolean;
  readonlyDocumentMode?: boolean;
  diskReadonly?: boolean;
  version?: number;
  encoding?: MarkdownEncoding;
}

interface PersistedWorkspaceStateV2 {
  version: 2;
  tabs: PersistedWorkspaceTabV2[];
  activeTabId: string;
  currentFolderPath?: string;
}

type LegacyWorkspaceTab = PersistedWorkspaceTabV2 & {
  markdown?: string;
  savedMarkdown?: string;
};

type LegacyWorkspaceState = {
  tabs?: LegacyWorkspaceTab[];
  activeTabId?: string;
  currentFolderPath?: string;
};

export interface WorkspaceMigrationResult {
  state: PersistedWorkspaceState;
  migrated: boolean;
}

/** 未完成关闭属于当前窗口；不能被上次正常退出留下的启动快照遮蔽。 */
export function pendingCloseWorkspaceSetting(settings: readonly SettingRecord[], windowLabel: string) {
  const setting = settings.find(setting => setting.key === `workspaceTabs:${windowLabel}`);
  if (!setting) return undefined;
  try {
    const value = JSON.parse(setting.valueJson);
    return Array.isArray(value?.pendingCloses) && value.pendingCloses.length ? setting : undefined;
  } catch { return undefined; }
}

/** 恢复外层标签的位置和组合关系，同时保留用户在等待期间打开的其它标签。 */
export function restorePendingCloseItems(tabs: readonly Tab[] | readonly PersistedWorkspaceTab[], current: readonly WorkspaceItem[], record: Pick<PersistedPendingClose, 'items' | 'itemPositions' | 'tabs' | 'positions'>) {
  const restoring = new Set(record.items.map(item => item.id));
  const items = current.filter(item => !restoring.has(item.id));
  const positioned = record.items.map((item, index) => ({ item, position: record.itemPositions?.[index] ?? record.positions[record.tabs.findIndex(tab => tab.id === getWorkspaceItemTabIds(item)[0])] ?? items.length }));
  for (const { item, position } of positioned.sort((a, b) => a.position - b.position))
    items.splice(Math.max(0, Math.min(position, items.length)), 0, item);
  return normalizeWorkspaceItems(tabs, items);
}

export type WorkspaceDraftWritePolicy = 'write-needed' | 'missing-only' | 'skip';
export type WorkspaceDraftPersistPolicy = 'changed' | 'missing-only';

export interface WorkspaceDraftPersistenceCacheEntry {
  draftId: string;
  signature: string;
}

export type WorkspaceDraftPersistenceCache = Map<string, WorkspaceDraftPersistenceCacheEntry>;

export interface WorkspaceDraftPersistenceResult {
  changed: boolean;
  changedDraftIds: boolean;
}

export interface MarkdownRuntimeTabOptions {
  savedMarkdown?: string;
  encoding?: MarkdownEncoding;
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  largeDocumentMode?: boolean;
  readonlyDocumentMode?: boolean;
  diskReadonly?: boolean;
  externalFileChange?: ExternalFileChangeState;
}

export interface SegmentedRuntimeTabOptions {
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  diskReadonly?: boolean;
  externalFileChange?: ExternalFileChangeState;
}

/** 首屏恢复活动项全部成员；其余文档延迟，避免组合的一侧首屏显示为空。 */
export function partitionPersistedWorkspaceTabsForRestore(
  tabs: PersistedWorkspaceTab[],
  activeTabId: string,
  items?: readonly WorkspaceItem[],
  activeItemId?: string,
) {
  const normalizedItems = normalizeWorkspaceItems(tabs, items);
  const activeItem = normalizedItems.find((item) => item.id === activeItemId) ??
    findWorkspaceItemForTab(normalizedItems, activeTabId) ?? normalizedItems[0];
  if (!activeItem) return { immediateTabs: [], deferredTabs: [] };
  const activeTabIds = getWorkspaceItemTabIds(activeItem);
  const immediateTabs = activeTabIds.flatMap((tabId) => {
    const tab = tabs.find((candidate) => candidate.id === tabId);
    return tab ? [tab] : [];
  });
  return {
    immediateTabs,
    deferredTabs: tabs.filter((tab) => !activeTabIds.includes(tab.id)),
  };
}

function isPersistedWorkspaceState(value: unknown): value is PersistedWorkspaceState {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const state = value as Partial<PersistedWorkspaceState>;
  return (
    (state.version === 3 || state.version === 4 || state.version === 5) &&
    Array.isArray(state.tabs) &&
    state.tabs.every(isPersistedWorkspaceTab) &&
    typeof state.activeTabId === 'string'
  );
}

export async function createPersistedWorkspaceState(input: {
  tabs: Tab[];
  activeTabId: string;
  items?: readonly WorkspaceItem[];
  activeItemId?: string;
  currentFolderPath: string;
  desktopEnabled: boolean;
  preservedDraftIds?: ReadonlySet<string>;
  draftWritePolicy?: WorkspaceDraftWritePolicy;
  pendingCloses?: PersistedPendingClose[];
}): Promise<PersistedWorkspaceState> {
  const persistedTabs: PersistedWorkspaceTab[] = [];
  const draftWritePolicy = input.draftWritePolicy ?? 'write-needed';

  for (const tab of input.tabs) {
    if (isSegmentedTextTab(tab)) {
      // 分段正文和恢复日志归 Rust 会话所有，工作区元数据绝不创建 Markdown draft。
      persistedTabs.push(toPersistedSegmentedWorkspaceTab(tab));
      continue;
    }
    if (tab.documentId) {
      persistedTabs.push(toPersistedMarkdownWorkspaceTab(tab, tab.draftId));
      continue;
    }

    let draftId = tab.draftId;
    if (draftId && input.preservedDraftIds?.has(draftId)) {
      persistedTabs.push(toPersistedMarkdownWorkspaceTab(tab, draftId));
      continue;
    }

    if (input.desktopEnabled && shouldPersistDraft(tab)) {
      const shouldWriteDraft =
        draftWritePolicy === 'write-needed' || (draftWritePolicy === 'missing-only' && !draftId);
      if (shouldWriteDraft) {
        const draft = await writeWorkspaceDraft(tab.markdown, draftId);
        draftId = draft.draftId;
        tab.draftId = draftId;
      }
    } else if (input.desktopEnabled && draftId && draftWritePolicy !== 'skip') {
      await deleteWorkspaceDraft(draftId).catch(() => undefined);
      draftId = null;
      tab.draftId = null;
    }

    persistedTabs.push(toPersistedMarkdownWorkspaceTab(tab, draftId));
  }

  return normalizePersistedWorkspaceState({
    version: 5,
    tabs: persistedTabs,
    activeTabId: input.activeTabId,
    items: input.items ? [...input.items] : undefined,
    activeItemId: input.activeItemId,
    currentFolderPath: input.currentFolderPath || undefined,
    pendingCloses: input.pendingCloses,
  });
}

export async function persistWorkspaceDrafts(input: {
  tabs: Tab[];
  desktopEnabled: boolean;
  policy: WorkspaceDraftPersistPolicy;
  preservedDraftIds?: ReadonlySet<string>;
  cache?: WorkspaceDraftPersistenceCache;
}): Promise<WorkspaceDraftPersistenceResult> {
  const result: WorkspaceDraftPersistenceResult = {
    changed: false,
    changedDraftIds: false,
  };

  if (!input.desktopEnabled) {
    return result;
  }

  for (const tab of input.tabs) {
    if (!isMarkdownTab(tab) || tab.documentId) {
      continue;
    }

    let draftId = tab.draftId;
    if (draftId && input.preservedDraftIds?.has(draftId)) {
      continue;
    }

    if (shouldPersistDraft(tab)) {
      const signature = createWorkspaceDraftSignature(tab);
      const cached = input.cache?.get(tab.id);
      const hasCurrentDraft =
        Boolean(draftId) && cached?.draftId === draftId && cached.signature === signature;
      const shouldWrite = !draftId || (input.policy === 'changed' && !hasCurrentDraft);

      if (!shouldWrite) {
        continue;
      }

      const previousDraftId = draftId;
      const draft = await writeWorkspaceDraft(tab.markdown, draftId);
      draftId = draft.draftId;
      tab.draftId = draftId;
      input.cache?.set(tab.id, { draftId, signature });
      result.changed = true;
      result.changedDraftIds = result.changedDraftIds || previousDraftId !== draftId;
      continue;
    }

    if (draftId) {
      await deleteWorkspaceDraft(draftId).catch(() => undefined);
      tab.draftId = null;
      input.cache?.delete(tab.id);
      result.changed = true;
      result.changedDraftIds = true;
    }
  }

  return result;
}

export async function migrateWorkspaceSetting(
  setting: Pick<SettingRecord, 'valueJson'>,
): Promise<WorkspaceMigrationResult | null> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(setting.valueJson);
  } catch {
    return null;
  }

  if (isPersistedWorkspaceState(parsed)) {
    const correctedKind = parsed.tabs.some(
      (tab) => resolvePersistedDocumentKind(tab) !== tab.documentKind,
    );
    const state = normalizePersistedWorkspaceState(parsed);
    const correctedItems = JSON.stringify(parsed.items) !== JSON.stringify(state.items) ||
      parsed.activeItemId !== state.activeItemId || parsed.activeTabId !== state.activeTabId;
    return {
      state,
      migrated: parsed.version !== 5 || correctedKind || correctedItems,
    };
  }

  if (isPersistedWorkspaceStateV2(parsed)) {
    return { state: migratePersistedWorkspaceStateV2(parsed), migrated: true };
  }

  const legacy = parsed as LegacyWorkspaceState;
  if (!Array.isArray(legacy.tabs)) {
    return null;
  }

  const tabs: PersistedWorkspaceTab[] = [];
  for (const legacyTab of legacy.tabs) {
    const markdown = typeof legacyTab.markdown === 'string' ? legacyTab.markdown : '';
    const dirty = Boolean(legacyTab.dirty);
    const nativePath = typeof legacyTab.nativePath === 'string' ? legacyTab.nativePath : null;
    const documentKind = resolvePersistedDocumentKind({ ...legacyTab, nativePath });
    if (documentKind !== 'markdown') {
      // 旧版曾把 TXT 当作 Markdown 正文；迁移后必须重新由 Rust 按路径打开，不能把全量正文带回 WebView。
      tabs.push(
        normalizePersistedSegmentedWorkspaceTab({
          ...legacyTab,
          nativePath,
          documentKind,
          recoveryConflictPath: null,
          selection: null,
          scrollAnchor: null,
        }),
      );
      continue;
    }
    let draftId = typeof legacyTab.draftId === 'string' ? legacyTab.draftId : null;

    if (dirty || (!nativePath && markdown.length > 0)) {
      const draft = await writeWorkspaceDraft(markdown, draftId);
      draftId = draft.draftId;
    } else {
      draftId = null;
    }

    // 无版本旧数据的正文来自 Markdown 全量模型；先保真迁移，不能按扩展名静默丢弃旧 draft。
    tabs.push(normalizePersistedMarkdownWorkspaceTab({ ...legacyTab, nativePath, draftId, dirty }));
  }

  return {
    migrated: true,
    state: normalizePersistedWorkspaceState({
      version: 4,
      tabs,
      activeTabId:
        typeof legacy.activeTabId === 'string' ? legacy.activeTabId : (tabs[0]?.id ?? ''),
      currentFolderPath:
        typeof legacy.currentFolderPath === 'string' ? legacy.currentFolderPath : undefined,
    }),
  };
}

export function createRuntimeTabFromPersisted(
  tab: PersistedMarkdownWorkspaceTab,
  markdown: string,
  options?: MarkdownRuntimeTabOptions,
): MarkdownTabState;
export function createRuntimeTabFromPersisted(
  tab: PersistedSegmentedWorkspaceTab,
  session: SegmentedSessionOpenData,
  options?: SegmentedRuntimeTabOptions,
): SegmentedTextTabState;
export function createRuntimeTabFromPersisted(
  tab: PersistedWorkspaceTab,
  contentOrSession: string | SegmentedSessionOpenData,
  options: MarkdownRuntimeTabOptions | SegmentedRuntimeTabOptions = {},
): Tab {
  const externalFileChange = options.externalFileChange ?? createEmptyExternalFileChange();

  if (tab.documentKind === 'markdown') {
    if (typeof contentOrSession !== 'string') {
      throw new Error('Markdown workspace restore requires Markdown content');
    }
    const markdownOptions = options as MarkdownRuntimeTabOptions;
    return {
      id: tab.id,
      documentKind: 'markdown',
      documentId: tab.documentId,
      closeFailure: tab.closeFailure,
      globalSelection: tab.globalSelection,
      globalScrollAnchor: tab.globalScrollAnchor,
      fileName: tab.fileName,
      filePath: tab.filePath,
      nativePath: tab.nativePath,
      draftId: tab.draftId,
      markdown: contentOrSession,
      savedMarkdown: markdownOptions.savedMarkdown ?? contentOrSession,
      encoding: normalizeMarkdownEncoding(markdownOptions.encoding ?? tab.encoding),
      dirty: markdownOptions.dirty ?? tab.dirty,
      lastKnownModifiedAt: markdownOptions.lastKnownModifiedAt ?? tab.lastKnownModifiedAt,
      largeDocumentMode: markdownOptions.largeDocumentMode ?? tab.largeDocumentMode,
      readonlyDocumentMode: markdownOptions.readonlyDocumentMode ?? tab.readonlyDocumentMode,
      diskReadonly: markdownOptions.diskReadonly ?? tab.diskReadonly ?? false,
      externalFileChange,
      version: tab.version,
    };
  }

  if (typeof contentOrSession === 'string') {
    throw new Error('Segmented workspace restore requires a newly opened session');
  }
  const segmentedOptions = options as SegmentedRuntimeTabOptions;
  return {
    id: tab.id,
    documentKind: tab.documentKind,
    fileName: tab.fileName,
    filePath: tab.filePath,
    nativePath: tab.nativePath,
    sessionId: contentOrSession.sessionId,
    revision: contentOrSession.revision,
    persistedRevision: contentOrSession.persistedRevision,
    recoveryConflictPath: normalizeRecoveryConflictPath(contentOrSession.recoveryConflictPath),
    selection: tab.selection,
    scrollAnchor: tab.scrollAnchor,
    indexProgress: normalizeIndexProgress(contentOrSession.indexProgress),
    dirty:
      segmentedOptions.dirty ?? contentOrSession.revision !== contentOrSession.persistedRevision,
    lastKnownModifiedAt: segmentedOptions.lastKnownModifiedAt ?? tab.lastKnownModifiedAt,
    diskReadonly: segmentedOptions.diskReadonly ?? contentOrSession.readonly,
    externalFileChange,
  };
}

function shouldPersistDraft(tab: MarkdownTabState) {
  return tab.dirty || (!tab.nativePath && tab.markdown.length > 0);
}

function createWorkspaceDraftSignature(tab: MarkdownTabState) {
  return `${tab.version}:${hashText(tab.markdown)}`;
}

function hashText(value: string) {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${value.length}:${hash >>> 0}`;
}

function toPersistedMarkdownWorkspaceTab(
  tab: MarkdownTabState,
  draftId: string | null,
): PersistedMarkdownWorkspaceTab {
  return normalizePersistedMarkdownWorkspaceTab({ ...tab, draftId });
}

function toPersistedSegmentedWorkspaceTab(
  tab: SegmentedTextTabState,
): PersistedSegmentedWorkspaceTab {
  return normalizePersistedSegmentedWorkspaceTab(tab);
}

function normalizePersistedWorkspaceState(state: PersistedWorkspaceState): PersistedWorkspaceState {
  const tabs = state.tabs.map(normalizePersistedWorkspaceTabByPath);
  const items = normalizeWorkspaceItems(tabs, state.items);
  const activeItem = items.find((item) => item.id === state.activeItemId) ??
    findWorkspaceItemForTab(items, state.activeTabId) ?? items[0];
  return {
    version: 5,
    tabs,
    items,
    activeItemId: activeItem?.id ?? '',
    activeTabId: activeItem ? getWorkspaceItemFocusedTabId(activeItem) : '',
    currentFolderPath:
      typeof state.currentFolderPath === 'string' ? state.currentFolderPath : undefined,
    pendingCloses: normalizePendingCloses(state.pendingCloses, tabs),
  };
}

function normalizePendingCloses(input: unknown, visible: PersistedWorkspaceTab[]): PersistedPendingClose[] {
  if (!Array.isArray(input)) return [];
  const assigned = new Set(visible.map(tab => tab.id));
  return input.flatMap((record: PersistedPendingClose) => {
    if (!record || typeof record.id !== 'string' || !Array.isArray(record.tabs)) return [];
    const tabs = record.tabs.filter(tab => isPersistedWorkspaceTab(tab) && tab.documentKind === 'markdown' && !assigned.has(tab.id)).map(normalizePersistedMarkdownWorkspaceTab);
    if (!tabs.length) return [];
    for (const tab of tabs) assigned.add(tab.id);
    return [{ id: record.id, phase: record.phase === 'readyToRelease' ? 'readyToRelease' : 'pending', tabs,
      positions: tabs.map((tab) => { const index = record.tabs.findIndex(candidate => candidate.id === tab.id); const position = record.positions?.[index]; return Number.isSafeInteger(position) && position >= 0 ? position : visible.length; }),
      items: normalizeWorkspaceItems(tabs, Array.isArray(record.items) ? record.items : []),
      itemPositions: Array.isArray(record.itemPositions) ? record.itemPositions.map(position => Number.isSafeInteger(position) && position >= 0 ? position : visible.length) : undefined,
      decisions: Object.fromEntries(tabs.map(tab => [tab.id, record.decisions?.[tab.id] === 'discard' ? 'discard' : record.decisions?.[tab.id] === 'clean' ? 'clean' : 'save'])),
      revisions: Object.fromEntries(tabs.map(tab => [tab.id, Number.isSafeInteger(record.revisions?.[tab.id]) ? record.revisions[tab.id] : tab.version])),
    } as PersistedPendingClose];
  });
}

function migratePersistedWorkspaceStateV2(
  state: PersistedWorkspaceStateV2,
): PersistedWorkspaceState {
  return normalizePersistedWorkspaceState({
    version: 4,
    tabs: state.tabs.map(normalizePersistedWorkspaceTabByPath),
    activeTabId: state.activeTabId,
    currentFolderPath:
      typeof state.currentFolderPath === 'string' ? state.currentFolderPath : undefined,
  });
}

function normalizePersistedWorkspaceTabByPath(
  tab: PersistedWorkspaceTabV2 | Partial<PersistedWorkspaceTab>,
): PersistedWorkspaceTab {
  const documentKind = resolvePersistedDocumentKind(tab);
  return documentKind === 'markdown'
    ? normalizePersistedMarkdownWorkspaceTab(tab)
    : normalizePersistedSegmentedWorkspaceTab({
        ...tab,
        documentKind,
      });
}

function resolvePersistedDocumentKind(
  tab: PersistedWorkspaceTabV2 | Partial<PersistedWorkspaceTab>,
) {
  // 扩展名是恢复路由的事实源；即使旧元数据缺少 nativePath，也不能把 TXT/JSON 正文送回 Markdown。
  for (const candidate of [tab.nativePath, tab.filePath, tab.fileName]) {
    if (typeof candidate !== 'string') continue;
    const documentKind = getDocumentKindFromPath(candidate);
    if (documentKind) return documentKind;
  }
  return 'markdown';
}

function normalizePersistedMarkdownWorkspaceTab(
  tab: PersistedWorkspaceTabV2 | Partial<PersistedMarkdownWorkspaceTab>,
): PersistedMarkdownWorkspaceTab {
  const common = normalizePersistedCommonWorkspaceTab(tab);
  return {
    ...common,
    documentKind: 'markdown',
    closeFailure: typeof tab.closeFailure === 'string' ? tab.closeFailure : undefined,
    documentId: typeof tab.documentId === 'string' && /^md-[a-z0-9-]+$/i.test(tab.documentId) ? tab.documentId : undefined,
    globalSelection: normalizeGlobalSelection(tab.globalSelection),
    globalScrollAnchor: normalizeGlobalScrollAnchor(tab.globalScrollAnchor),
    draftId: typeof tab.draftId === 'string' && tab.draftId ? tab.draftId : null,
    encoding: normalizeMarkdownEncoding(tab.encoding ?? DEFAULT_MARKDOWN_ENCODING),
    largeDocumentMode: Boolean(tab.largeDocumentMode),
    readonlyDocumentMode: Boolean(tab.readonlyDocumentMode),
    version: typeof tab.version === 'number' ? tab.version : 0,
  };
}

function normalizePersistedSegmentedWorkspaceTab(
  tab: Partial<PersistedSegmentedWorkspaceTab> & { documentKind: 'text' | 'json' },
): PersistedSegmentedWorkspaceTab {
  const common = normalizePersistedCommonWorkspaceTab(tab);
  return {
    ...common,
    documentKind: tab.documentKind,
    recoveryConflictPath: normalizeRecoveryConflictPath(tab.recoveryConflictPath),
    selection: normalizeGlobalSelection(tab.selection),
    scrollAnchor: normalizeGlobalScrollAnchor(tab.scrollAnchor),
  };
}

function normalizePersistedCommonWorkspaceTab(
  tab: PersistedWorkspaceTabV2 | Partial<PersistedWorkspaceTab>,
) {
  return {
    id: typeof tab.id === 'string' && tab.id ? tab.id : createFallbackTabId(),
    fileName: typeof tab.fileName === 'string' ? tab.fileName : 'untitled.md',
    filePath: typeof tab.filePath === 'string' ? tab.filePath : '',
    nativePath: typeof tab.nativePath === 'string' ? tab.nativePath : null,
    dirty: Boolean(tab.dirty),
    lastKnownModifiedAt: typeof tab.lastKnownModifiedAt === 'number' ? tab.lastKnownModifiedAt : 0,
    diskReadonly: Boolean(tab.diskReadonly),
  };
}

function isPersistedWorkspaceStateV2(value: unknown): value is PersistedWorkspaceStateV2 {
  if (!value || typeof value !== 'object') return false;
  const state = value as Partial<PersistedWorkspaceStateV2>;
  return state.version === 2 && Array.isArray(state.tabs) && typeof state.activeTabId === 'string';
}

function isPersistedWorkspaceTab(value: unknown): value is PersistedWorkspaceTab {
  if (!value || typeof value !== 'object') return false;
  const tab = value as Partial<PersistedWorkspaceTab>;
  return (
    (tab.documentKind === 'markdown' ||
      tab.documentKind === 'text' ||
      tab.documentKind === 'json') &&
    typeof tab.id === 'string' &&
    typeof tab.fileName === 'string' &&
    typeof tab.filePath === 'string'
  );
}

function normalizeGlobalSelection(value: unknown): GlobalSelection | null {
  if (!value || typeof value !== 'object') return null;
  const selection = value as Partial<GlobalSelection>;
  if (!isByteOffset(selection.anchorByte) || !isByteOffset(selection.headByte)) return null;
  return { anchorByte: selection.anchorByte, headByte: selection.headByte };
}

function normalizeGlobalScrollAnchor(value: unknown): GlobalScrollAnchor | null {
  if (!value || typeof value !== 'object') return null;
  const anchor = value as Partial<GlobalScrollAnchor>;
  if (!isByteOffset(anchor.byteOffset) || !isByteOffset(anchor.line)) return null;
  return { byteOffset: anchor.byteOffset, line: anchor.line };
}

function isByteOffset(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function normalizeIndexProgress(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function createFallbackTabId() {
  return `tab-${Date.now()}-${Math.random().toString(36).slice(2, 11)}`;
}
