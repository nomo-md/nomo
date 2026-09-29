import type { UnlistenFn } from '@tauri-apps/api/event';
import { createPerfTimer, logDebug, logInfo, perfAsync } from '../../lib/services/logger';
import { normalizeMarkdownEncoding, type MarkdownEncoding } from '../../lib/services/storage';

export interface NativeDocument {
  path: string;
  fileName: string;
  markdown: string;
  /** 兼容旧调用方缺省；真实 Tauri payload 会在规范化后始终提供编码。 */
  encoding?: MarkdownEncoding;
  modifiedAt: number;
  sizeBytes: number;
  readonly: boolean;
}

export type RecentEntryType = 'file' | 'folder';

export interface RecentEntry {
  path: string;
  entryType: RecentEntryType;
  title?: string | null;
  modifiedAt: number;
  wordCount: number;
  openedAt: number;
}

/** @deprecated 使用 RecentEntry */
export type RecentDocument = RecentEntry;

export interface SettingRecord {
  key: string;
  valueJson: string;
  updatedAt: number;
}

export interface FileStatus {
  path: string;
  exists: boolean;
  isFile: boolean;
  modifiedAt: number;
  sizeBytes: number;
  readonly: boolean;
}

export interface SnapshotRecord {
  id: string;
  documentPath: string;
  contentHash: string;
  markdown: string;
  createdAt: number;
  reason: string;
}

export interface WorkspaceDraftRecord {
  draftId: string;
  markdown: string;
  updatedAt: number;
}

export interface WindowState {
  x?: number | null;
  y?: number | null;
  width?: number | null;
  height?: number | null;
}

export type DesktopMenuCommand =
  | 'new-file'
  | 'new-window'
  | 'open-file'
  | 'open-directory'
  | 'save-file'
  | 'save-file-as'
  | 'undo'
  | 'redo'
  | 'toggle-blockquote'
  | 'insert-table'
  | 'insert-math-block'
  | 'insert-code-block'
  | 'toggle-source'
  | 'toggle-theme'
  | 'toggle-focus'
  | string;

interface NativeDocumentPayload {
  path: string;
  file_name: string;
  markdown: string;
  encoding?: MarkdownEncoding;
  modified_at: number;
  size_bytes: number;
  readonly: boolean;
}

interface RecentEntryPayload {
  path: string;
  entry_type: RecentEntryType;
  title?: string | null;
  modified_at: number;
  word_count: number;
  opened_at: number;
}

interface SettingRecordPayload {
  key: string;
  value_json: string;
  updated_at: number;
}

interface FileStatusPayload {
  path: string;
  exists: boolean;
  is_file: boolean;
  modified_at: number;
  size_bytes: number;
  readonly: boolean;
}

interface SnapshotRecordPayload {
  id: string;
  document_path: string;
  content_hash: string;
  markdown: string;
  created_at: number;
  reason: string;
}

interface WorkspaceDraftPayload {
  draft_id: string;
  markdown?: string;
  updated_at: number;
}

interface ExternalOpenPayload {
  windowLabel?: unknown;
  window_label?: unknown;
  paths?: unknown;
}

interface ExternalOpenFolderPayload {
  windowLabel?: unknown;
  window_label?: unknown;
  folder_path?: unknown;
}

interface ExportHtmlInput {
  html_content: string;
  file_path: string;
}

interface ExportPdfInput {
  html_content: string;
  file_path: string;
  paper_size?: string;
  orientation?: string;
  margins?: { top: number; right: number; bottom: number; left: number };
  print_background?: boolean;
  outline?: Array<{ marker_uri: string; title: string; level: number }>;
}

interface ExportResultPayload {
  file_path: string;
  bytes_written: number;
  warnings?: string[];
}

interface Base64FileResultPayload {
  data_url: string;
  mime_type: string;
}

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export async function openMarkdownWithDialog(): Promise<NativeDocument | null> {
  const timer = createPerfTimer('tauriStorage', '打开文件对话框');
  logInfo('tauriStorage', '打开 Markdown 文件选择对话框');
  const selected = await pickPathWithDialog('Markdown', ['md', 'markdown']);

  if (!selected) {
    timer.end({ cancelled: true });
    logInfo('tauriStorage', '用户取消打开 Markdown 文件');
    return null;
  }

  const document = await readMarkdownFile(selected);
  timer.end({ path: selected });
  return document;
}

/**
 * 只选择文档路径，不读取正文。TXT/JSON 必须先经过类型路由，避免误入 Markdown 全量读取链路。
 */
export async function pickDocumentPathWithDialog(): Promise<string | null> {
  return pickPathWithDialog('Documents', ['md', 'markdown', 'txt', 'json']);
}

