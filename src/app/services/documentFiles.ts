import {
  createDocumentSnapshot,
  listRecentEntries,
  listDocumentSnapshots,
  openFolderWithDialog,
  openMarkdownWithDialog,
  readMarkdownFile,
  rememberRecentEntry,
  saveMarkdownNative,
  statMarkdownFile,
  type FileStatus,
  type NativeDocument,
  type RecentEntry,
  type SnapshotRecord,
} from '../../lib/desktop/tauriStorage';
import {
  createEmptyExternalFileChange,
  type ExternalFileChangeState,
  type FileTreeNode,
} from '../types';
import { t } from '../i18n';
import type { MarkdownEncoding } from '../../lib/services/storage';

/** 串行打开，单项失败不阻断后续文件；批次只保留错误信息，不持有正文。 */
export async function openDocumentBatch(
  paths: readonly string[],
  openDocument: (path: string) => Promise<unknown>,
  fallbackMessage: string,
): Promise<Array<{ path: string; message: string }>> {
  const failures: Array<{ path: string; message: string }> = [];
  for (const path of paths) {
    try {
      await openDocument(path);
    } catch (error) {
      const message = error instanceof Error ? error.message
        : typeof error === 'string' && error.trim() ? error : fallbackMessage;
      failures.push({ path, message });
    }
  }
  return failures;
}

export function findDroppedDocumentPath(paths: string[]) {
  return paths.find((path) => /\.(md|markdown|txt|json)$/i.test(path)) ?? null;
}

/** @deprecated 使用 findDroppedDocumentPath，旧名称只保留给现有 Markdown 调用方迁移。 */
export function findDroppedMarkdownPath(paths: string[]) {
  return findDroppedDocumentPath(paths);
}

export async function openMarkdownFromDialog() {
  return openMarkdownWithDialog()
    .catch((error) => ({
      error: error instanceof Error ? error.message : t.openFileFailed(),
      document: null,
    }))
    .then((result) => normalizeDocumentResult(result));
}

export async function readMarkdownFromPath(path: string, fallbackMessage: string) {
  return readMarkdownFile(path)
    .catch((error) => ({
      error: error instanceof Error ? error.message : fallbackMessage,
      document: null,
    }))
    .then((result) => normalizeDocumentResult(result));
}

/** 打开只取文件身份，正文由原生 Markdown 会话捕获并按窗口读取。 */
export async function readMarkdownMetadata(path: string, fallbackMessage: string): Promise<{ document: NativeDocument | null; error: string | null }> {
  try {
    const status = await statMarkdownFile(path);
    if (!status.exists || !status.isFile) throw new Error(fallbackMessage);
    return { document: { path: status.path, fileName: status.path.split(/[\\/]/).pop() ?? path,
      markdown: '', sizeBytes: status.sizeBytes, modifiedAt: status.modifiedAt, readonly: status.readonly, encoding: 'utf-8' }, error: null };
  } catch (error) { return { document: null, error: error instanceof Error ? error.message : fallbackMessage }; }
}

export async function saveNativeMarkdownFile(
  path: string | null,
  markdown: string,
  fileName: string,
  snapshotPath: string | null,
  encoding: MarkdownEncoding | null = null,
) {
  if (snapshotPath) {
    await createDocumentSnapshot(snapshotPath, markdown, 'before-save').catch(() => undefined);
  }

  return saveMarkdownNative(path, markdown, fileName, encoding)
    .catch((error) => ({
      error:
        typeof error === 'string' && error.trim()
          ? error
          : error instanceof Error
            ? error.message
            : t.saveFileFailed(),
      document: null,
    }))
    .then((result) => normalizeDocumentResult(result));
}

