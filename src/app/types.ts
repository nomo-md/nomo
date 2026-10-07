import type { MarkdownEncoding } from '../lib/services/storage';
import type { EditorMode } from '../lib/editor-core';

export type EditorViewMode = EditorMode | 'split';
export type SplitViewLayout = 'semantic-source' | 'source-semantic';
export type SplitActivePane = EditorMode;

export type DocumentKind = 'markdown' | 'text' | 'json';

export interface CommonTabState {
  id: string;
  fileName: string;
  filePath: string;
  nativePath: string | null;
  dirty: boolean;
  lastKnownModifiedAt: number;
  diskReadonly: boolean;
  externalFileChange: ExternalFileChangeState;
}

export interface MarkdownTabState extends CommonTabState {
  documentKind: 'markdown';
  draftId: string | null;
  markdown: string;
  savedMarkdown: string;
  /** 兼容旧工作区缺省；磁盘读取成功后会以实际源编码覆盖该值。 */
  encoding?: MarkdownEncoding;
  largeDocumentMode: boolean;
  readonlyDocumentMode: boolean;
  version: number;
}

/** 分段选区只保存全局 UTF-8 字节端点，不把不可见正文带入 Svelte 状态。 */
export interface GlobalSelection {
  anchorByte: number;
  headByte: number;
}

/** 恢复滚动位置时以全局字节位置为事实源，行号只用于首屏定位提示。 */
export interface GlobalScrollAnchor {
  byteOffset: number;
  line: number;
}

export interface SegmentedTextTabState extends CommonTabState {
  documentKind: 'text' | 'json';
  sessionId: string;
  revision: number;
  persistedRevision: number;
  recoveryConflictPath: string | null;
  selection: GlobalSelection | null;
  scrollAnchor: GlobalScrollAnchor | null;
  indexProgress: number;
}

export type Tab = MarkdownTabState | SegmentedTextTabState;

/** 标签栏中的一个工作区项；组合只引用文档状态，不复制正文或编辑历史。 */
export type WorkspaceItem = SingleWorkspaceItem | ComparisonWorkspaceItem;

export interface SingleWorkspaceItem {
  kind: 'single';
  id: string;
  tabId: string;
}

export interface ComparisonWorkspaceItem {
  kind: 'comparison';
  id: string;
  leftTabId: string;
  rightTabId: string;
  focusedTabId: string;
  mode: EditorMode;
  leftPercent: number;
}

/** Rust 打开或恢复文件后返回的新会话数据；旧 sessionId 不得跨应用启动复用。 */
export interface SegmentedSessionOpenData {
  sessionId: string;
  revision: number;
  persistedRevision: number;
  indexProgress: number;
  readonly: boolean;
  recoveryConflictPath: string | null;
}

/** 恢复冲突文件只保存路径元数据；缺失或非法值统一视为无冲突。 */
export function normalizeRecoveryConflictPath(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null;
}

export type ExternalFileChangeType = 'none' | 'modified' | 'deleted';

export interface ExternalFileChangeState {
  type: ExternalFileChangeType;
  path: string | null;
  modifiedAt: number;
  dirtyAtDetection: boolean;
  message: string;
}

export function createEmptyExternalFileChange(): ExternalFileChangeState {
  return {
    type: 'none',
    path: null,
    modifiedAt: 0,
    dirtyAtDetection: false,
    message: '',
  };
}

export function normalizeExternalFileChange(value: unknown): ExternalFileChangeState {
  if (!value || typeof value !== 'object') {
    return createEmptyExternalFileChange();
  }

  const state = value as Partial<ExternalFileChangeState>;
  if (state.type !== 'modified' && state.type !== 'deleted') {
    return createEmptyExternalFileChange();
  }

  return {
    type: state.type,
    path: typeof state.path === 'string' ? state.path : null,
    modifiedAt: typeof state.modifiedAt === 'number' ? state.modifiedAt : 0,
    dirtyAtDetection: Boolean(state.dirtyAtDetection),
    message: typeof state.message === 'string' ? state.message : '',
  };
}

export interface FileTreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  has_children?: boolean;
  children_loaded?: boolean;
  loading?: boolean;
  children: FileTreeNode[];
}

export interface WorkspaceState {
  tabs: Tab[];
  activeTabId: string;
  items?: WorkspaceItem[];
  activeItemId?: string;
  currentFolderPath?: string;
}

export interface PersistedCommonWorkspaceTab {
  id: string;
  documentKind: DocumentKind;
  fileName: string;
  filePath: string;
  nativePath: string | null;
  dirty: boolean;
  lastKnownModifiedAt: number;
  diskReadonly: boolean;
}

export interface PersistedMarkdownWorkspaceTab extends PersistedCommonWorkspaceTab {
  documentKind: 'markdown';
  draftId: string | null;
  encoding?: MarkdownEncoding;
  largeDocumentMode: boolean;
  readonlyDocumentMode: boolean;
  version: number;
}

export interface PersistedSegmentedWorkspaceTab extends PersistedCommonWorkspaceTab {
  documentKind: 'text' | 'json';
  recoveryConflictPath: string | null;
  selection: GlobalSelection | null;
  scrollAnchor: GlobalScrollAnchor | null;
}

export type PersistedWorkspaceTab = PersistedMarkdownWorkspaceTab | PersistedSegmentedWorkspaceTab;

export interface PersistedWorkspaceState {
  /** v3 只用于兼容既有输入；持久化写入统一生成 v4。 */
  version: 3 | 4;
  tabs: PersistedWorkspaceTab[];
  activeTabId: string;
  items?: WorkspaceItem[];
  activeItemId?: string;
  currentFolderPath?: string;
}