async function pickPathWithDialog(name: string, extensions: string[]): Promise<string | null> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({
    multiple: false,
    filters: [
      {
        name,
        extensions,
      },
    ],
  });
  return typeof selected === 'string' ? selected : null;
}

export async function openFolderWithDialog(): Promise<string | null> {
  const timer = createPerfTimer('tauriStorage', '打开文件夹对话框');
  logInfo('tauriStorage', '打开文件夹选择对话框');
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({
    multiple: false,
    directory: true,
  });

  timer.end({ selected: typeof selected === 'string' });
  return typeof selected === 'string' ? selected : null;
}

export async function readMarkdownFile(path: string): Promise<NativeDocument> {
  return perfAsync('tauriStorage', 'read_markdown_file', async () => {
    logInfo('tauriStorage', '开始读取 Markdown 文件', { path });
    const { invoke } = await import('@tauri-apps/api/core');
    const document = normalizeDocumentPayload(
      await invoke<NativeDocumentPayload>('read_markdown_file', { path }),
    );
    logInfo('tauriStorage', 'Markdown 文件读取完成', {
      path,
      sizeBytes: document.sizeBytes,
      readonly: document.readonly,
    });
    return document;
  });
}

export async function installSampleDocument(): Promise<NativeDocument> {
  return perfAsync('tauriStorage', 'install_sample_document', async () => {
    logInfo('tauriStorage', '安装或打开实例文档');
    const { invoke } = await import('@tauri-apps/api/core');
    return normalizeDocumentPayload(await invoke<NativeDocumentPayload>('install_sample_document'));
  });
}

export async function saveMarkdownNative(
  path: string | null,
  markdown: string,
  fallbackName: string,
  encoding: MarkdownEncoding | null = null,
): Promise<NativeDocument | null> {
  const timer = createPerfTimer('tauriStorage', '保存 Markdown 文件');
  logInfo('tauriStorage', '开始保存 Markdown 文件', {
    path,
    fallbackName,
    bytes: markdown.length,
    encoding,
  });
  const { invoke } = await import('@tauri-apps/api/core');
  const { save } = await import('@tauri-apps/plugin-dialog');
  const targetPath =
    path ??
    (await save({
      defaultPath: fallbackName,
      filters: [
        {
          name: 'Markdown',
          extensions: ['md', 'markdown'],
        },
      ],
    }));

  if (!targetPath) {
    timer.end({ cancelled: true });
    logInfo('tauriStorage', '用户取消保存 Markdown 文件');
    return null;
  }

  const document = normalizeDocumentPayload(
    await invoke<NativeDocumentPayload>('write_markdown_file_with_encoding', {
      path: targetPath,
      markdown,
      encoding,
    }),
  );
  timer.end({ path: targetPath, bytes: document.sizeBytes });
  logInfo('tauriStorage', 'Markdown 文件保存完成', {
    path: targetPath,
    sizeBytes: document.sizeBytes,
  });
  return document;
}

export async function checkPathsExist(paths: string[]): Promise<boolean[]> {
  logDebug('tauriStorage', '检查路径是否存在', { count: paths.length });
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<boolean[]>('check_paths_exist', { paths });
}

export async function statMarkdownFile(path: string): Promise<FileStatus> {
  return perfAsync('tauriStorage', 'stat_markdown_file', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    return normalizeFileStatus(await invoke<FileStatusPayload>('stat_markdown_file', { path }));
  });
}

export async function rememberRecentEntry(
  path: string,
  entryType: RecentEntryType,
  title: string | null,
  wordCount: number,
): Promise<void> {
  logInfo('tauriStorage', '记录最近打开项', { path, entryType, title, wordCount });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('remember_recent_entry', {
    input: {
      path,
      entry_type: entryType,
      title,
      word_count: wordCount,
    },
  });
}

/** @deprecated 使用 rememberRecentEntry */
export async function rememberRecentFile(
  path: string,
  title: string | null,
  wordCount: number,
): Promise<void> {
  return rememberRecentEntry(path, 'file', title, wordCount);
}

export async function listRecentEntries(): Promise<RecentEntry[]> {
  return perfAsync('tauriStorage', 'list_recent_entries', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    const rows = await invoke<RecentEntryPayload[]>('list_recent_entries');
    logDebug('tauriStorage', '最近打开项读取完成', { count: rows.length });
    return rows.map((row) => ({
      path: row.path,
      entryType: row.entry_type,
      title: row.title,
      modifiedAt: row.modified_at,
      wordCount: row.word_count,
      openedAt: row.opened_at,
    }));
  });
}

/** @deprecated 使用 listRecentEntries */
export async function listRecentFiles(): Promise<RecentDocument[]> {
  return listRecentEntries();
}

export async function removeRecentEntryByPath(path: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('remove_recent_entry', { path });
}

export async function clearRecentEntries(): Promise<void> {
  logInfo('tauriStorage', '清空最近打开项');
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('clear_recent_entries');
}