export function exportMarkdownInBrowser(markdown: string, fileName: string) {
  const blob = new Blob([markdown], { type: 'text/markdown;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName.endsWith('.md') ? fileName : `${fileName}.md`;
  anchor.click();
  URL.revokeObjectURL(url);
}

export async function rememberNativeDocument(document: NativeDocument, words: number) {
  await rememberRecentEntry(document.path, 'file', document.fileName, words).catch(() => undefined);
}

export async function rememberNativeFolder(folderPath: string) {
  await rememberRecentEntry(folderPath, 'folder', null, 0).catch(() => undefined);
}

export async function loadRecentEntries(desktopEnabled: boolean): Promise<RecentEntry[]> {
  if (!desktopEnabled) {
    return [];
  }
  return listRecentEntries().catch(() => []);
}

/** @deprecated 使用 loadRecentEntries */
async function loadRecentDocuments(desktopEnabled: boolean): Promise<RecentEntry[]> {
  return loadRecentEntries(desktopEnabled);
}

async function loadDocumentSnapshots(
  desktopEnabled: boolean,
  nativePath: string | null,
): Promise<SnapshotRecord[]> {
  if (!desktopEnabled || !nativePath) {
    return [];
  }
  return listDocumentSnapshots(nativePath).catch(() => []);
}

export async function getExternalFileChange(
  desktopEnabled: boolean,
  nativePath: string | null,
  lastKnownModifiedAt: number,
  dirty: boolean,
): Promise<ExternalFileChangeState> {
  if (!desktopEnabled || !nativePath || lastKnownModifiedAt === 0) {
    return createEmptyExternalFileChange();
  }

  const status = await statMarkdownFile(nativePath).catch((error) => ({
    error: error instanceof Error ? error.message : t.fileStatusCheckFailed(),
  }));

  if ('error' in status) {
    return {
      type: 'deleted',
      path: nativePath,
      modifiedAt: 0,
      dirtyAtDetection: dirty,
      message: status.error,
    };
  }

  return resolveExternalFileChange({
    desktopEnabled,
    nativePath,
    lastKnownModifiedAt,
    dirty,
    status,
  });
}

export function resolveExternalFileChange(input: {
  desktopEnabled: boolean;
  nativePath: string | null;
  lastKnownModifiedAt: number;
  dirty: boolean;
  status: Pick<FileStatus, 'exists' | 'modifiedAt'> | null;
}): ExternalFileChangeState {
  const { desktopEnabled, nativePath, lastKnownModifiedAt, dirty, status } = input;
  if (!desktopEnabled || !nativePath || lastKnownModifiedAt === 0 || !status) {
    return createEmptyExternalFileChange();
  }

  if (!status.exists) {
    return {
      type: 'deleted',
      path: nativePath,
      modifiedAt: 0,
      dirtyAtDetection: dirty,
      message: t.externalFileDeleted(),
    };
  }

  if (status.modifiedAt > lastKnownModifiedAt) {
    return {
      type: 'modified',
      path: nativePath,
      modifiedAt: status.modifiedAt,
      dirtyAtDetection: dirty,
      message: dirty ? t.externalFileModifiedDirty() : t.externalFileModifiedClean(),
    };
  }

  return createEmptyExternalFileChange();
}

export async function pickFolderPath() {
  return openFolderWithDialog()
    .catch((error) => ({
      error: error instanceof Error ? error.message : t.openFolderFailed(),
      folderPath: null,
    }))
    .then((result) => {
      if (typeof result === 'string') {
        return { folderPath: result, error: '' };
      }
      if (!result) {
        return { folderPath: null, error: '' };
      }
      return result;
    });
}

export async function loadFolderTree(path: string) {
  return loadFolderChildren(path, path);
}

export async function loadFolderChildren(path: string, rootPath: string) {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<FileTreeNode[]>('list_folder_children', { path, rootPath }).catch((error) => ({
    error: error instanceof Error ? error.message : t.loadFolderTreeFailed(),
    tree: [],
  }));
}

function normalizeDocumentResult(
  result: NativeDocument | { error: string; document: null } | null,
) {
  if (!result) {
    return { document: null, error: '' };
  }
  if ('error' in result) {
    return result;
  }
  return { document: result, error: '' };
}
