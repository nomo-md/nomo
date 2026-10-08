import type { FileTreeNode } from '../types';
import { checkPathsExist } from '../../lib/desktop/tauriStorage';
import { getFolderName } from '../utils/pathLabels';
import { loadFolderChildren, loadFolderTree, pickFolderPath } from './documentFiles';
import {
  collectTreePaths,
  expandAncestors as expandFolderAncestors,
  findTreeNode,
  markFolderLoading,
  normalizeFolderEntries,
  pruneExpandedFolders,
  removeTreePaths,
  toggleExpandedFolder,
  updateFolderChildren,
} from './folderTree';
import { t } from '../i18n';

interface FolderExplorerControllerOptions {
  getDesktopEnabled(): boolean;
  getFolderTree(): FileTreeNode[];
  setFolderTree(value: FileTreeNode[]): void;
  getExpandedFolders(): Set<string>;
  setExpandedFolders(value: Set<string>): void;
  getRootFolderExpanded(): boolean;
  setRootFolderExpanded(value: boolean): void;
  getCurrentFolderPath(): string;
  setCurrentFolderPath(value: string): void;
  setStatusMessage(value: string): void;
}

export function createFolderExplorerController(options: FolderExplorerControllerOptions) {
  let syncInFlight: Promise<void> | null = null;

  async function expandAncestors(filePath: string, rootPath: string) {
    options.setExpandedFolders(
      expandFolderAncestors(options.getExpandedFolders(), filePath, rootPath),
    );
    await loadAncestorFolders(filePath, rootPath);
  }

  function toggleFolderCollapse(folderPath: string) {
    const currentExpanded = options.getExpandedFolders();
    const willExpand = !currentExpanded.has(folderPath);
    options.setExpandedFolders(toggleExpandedFolder(currentExpanded, folderPath));

    if (willExpand) {
      const node = findTreeNode(options.getFolderTree(), folderPath);
      if (node?.is_dir && !node.children_loaded && !node.loading) {
        loadFolderChildrenIntoTree(folderPath).catch((error) => {
          options.setStatusMessage(t.loadSubfolderFailed({ error }));
        });
      }
    }
  }

  function toggleRootFolder() {
    options.setRootFolderExpanded(!options.getRootFolderExpanded());
  }

  async function loadFolder(folderPath: string) {
    options.setCurrentFolderPath(folderPath);
    options.setRootFolderExpanded(true);
    options.setExpandedFolders(new Set());
    const result = await loadFolderTree(folderPath);

    if ('error' in result) {
      options.setStatusMessage(result.error);
      options.setCurrentFolderPath('');
      options.setFolderTree(normalizeFolderEntries(result.tree));
      return;
    }

    options.setFolderTree(normalizeFolderEntries(result));
    options.setStatusMessage(t.workspaceLoaded({ name: getFolderName(folderPath) }));
  }

  function syncLoadedFolders(): Promise<void> {
    if (syncInFlight) {
      return syncInFlight;
    }

    const operation = syncLoadedFoldersOnce();
    syncInFlight = operation;
    operation.then(clearSyncInFlight, clearSyncInFlight);
    return operation;

    function clearSyncInFlight() {
      if (syncInFlight === operation) {
        syncInFlight = null;
      }
    }
  }

  async function refreshLoadedFolders() {
    // 用户操作完成后必须读取新快照，不能复用操作前已经开始的后台刷新。
    if (syncInFlight) {
      await syncInFlight;
    }
    await syncLoadedFolders();
  }

  async function syncLoadedFoldersOnce() {
    const rootPath = options.getCurrentFolderPath();
    if (!options.getDesktopEnabled() || !rootPath) {
      return;
    }

    const knownPaths = [rootPath, ...collectTreePaths(options.getFolderTree())];
    const exists = await checkPathsExist(knownPaths).catch(() => knownPaths.map(() => true));
    const missingPaths = knownPaths.filter((_path, index) => !exists[index]);

    if (missingPaths.some((path) => samePath(path, rootPath))) {
      clearMissingRoot(rootPath);
      return;
    }

    const foldersToReload = getLoadedFolderPathsForRefresh(
      rootPath,
      options.getExpandedFolders(),
      missingPaths,
    );
    let nextFolderTree = options.getFolderTree();
    const pathsToPrune = [...missingPaths];
    if (missingPaths.length > 0) {
      nextFolderTree = removeTreePaths(nextFolderTree, missingPaths);
    }

    for (const folderPath of foldersToReload) {
      const result = await loadFolderChildren(folderPath, rootPath);
      if ('error' in result) {
        if (samePath(folderPath, rootPath)) {
          clearMissingRoot(rootPath);
          return;
        }
        nextFolderTree = removeTreePaths(nextFolderTree, [folderPath]);
        pathsToPrune.push(folderPath);
        continue;
      }

      if (samePath(folderPath, rootPath)) {
        nextFolderTree = normalizeFolderEntries(result);
      } else if (findTreeNode(nextFolderTree, folderPath)?.is_dir) {
        nextFolderTree = updateFolderChildren(nextFolderTree, folderPath, result);
      }
    }

    options.setFolderTree(nextFolderTree);
    const currentExpandedFolders = options.getExpandedFolders();
    const nextExpandedFolders = pruneExpandedFolders(currentExpandedFolders, pathsToPrune);
    if (nextExpandedFolders !== currentExpandedFolders) {
      options.setExpandedFolders(nextExpandedFolders);
    }

    if (missingPaths.length > 0) {
      options.setStatusMessage(t.syncedExternalDeletes({ count: missingPaths.length }));
    }
  }

  function removeMissingPaths(paths: string[], showMessage = true) {
    const rootPath = options.getCurrentFolderPath();
    if (rootPath && paths.some((path) => samePath(path, rootPath))) {
      clearMissingRoot(rootPath);
      return;
    }

    const missingPaths = paths.filter((path) => !rootPath || !samePath(path, rootPath));
    if (missingPaths.length === 0) {
      return;
    }

    options.setFolderTree(removeTreePaths(options.getFolderTree(), missingPaths));
    options.setExpandedFolders(pruneExpandedFolders(options.getExpandedFolders(), missingPaths));
    if (showMessage) {
      options.setStatusMessage(t.missingPathRemoved({ name: getFolderName(missingPaths[0]) }));
    }
  }

  function clearMissingRoot(rootPath: string) {
    options.setCurrentFolderPath('');
    options.setRootFolderExpanded(true);
    options.setExpandedFolders(new Set());
    options.setFolderTree([]);
    options.setStatusMessage(t.workspaceMissing({ path: rootPath }));
  }

  async function loadFolderChildrenIntoTree(folderPath: string) {
    const rootPath = options.getCurrentFolderPath();
    if (!rootPath) return;

    options.setFolderTree(markFolderLoading(options.getFolderTree(), folderPath, true));
    const result = await loadFolderChildren(folderPath, rootPath);

    if ('error' in result) {
      options.setFolderTree(markFolderLoading(options.getFolderTree(), folderPath, false));
      options.setStatusMessage(result.error);
      return;
    }

    options.setFolderTree(updateFolderChildren(options.getFolderTree(), folderPath, result));
  }

  async function loadAncestorFolders(filePath: string, rootPath: string) {
    const ancestors = getAncestorFolderPaths(filePath, rootPath);
    for (const folderPath of ancestors) {
      const node = findTreeNode(options.getFolderTree(), folderPath);
      if (node?.is_dir && !node.children_loaded && !node.loading) {
        await loadFolderChildrenIntoTree(folderPath);
      }
    }
  }

  async function openFolderDialog() {
    if (!options.getDesktopEnabled()) {
      return;
    }

    const { folderPath, error } = await pickFolderPath();
    if (error) {
      options.setStatusMessage(error);
    }
    if (folderPath) {
      await loadFolder(folderPath);
    }
  }

  return {
    expandAncestors,
    toggleFolderCollapse,
    toggleRootFolder,
    loadFolder,
    removeMissingPaths,
    syncLoadedFolders,
    refreshLoadedFolders,
    openFolderDialog,
  };
}