export async function createDocumentSnapshot(
  path: string,
  markdown: string,
  reason: string,
): Promise<void> {
  logInfo('tauriStorage', '创建文档快照', { path, reason, bytes: markdown.length });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('create_document_snapshot', {
    input: {
      path,
      markdown,
      reason,
    },
  });
}

export async function listDocumentSnapshots(path: string): Promise<SnapshotRecord[]> {
  return perfAsync('tauriStorage', 'list_document_snapshots', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    const rows = await invoke<SnapshotRecordPayload[]>('list_document_snapshots', { path });
    logDebug('tauriStorage', '文档快照读取完成', { path, count: rows.length });
    return rows.map((row) => ({
      id: row.id,
      documentPath: row.document_path,
      contentHash: row.content_hash,
      markdown: row.markdown,
      createdAt: row.created_at,
      reason: row.reason,
    }));
  });
}

export async function updateAppSetting(key: string, value: unknown): Promise<void> {
  logDebug('tauriStorage', '更新应用设置', { key });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('update_app_setting', {
    input: {
      key,
      value_json: JSON.stringify(value),
    },
  });
}

export async function updateAppSettings(entries: Record<string, unknown>): Promise<void> {
  logDebug('tauriStorage', '批量更新应用设置', { keys: Object.keys(entries) });
  const inputs = Object.entries(entries).map(([key, value]) => ({
    key,
    value_json: JSON.stringify(value),
  }));

  if (inputs.length === 0) {
    return;
  }

  await perfAsync('tauriStorage', 'update_app_settings', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('update_app_settings', { inputs });
  });
}

export async function listAppSettings(): Promise<SettingRecord[]> {
  return perfAsync('tauriStorage', 'list_app_settings', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    const rows = await invoke<SettingRecordPayload[]>('list_app_settings');
    return rows.map((row) => ({
      key: row.key,
      valueJson: row.value_json,
      updatedAt: row.updated_at,
    }));
  });
}

export async function updateWindowState(state: WindowState): Promise<void> {
  logDebug('tauriStorage', '更新窗口状态', state);
  const { invoke } = await import('@tauri-apps/api/core');
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const label = getCurrentWindow().label || 'main';
  await invoke('update_window_state', {
    key: `windowState:${label}`,
    input: toSnakeWindowState(state),
  });
}

export async function openExternalLink(href: string): Promise<void> {
  logInfo('tauriStorage', '打开外部链接', { href });
  if (!isTauriRuntime()) {
    window.open(href, '_blank', 'noopener,noreferrer');
    return;
  }

  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('open_external_link', { href });
}

export async function openLocalAttachment(path: string): Promise<void> {
  logInfo('tauriStorage', '打开本地附件', { path });
  if (!isTauriRuntime()) {
    throw new Error('Local attachments can only be opened in the desktop app');
  }

  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('open_local_attachment', { path });
}

export async function createFolder(path: string): Promise<void> {
  logInfo('tauriStorage', '创建文件夹', { path });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('create_folder', { path });
}

export async function renameFile(oldPath: string, newPath: string): Promise<void> {
  logInfo('tauriStorage', '重命名文件', { oldPath, newPath });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('rename_file', { oldPath, newPath });
}

export async function deleteFile(path: string): Promise<void> {
  logInfo('tauriStorage', '删除文件', { path });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('delete_file', { path });
}

export async function revealInExplorer(path: string): Promise<void> {
  logInfo('tauriStorage', '在资源管理器中显示', { path });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('reveal_in_explorer', { path });
}

export async function exportHtmlFile(input: ExportHtmlInput): Promise<ExportResultPayload> {
  logInfo('tauriStorage', '导出 HTML 文件', { filePath: input.file_path });
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<ExportResultPayload>('export_html', { input });
}

export async function exportPdfFromHtml(input: ExportPdfInput): Promise<ExportResultPayload> {
  logInfo('tauriStorage', '导出 PDF 文件', { filePath: input.file_path });
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<ExportResultPayload>('export_pdf_from_html', { input });
}

export async function readFileAsBase64(path: string): Promise<Base64FileResultPayload> {
  logInfo('tauriStorage', '读取文件为 base64', { path });
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<Base64FileResultPayload>('read_file_as_base64', { input: { path } });
}

export async function readCurrentWindowState(): Promise<WindowState> {
  const timer = createPerfTimer('tauriStorage', '读取当前窗口状态');
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const currentWindow = getCurrentWindow();
  const [position, size] = await Promise.all([
    currentWindow.outerPosition(),
    currentWindow.innerSize(),
  ]);

  const state = {
    x: position.x,
    y: position.y,
    width: size.width,
    height: size.height,
  };
  timer.end(state);
  return state;
}

