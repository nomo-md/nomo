import {
  createEmptyExternalFileChange,
  normalizeRecoveryConflictPath,
  type DocumentKind,
  type ExternalFileChangeState,
  type GlobalScrollAnchor,
  type GlobalSelection,
  type MarkdownTabState,
  type SegmentedSessionOpenData,
  type SegmentedTextTabState,
  type Tab,
} from '../types';
import { t } from '../i18n';
import {
  DEFAULT_MARKDOWN_ENCODING,
  type MarkdownEncoding,
} from '../../lib/services/storage';

export interface ActiveTabState {
  fileName: string;
  filePath: string;
  nativePath: string | null;
  markdown: string;
  savedMarkdown: string;
  dirty: boolean;
  lastKnownModifiedAt: number;
  largeDocumentMode: boolean;
  readonlyDocumentMode: boolean;
  diskReadonly: boolean;
  externalFileChange: ExternalFileChangeState;
  version: number;
}

export interface MarkdownTabInput {
  id?: string;
  fileName?: string;
  filePath?: string;
  nativePath?: string | null;
  draftId?: string | null;
  markdown?: string;
  savedMarkdown?: string;
  encoding?: MarkdownEncoding;
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  largeDocumentMode?: boolean;
  readonlyDocumentMode?: boolean;
  diskReadonly?: boolean;
  externalFileChange?: ExternalFileChangeState;
  version?: number;
}

export interface SegmentedTextTabInput {
  id?: string;
  documentKind: 'text' | 'json';
  fileName: string;
  filePath: string;
  nativePath: string | null;
  sessionId: string;
  revision: number;
  persistedRevision: number;
  indexProgress: number;
  recoveryConflictPath?: string | null;
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  diskReadonly?: boolean;
  externalFileChange?: ExternalFileChangeState;
  selection?: GlobalSelection | null;
  scrollAnchor?: GlobalScrollAnchor | null;
}

export interface CreateTabForDocumentInput {
  id?: string;
  fileName: string;
  filePath: string;
  nativePath: string | null;
  segmentedSession?: SegmentedSessionOpenData;
  markdown?: string;
  savedMarkdown?: string;
  encoding?: MarkdownEncoding;
  dirty?: boolean;
  lastKnownModifiedAt?: number;
  diskReadonly?: boolean;
}

export function isMarkdownTab(tab: Tab | null | undefined): tab is MarkdownTabState {
  return tab?.documentKind === 'markdown';
}

export function isSegmentedTextTab(tab: Tab | null | undefined): tab is SegmentedTextTabState {
  return tab?.documentKind === 'text' || tab?.documentKind === 'json';
}

export function getDocumentKindFromPath(path: string): DocumentKind | null {
  const normalized = path.trim().toLowerCase();
  if (normalized.endsWith('.md') || normalized.endsWith('.markdown')) return 'markdown';
  if (normalized.endsWith('.txt')) return 'text';
  if (normalized.endsWith('.json')) return 'json';
  return null;
}

export function createMarkdownTab(input: MarkdownTabInput = {}): MarkdownTabState {
  const markdown = input.markdown ?? '';
  return {
    id: input.id ?? createTabId(),
    documentKind: 'markdown',
    fileName: input.fileName ?? 'untitled.md',
    filePath: input.filePath ?? t.untitledMarkdown(),
    nativePath: input.nativePath ?? null,
    draftId: input.draftId ?? null,
    markdown,
    savedMarkdown: input.savedMarkdown ?? markdown,
    encoding: input.encoding ?? DEFAULT_MARKDOWN_ENCODING,
    dirty: input.dirty ?? false,
    lastKnownModifiedAt: input.lastKnownModifiedAt ?? 0,
    largeDocumentMode: input.largeDocumentMode ?? false,
    readonlyDocumentMode: input.readonlyDocumentMode ?? false,
    diskReadonly: input.diskReadonly ?? false,
    externalFileChange: input.externalFileChange ?? createEmptyExternalFileChange(),
    version: input.version ?? 0,
  };
}

export function createSegmentedTextTab(input: SegmentedTextTabInput): SegmentedTextTabState {
  return {
    id: input.id ?? createTabId(),
    documentKind: input.documentKind,
    fileName: input.fileName,
    filePath: input.filePath,
    nativePath: input.nativePath,
    sessionId: input.sessionId,
    revision: input.revision,
    persistedRevision: input.persistedRevision,
    recoveryConflictPath: normalizeRecoveryConflictPath(input.recoveryConflictPath),
    selection: input.selection ?? null,
    scrollAnchor: input.scrollAnchor ?? null,
    indexProgress: normalizeIndexProgress(input.indexProgress),
    dirty: input.dirty ?? input.revision !== input.persistedRevision,
    lastKnownModifiedAt: input.lastKnownModifiedAt ?? 0,
    diskReadonly: input.diskReadonly ?? false,
    externalFileChange: input.externalFileChange ?? createEmptyExternalFileChange(),
  };
}