function getLoadedFolderPathsForRefresh(
  rootPath: string,
  expandedFolders: Set<string>,
  removedPaths: string[],
): string[] {
  return [rootPath].concat(
    Array.from(expandedFolders)
      .filter((folderPath) => !samePath(folderPath, rootPath))
      .filter(
        (folderPath) =>
          !removedPaths.some((removedPath) => pathMatchesOrDescendsFrom(folderPath, removedPath)),
      )
      .sort((left, right) => pathDepth(left) - pathDepth(right)),
  );
}

function getAncestorFolderPaths(filePath: string, rootPath: string) {
  const normalizedFile = normalizePath(filePath);
  const normalizedRoot = normalizePath(rootPath);
  if (!normalizedFile.startsWith(normalizedRoot)) {
    return [];
  }

  const relative = normalizedFile.slice(normalizedRoot.length);
  const parts = relative.split('/').filter(Boolean);
  const ancestors: string[] = [];
  let currentPath = normalizedRoot;
  for (let i = 0; i < parts.length - 1; i++) {
    currentPath = currentPath + '/' + parts[i];
    ancestors.push(toPlatformPath(currentPath, rootPath));
  }
  return ancestors;
}

function normalizePath(path: string) {
  // 统一 Unicode 规范化为 NFC，解决 Mac 文件系统 NFD 与 JS 字符串 NFC 不一致的问题
  return path.replace(/\\/g, '/').replace(/\/$/, '').normalize('NFC');
}

function toPlatformPath(path: string, referencePath: string) {
  return referencePath.includes('\\')
    ? path.replace(/\//g, '\\').normalize('NFC')
    : path.normalize('NFC');
}

function samePath(left: string, right: string) {
  return normalizePath(left).toLowerCase() === normalizePath(right).toLowerCase();
}

function pathMatchesOrDescendsFrom(path: string, ancestorPath: string) {
  const normalizedPath = normalizePath(path).toLowerCase();
  const normalizedAncestor = normalizePath(ancestorPath).toLowerCase();
  return (
    normalizedPath === normalizedAncestor || normalizedPath.startsWith(`${normalizedAncestor}/`)
  );
}

function pathDepth(path: string) {
  return normalizePath(path).split('/').filter(Boolean).length;
}