export async function listenDesktopMenuCommands(
  handler: (command: DesktopMenuCommand) => void,
): Promise<UnlistenFn> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<string>('nomo://menu-command', (event) =>
    handler(event.payload as DesktopMenuCommand),
  );
}

export async function listenDesktopFileDrops(
  handler: (paths: string[]) => void,
): Promise<UnlistenFn> {
  const { listen, TauriEvent } = await import('@tauri-apps/api/event');
  return listen<{ paths: string[] }>(TauriEvent.DRAG_DROP, (event) => handler(event.payload.paths));
}

export async function listenDesktopOpenDocuments(
  handler: (paths: string[], windowLabel?: string) => void,
): Promise<UnlistenFn> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<ExternalOpenPayload>('nomo://open-document', (event) => {
    const eventWindowLabel = normalizeEventWindowLabel(event.payload);
    const paths = Array.isArray(event.payload?.paths)
      ? event.payload.paths.filter((path): path is string => typeof path === 'string')
      : [];
    if (paths.length > 0) {
      handler(paths, eventWindowLabel);
    }
  });
}

export async function writeWorkspaceDraft(
  markdown: string,
  draftId?: string | null,
): Promise<WorkspaceDraftRecord> {
  logDebug('tauriStorage', '写入工作区草稿', { draftId, bytes: markdown.length });
  const { invoke } = await import('@tauri-apps/api/core');
  const row = await invoke<WorkspaceDraftPayload>('write_workspace_draft', {
    input: {
      draft_id: draftId ?? null,
      markdown,
    },
  });
  return normalizeWorkspaceDraftPayload(row);
}

export async function readWorkspaceDraft(draftId: string): Promise<WorkspaceDraftRecord> {
  const { invoke } = await import('@tauri-apps/api/core');
  return normalizeWorkspaceDraftPayload(
    await invoke<WorkspaceDraftPayload>('read_workspace_draft', { draftId }),
  );
}

export async function deleteWorkspaceDraft(draftId: string): Promise<void> {
  logDebug('tauriStorage', '删除工作区草稿', { draftId });
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('delete_workspace_draft', { draftId });
}

export async function listenDesktopOpenFolder(
  handler: (folderPath: string, windowLabel?: string) => void,
): Promise<UnlistenFn> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<ExternalOpenFolderPayload>('nomo://open-folder', (event) => {
    const eventWindowLabel = normalizeEventWindowLabel(event.payload);
    const folderPath = event.payload?.folder_path;
    if (typeof folderPath === 'string' && folderPath.length > 0) {
      handler(folderPath, eventWindowLabel);
    }
  });
}

function normalizeEventWindowLabel(
  payload: Pick<ExternalOpenPayload, 'windowLabel' | 'window_label'> | null | undefined,
): string | undefined {
  const label = payload?.windowLabel ?? payload?.window_label;
  return typeof label === 'string' && label.length > 0 ? label : undefined;
}

function normalizeDocumentPayload(payload: NativeDocumentPayload): NativeDocument {
  const document: NativeDocument = {
    path: payload.path,
    fileName: payload.file_name,
    markdown: payload.markdown,
    modifiedAt: payload.modified_at,
    sizeBytes: payload.size_bytes,
    readonly: payload.readonly,
  };
  if (payload.encoding !== undefined) {
    document.encoding = normalizeMarkdownEncoding(payload.encoding);
  }
  return document;
}

function normalizeFileStatus(payload: FileStatusPayload): FileStatus {
  return {
    path: payload.path,
    exists: payload.exists,
    isFile: payload.is_file,
    modifiedAt: payload.modified_at,
    sizeBytes: payload.size_bytes,
    readonly: payload.readonly,
  };
}

function normalizeWorkspaceDraftPayload(payload: WorkspaceDraftPayload): WorkspaceDraftRecord {
  return {
    draftId: payload.draft_id,
    markdown: payload.markdown ?? '',
    updatedAt: payload.updated_at,
  };
}

function toSnakeWindowState(state: WindowState) {
  return {
    x: state.x ?? null,
    y: state.y ?? null,
    width: state.width ?? null,
    height: state.height ?? null,
  };
}

/**
 * 解析 Tauri 原生错误消息，返回错误码和可读消息。
 * - 支持 '[ERROR_CODE] message' 格式的字符串
 * - 对于 Error 对象，提取其 message 属性后递归解析
 * - 非字符串类型回退到 UNKNOWN
 */
export function parseNativeError(error: unknown): { code: string; message: string } {
  if (error instanceof Error) {
    return parseNativeError(error.message);
  }
  if (typeof error !== 'string') {
    return { code: 'UNKNOWN', message: String(error) };
  }
  const match = error.match(/^\[([A-Z_]+)\]\s*(.*)$/);
  if (match) {
    return { code: match[1], message: match[2] };
  }
  return { code: 'UNKNOWN', message: error };
}