export function createTabForDocument(input: CreateTabForDocumentInput): Tab {
  const documentKind =
    getDocumentKindFromPath(input.fileName) ?? getDocumentKindFromPath(input.filePath);
  if (!documentKind) {
    throw new Error(`Unsupported document type: ${input.fileName}`);
  }

  if (documentKind === 'markdown') {
    return createMarkdownTab({
      id: input.id,
      fileName: input.fileName,
      filePath: input.filePath,
      nativePath: input.nativePath,
      markdown: input.markdown,
      savedMarkdown: input.savedMarkdown,
      encoding: input.encoding,
      dirty: input.dirty,
      lastKnownModifiedAt: input.lastKnownModifiedAt,
      diskReadonly: input.diskReadonly,
    });
  }

  if (!input.segmentedSession) {
    // TXT/JSON 的正文只能由 Rust 分段会话提供，禁止缺少会话时回退到 Markdown 全量读取。
    throw new Error(`Segmented document requires an open session: ${input.fileName}`);
  }

  return createSegmentedTextTab({
    id: input.id,
    documentKind,
    fileName: input.fileName,
    filePath: input.filePath,
    nativePath: input.nativePath,
    sessionId: input.segmentedSession.sessionId,
    revision: input.segmentedSession.revision,
    persistedRevision: input.segmentedSession.persistedRevision,
    indexProgress: input.segmentedSession.indexProgress,
    recoveryConflictPath: input.segmentedSession.recoveryConflictPath,
    dirty: input.dirty,
    lastKnownModifiedAt: input.lastKnownModifiedAt,
    diskReadonly: input.diskReadonly ?? input.segmentedSession.readonly,
  });
}

export function createBlankTab(
  fileName = 'untitled.md',
  filePath = t.untitledMarkdown(),
): MarkdownTabState {
  return createMarkdownTab({ fileName, filePath });
}

export function getOrCreateReusableTab(tabs: Tab[], activeTabId: string) {
  const activeTab = tabs.find((tab) => tab.id === activeTabId);
  if (isReusableUntitledTab(activeTab)) {
    return { tabs, activeTabId, targetTab: activeTab };
  }

  const newTab = createBlankTab('', '');
  return { tabs: [...tabs, newTab], activeTabId: newTab.id, targetTab: newTab };
}

export function getNativeDocumentTargetTab(
  tabs: Tab[],
  activeTabId: string,
  existingTab: Tab | undefined,
  saved: boolean,
) {
  const activeTab = tabs.find((tab) => tab.id === activeTabId);
  if (isReusableUntitledTab(activeTab)) {
    return { tabs, activeTabId, targetTab: activeTab };
  }
  if (saved) {
    if (isMarkdownTab(existingTab)) {
      return { tabs, activeTabId, targetTab: existingTab };
    }
    const activeTabForSave = tabs.find((tab) => tab.id === activeTabId);
    // Markdown 保存只能复用 Markdown 标签，避免把正文状态写进 TXT/JSON 分段标签。
    if (isMarkdownTab(activeTabForSave)) {
      return { tabs, activeTabId, targetTab: activeTabForSave };
    }
  }

  const newTab = createBlankTab('', '');
  return { tabs: [...tabs, newTab], activeTabId: newTab.id, targetTab: newTab };
}

export function isReusableUntitledTab(tab: Tab | undefined): tab is MarkdownTabState {
  return Boolean(
    isMarkdownTab(tab) &&
    tab.fileName === 'untitled.md' &&
    !tab.dirty &&
    tab.markdown.trim() === '' &&
    !tab.byteLength &&
    !tab.nativePath,
  );
}

export function writeActiveTabState(tabs: Tab[], activeTabId: string, state: ActiveTabState) {
  const activeTab = tabs.find((tab) => tab.id === activeTabId);
  if (!isMarkdownTab(activeTab)) {
    return tabs;
  }

  Object.assign(activeTab, state);
  return [...tabs];
}

function normalizeIndexProgress(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function createTabId() {
  return 'tab-' + Date.now() + '-' + Math.random().toString(36).substring(2, 11);
}
