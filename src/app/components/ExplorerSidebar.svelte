<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import {
    FileText,
    FolderOpen,
    FolderPlus,
    FilePlus,
    RefreshCw,
    ChevronsUp,
  } from '@lucide/svelte';
  import { slide } from 'svelte/transition';
  import type { FileTreeNode } from '../types';
  import type {
    ContextMenuItem,
    ContextMenuRequest,
  } from '../../lib/editor-core/plugins/contextMenu';
  import { createEventDispatcher } from 'svelte';
  import { clickOutside } from '../actions/clickOutside';
  import {
    explorerSelectionIndicator,
    motionIn,
    pulseOnChange,
    transitionDuration,
  } from '../actions/motion';
  import { getExplorerRenameSelectionRange } from '../services/explorerRename';

  // 重命名输入框专用：延迟激活点击外部检测，避免菜单关闭时的 click 冒泡误触发取消
  function renamingClickOutside(node: HTMLElement, handler: () => void) {
    let active = false;
    const onClick = (event: MouseEvent) => {
      if (!active) return;
      if (node && !node.contains(event.target as Node)) {
        handler();
      }
    };
    const timer = setTimeout(() => {
      active = true;
    }, 0);
    document.addEventListener('click', onClick, true);
    return {
      destroy() {
        clearTimeout(timer);
        document.removeEventListener('click', onClick, true);
      },
    };
  }

  // 重命名输入框挂载后多次应用同一选区，避免菜单点击收尾或重绘覆盖 selection。
  function renameAutoSelect(
    node: HTMLInputElement,
    params: { isDir: boolean },
  ) {
    let disposed = false;
    let frameId: number | null = null;
    let timeoutIds: ReturnType<typeof setTimeout>[] = [];
    let initialValue = '';

    const clearScheduledSelection = () => {
      if (frameId !== null) {
        cancelAnimationFrame(frameId);
        frameId = null;
      }
      if (timeoutIds.length > 0) {
        timeoutIds.forEach((timeoutId) => clearTimeout(timeoutId));
        timeoutIds = [];
      }
    };

    const applySelection = (onlyIfUnedited = false) => {
      if (disposed || !node.isConnected) {
        return;
      }
      if (onlyIfUnedited && node.value !== initialValue) {
        return;
      }

      try {
        node.focus({ preventScroll: true });
      } catch {
        node.focus();
      }

      const range = getExplorerRenameSelectionRange(node.value, params.isDir);
      node.setSelectionRange(range.start, range.end);
    };

    const scheduleSelection = () => {
      clearScheduledSelection();
      void tick().then(() => {
        if (disposed) {
          return;
        }
        initialValue = node.value;
        applySelection();
        frameId = requestAnimationFrame(() => {
          frameId = null;
          applySelection();
          timeoutIds = [
            setTimeout(() => applySelection(true), 0),
            setTimeout(() => applySelection(true), 50),
          ];
        });
      });
    };

    scheduleSelection();

    return {
      update(nextParams: { isDir: boolean }) {
        params = nextParams;
        scheduleSelection();
      },
      destroy() {
        disposed = true;
        clearScheduledSelection();
      },
    };
  }
  import { buildVisibleExplorerRows, type ExplorerTreeRow } from '../services/explorerRows';
  import { canExpandFolderNode } from '../services/folderTree';
  import { t } from '../i18n';

  export let interfaceLocale: string;
  export let currentFolderPath: string;
  export let rootFolderExpanded: boolean;
  export let folderTree: FileTreeNode[];
  export let expandedFolders: Set<string>;
  export let nativePath: string | null;
  export let dirty: boolean;
  export let fileName: string;
  export let filePath: string;
  export let isResizing: boolean;
  export let getFolderName: (path: string) => string;
  export let getDirectoryLabel: (path: string) => string;
  export let toggleRootFolder: () => void;
  export let toggleFolderCollapse: (path: string) => void;
  export let openPreviewFile: (path: string) => void | boolean | Promise<void | boolean>;
  export let pinPreviewFile: () => void;
  export let previewNativePath: string | null;
  export let startResize: (event: MouseEvent) => void;
  export let openContextMenu: (request: ContextMenuRequest) => void = () => undefined;
  export let copyContextText: (text: string) => void | Promise<void> = () => undefined;
  export let copyContextFile: (path: string) => void | Promise<void> = () => undefined;
  export let canCopyFile = false;

  const dispatch = createEventDispatcher<{
    createNode: { parentPath: string; type: 'folder' | 'file'; name: string };
    renameNode: { path: string; newName: string };
    refreshFolder: void;
    collapseAll: void;
    deleteNode: { path: string; isDir: boolean };
    revealError: unknown;
  }>();

  let creatingParentPath: string | null = null;
  let creatingType: 'folder' | 'file' | null = null;
  let creatingValue = '';
  let creatingInputRef: HTMLInputElement | null = null;

  let renamingPath: string | null = null;
  let renamingValue = '';
  let renamingInputRef: HTMLInputElement | null = null;
  let pendingRenameTimer: ReturnType<typeof setTimeout> | null = null;

  // 正在创建中的文件夹路径（用于空文件夹创建时临时显示箭头）
  let pendingCreatePaths: Set<string> = new Set();

  // 文件树双击检测状态（单击预览 / 双击固定）
  let pendingClickTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingClickPath: string | null = null;

  const TREE_ROW_HEIGHT = 30;
  const TREE_OVERSCAN = 8;
  const TREE_BOTTOM_PADDING = 18;
  let fileTreeElement: HTMLElement;
  let fileTreeScrollTop = 0;
  let fileTreeViewportHeight = 0;
  let flattenedRows: ExplorerTreeRow[] = [];
  let virtualRows: ExplorerTreeRow[] = [];
  let virtualTreeHeight = 0;
  let visibleExplorerRowsSignature = '';
  let renderedExplorerRowsSignature = '';
  let lastAutoScrolledExplorerPath = '';
  let lastRenderedFolderPath = currentFolderPath;
  let activeExplorerScrollToken = 0;
  let pendingActiveExplorerScrollSignature = '';

  $: flattenedRows = buildVisibleExplorerRows(
    folderTree,
    expandedFolders,
    creatingParentPath,
    TREE_ROW_HEIGHT,
  );
  $: virtualTreeHeight = flattenedRows.length * TREE_ROW_HEIGHT + TREE_BOTTOM_PADDING;
  $: visibleExplorerRowsSignature = flattenedRows.map((row) => row.key).join('\u001f');
  $: hasStandaloneFile = fileName.trim().length > 0 && filePath.trim().length > 0;
  $: activeExplorerPath = nativePath ?? previewNativePath;
  $: if (currentFolderPath !== lastRenderedFolderPath) {
    lastRenderedFolderPath = currentFolderPath;
    resetFileTreeScrollState();
  }
  $: {
    const start = Math.max(0, Math.floor(fileTreeScrollTop / TREE_ROW_HEIGHT) - TREE_OVERSCAN);
    const visibleCount =
      Math.ceil(Math.max(fileTreeViewportHeight, TREE_ROW_HEIGHT) / TREE_ROW_HEIGHT) +
      TREE_OVERSCAN * 2;
    virtualRows = flattenedRows.slice(start, start + visibleCount);
  }
  $: renderedExplorerRowsSignature = virtualRows.map((row) => row.key).join('\u001f');
  $: activeExplorerRowTop = activeExplorerPath
    ? (flattenedRows.find(
        (row) => row.type === 'file' && sameExplorerPath(row.node.path, activeExplorerPath),
      )?.top ?? null)
    : null;
  $: {
    const autoScrollSignature = [
      currentFolderPath,
      activeExplorerPath ?? '',
      rootFolderExpanded ? 'open' : 'closed',
      String(fileTreeViewportHeight),
      visibleExplorerRowsSignature,
    ].join('\u001e');
    if (
      rootFolderExpanded &&
      activeExplorerPath &&
      activeExplorerPath !== lastAutoScrolledExplorerPath &&
      flattenedRows.length > 0
    ) {
      scheduleActiveExplorerScroll(activeExplorerPath, autoScrollSignature);
    }
  }

  function handleFileTreeScroll(event: Event) {
    const scrollContainer = event.currentTarget as HTMLElement;
    fileTreeScrollTop = Math.max(0, scrollContainer.scrollTop);
  }

  function resetFileTreeScrollState() {
    activeExplorerScrollToken += 1;
    pendingActiveExplorerScrollSignature = '';
    lastAutoScrolledExplorerPath = '';
    fileTreeScrollTop = 0;
    if (fileTreeElement) {
      fileTreeElement.scrollTop = 0;
    }
  }

  function syncFileTreeScrollTopFromDom() {
    fileTreeScrollTop = Math.max(0, fileTreeElement?.scrollTop ?? 0);
  }

  function scheduleActiveExplorerScroll(path: string, signature: string) {
    if (pendingActiveExplorerScrollSignature === signature) {
      return;
    }

    pendingActiveExplorerScrollSignature = signature;
    const token = ++activeExplorerScrollToken;
    scrollActiveExplorerRowIntoView(path, signature, token);
  }

  async function scrollActiveExplorerRowIntoView(
    path: string,
    signature: string,
    token: number,
  ) {
    await tick();
    await waitForAnimationFrame();
    if (token !== activeExplorerScrollToken) {
      return;
    }
    if (!fileTreeElement || path !== activeExplorerPath || !rootFolderExpanded) {
      clearPendingActiveExplorerScroll(signature);
      return;
    }

    if (fileTreeViewportHeight <= 0) {
      syncFileTreeScrollTopFromDom();
      clearPendingActiveExplorerScroll(signature);
      return;
    }

    const activeRow = flattenedRows.find(
      (row) => row.type === 'file' && sameExplorerPath(row.node.path, path),
    );
    if (!activeRow) {
      clearPendingActiveExplorerScroll(signature);
      return;
    }

    const viewportTop = Math.max(0, fileTreeElement.scrollTop);
    const rowTop = activeRow.top;
    const rowBottomWithPadding = rowTop + TREE_ROW_HEIGHT + TREE_BOTTOM_PADDING;
    if (isExplorerRowVisible(activeRow, viewportTop)) {
      syncFileTreeScrollTopFromDom();
      lastAutoScrolledExplorerPath = path;
      clearPendingActiveExplorerScroll(signature);
      return;
    }

    const nextScrollTop =
      rowTop < viewportTop
        ? rowTop
        : Math.max(0, rowBottomWithPadding - Math.max(fileTreeViewportHeight, TREE_ROW_HEIGHT));
    fileTreeElement.scrollTop = nextScrollTop;
    await waitForAnimationFrame();
    if (token !== activeExplorerScrollToken) {
      return;
    }
    if (!fileTreeElement || path !== activeExplorerPath || !rootFolderExpanded) {
      clearPendingActiveExplorerScroll(signature);
      return;
    }
    syncFileTreeScrollTopFromDom();
    if (isExplorerRowVisible(activeRow, Math.max(0, fileTreeElement.scrollTop))) {
      lastAutoScrolledExplorerPath = path;
    }
    clearPendingActiveExplorerScroll(signature);
  }

  function isExplorerRowVisible(row: ExplorerTreeRow, viewportTop: number) {
    const viewportBottom = viewportTop + fileTreeViewportHeight;
    return (
      row.top >= viewportTop &&
      row.top + TREE_ROW_HEIGHT + TREE_BOTTOM_PADDING <= viewportBottom
    );
  }

  function clearPendingActiveExplorerScroll(signature: string) {
    if (pendingActiveExplorerScrollSignature === signature) {
      pendingActiveExplorerScrollSignature = '';
    }
  }

  function waitForAnimationFrame() {
    return new Promise<void>((resolve) => {
      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(() => resolve());
      } else {
        setTimeout(resolve, 0);
      }
    });
  }

  function sameExplorerPath(left: string, right: string) {
    // 统一 Unicode 规范化为 NFC，解决 Mac 文件系统 NFD 与 JS 字符串 NFC 不一致的问题
    return (
      left.replace(/\\/g, '/').toLowerCase().normalize('NFC') ===
      right.replace(/\\/g, '/').toLowerCase().normalize('NFC')
    );
  }

  function isActiveFilePath(path: string) {
    return Boolean(nativePath && sameExplorerPath(nativePath, path));
  }

  function isPreviewFilePath(path: string) {
    return Boolean(previewNativePath && sameExplorerPath(previewNativePath, path));
  }

  function isSelectedFilePath(path: string) {
    return Boolean(activeExplorerPath && sameExplorerPath(activeExplorerPath, path));
  }

  function folderCanExpand(node: FileTreeNode) {
    return canExpandFolderNode(node, pendingCreatePaths.has(node.path));
  }

  function handleFileClick(path: string) {
    // 取消任何待处理的单击（跨文件单击直接替换）
    if (pendingClickTimer) {
      clearTimeout(pendingClickTimer);
      pendingClickTimer = null;
      pendingClickPath = null;
    }

    pendingClickPath = path;
    pendingClickTimer = setTimeout(() => {
      pendingClickTimer = null;
      pendingClickPath = null;
      openPreviewFile(path);
    }, 250);
  }

  async function handleFileDblClick(path: string) {
    if (pendingClickTimer && pendingClickPath === path) {
      clearTimeout(pendingClickTimer);
      pendingClickTimer = null;
      pendingClickPath = null;
    }
    if ((await openPreviewFile(path)) !== false) {
      pinPreviewFile();
    }
  }

  function startCreating(parentPath: string, type: 'folder' | 'file', event?: MouseEvent) {
    event?.stopPropagation();
    creatingParentPath = parentPath;
    creatingType = type;
    creatingValue = '';
    pendingCreatePaths = pendingCreatePaths.add(parentPath);
    // Expand parent if it's collapsed
    if (parentPath === currentFolderPath) {
      if (!rootFolderExpanded) toggleRootFolder();
    } else {
      if (!expandedFolders.has(parentPath)) toggleFolderCollapse(parentPath);
    }
    setTimeout(() => {
      if (creatingInputRef) creatingInputRef.focus();
    }, 0);
  }

  function commitCreating() {
    if (!creatingType || !creatingParentPath) return;
    const value = creatingValue.trim();
    if (!value) {
      // 未输入内容时取消创建
      cancelCreating();
      return;
    }
    dispatch('createNode', {
      parentPath: creatingParentPath,
      type: creatingType,
      name: value,
    });
    pendingCreatePaths.delete(creatingParentPath);
    pendingCreatePaths = pendingCreatePaths;
    creatingParentPath = null;
    creatingType = null;
  }

  function cancelCreating() {
    if (creatingParentPath) {
      pendingCreatePaths.delete(creatingParentPath);
      pendingCreatePaths = pendingCreatePaths;
    }
    creatingParentPath = null;
    creatingType = null;
  }

  function handleCreatingKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      if (creatingValue.trim()) {
        commitCreating();
      } else {
        cancelCreating();
      }
    } else if (event.key === 'Escape') {
      cancelCreating();
    }
  }

  function startRenaming(path: string, currentName: string, event?: MouseEvent) {
    event?.stopPropagation();
    clearPendingRename();
    renamingPath = path;
    renamingValue = currentName;
  }

  function startRenamingFromContextMenu(path: string, currentName: string) {
    clearPendingRename();
    pendingRenameTimer = setTimeout(() => {
      pendingRenameTimer = null;
      startRenaming(path, currentName);
    }, 0);
  }

  function clearPendingRename() {
    if (pendingRenameTimer !== null) {
      clearTimeout(pendingRenameTimer);
      pendingRenameTimer = null;
    }
  }

  function handleFolderDoubleClick(node: FileTreeNode, event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    if (target?.closest('.chevron-icon')) {
      event.stopPropagation();
      return;
    }
    startRenaming(node.path, node.name, event);
  }

  function commitRenaming() {
    if (!renamingPath) return;
    const value = renamingValue.trim();
    if (value) {
      dispatch('renameNode', { path: renamingPath, newName: value });
    }
    renamingPath = null;
  }

  function cancelRenaming() {
    renamingPath = null;
  }

  function handleRenamingKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      commitRenaming();
    } else if (event.key === 'Escape') {
      cancelRenaming();
    }
  }

  // 在系统文件管理器中定位文件/文件夹
  async function revealPathInFolder(path: string) {
    try {
      const { revealInExplorer } = await import('../../lib/desktop/tauriStorage');
      await revealInExplorer(path);
    } catch (error) {
      dispatch('revealError', error);
    }
  }

  function menuSeparator(): ContextMenuItem {
    return { label: '', separator: true };
  }

  function showExplorerContextMenu(event: MouseEvent, items: ContextMenuItem[]) {
    event.preventDefault();
    openContextMenu({ x: event.clientX, y: event.clientY, items });
  }

  function buildRootContextMenuItems(): ContextMenuItem[] {
    if (!currentFolderPath) return [];
    return [
      { label: t.newFile(), icon: 'new-file', action: () => startCreating(currentFolderPath, 'file') },
      { label: t.newFolder(), icon: 'new-folder', action: () => startCreating(currentFolderPath, 'folder') },
      menuSeparator(),
      { label: t.copyPath(), icon: 'copy', action: () => copyContextText(currentFolderPath) },
      { label: t.revealInFolder(), icon: 'folder', action: () => revealPathInFolder(currentFolderPath) },
      menuSeparator(),
      { label: t.refresh(), icon: 'refresh', action: () => dispatch('refreshFolder') },
      { label: t.collapseAll(), icon: 'collapse', action: () => dispatch('collapseAll') },
    ];
  }

  function buildStandaloneContextMenuItems(): ContextMenuItem[] {
    const path = nativePath || filePath;
    if (!path) return [];
    return [
      { label: t.copyFile(), icon: 'copy', disabled: !canCopyFile || !nativePath, action: () => copyContextFile(path) },
      { label: t.copyPath(), icon: 'copy', action: () => copyContextText(path) },
      { label: t.revealInFolder(), icon: 'folder', action: () => revealPathInFolder(path) },
      menuSeparator(),
      { label: t.refresh(), icon: 'refresh', action: () => dispatch('refreshFolder') },
    ];
  }

  function handleExplorerBlankContextMenu(event: MouseEvent) {
    if (event.defaultPrevented) return;
    const target = event.target as HTMLElement | null;
    if (!target || target.closest('input, textarea, select')) return;
    const items = currentFolderPath
      ? buildRootContextMenuItems()
      : buildStandaloneContextMenuItems();
    if (items.length) showExplorerContextMenu(event, items);
  }

  // 步骤1：构建文件右键菜单项
  function buildFileContextMenuItems(node: FileTreeNode): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];

    items.push({
      label: t.open(),
      icon: 'open',
      action: () => openPreviewFile(node.path),
    });
    items.push({
      label: t.openInNewTab(),
      icon: 'open',
      action: () => {
        openPreviewFile(node.path);
        pinPreviewFile();
      },
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.rename(),
      icon: 'edit',
      action: () => startRenamingFromContextMenu(node.path, node.name),
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.copyFile(),
      icon: 'copy',
      disabled: !canCopyFile,
      action: () => copyContextFile(node.path),
    });
    items.push({
      label: t.copyPath(),
      icon: 'copy',
      action: () => copyContextText(node.path),
    });
    items.push({
      label: t.revealInFolder(),
      icon: 'folder',
      action: () => revealPathInFolder(node.path),
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.deleteAction(),
      icon: 'delete',
      danger: true,
      action: () => {
        dispatch('deleteNode', { path: node.path, isDir: false });
      },
    });

    return items;
  }

  // 步骤2：构建文件夹右键菜单项
  function buildFolderContextMenuItems(node: FileTreeNode): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];

    items.push({
      label: t.newFile(),
      icon: 'new-file',
      action: () => startCreating(node.path, 'file'),
    });
    items.push({
      label: t.newFolder(),
      icon: 'new-folder',
      action: () => startCreating(node.path, 'folder'),
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.rename(),
      icon: 'edit',
      action: () => startRenamingFromContextMenu(node.path, node.name),
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.copyPath(),
      icon: 'copy',
      action: () => copyContextText(node.path),
    });
    items.push({
      label: t.revealInFolder(),
      icon: 'folder',
      action: () => revealPathInFolder(node.path),
    });

    items.push({ label: '', action: () => {}, separator: true });
    items.push({
      label: t.refresh(),
      icon: 'refresh',
      action: () => dispatch('refreshFolder'),
    });
    items.push({
      label: t.deleteAction(),
      icon: 'delete',
      danger: true,
      action: () => {
        dispatch('deleteNode', { path: node.path, isDir: true });
      },
    });

    return items;
  }

  // 步骤3：处理文件右键事件
  function handleFileContextMenu(node: FileTreeNode, event: MouseEvent) {
    showExplorerContextMenu(event, buildFileContextMenuItems(node));
  }

  // 步骤4：处理文件夹右键事件
  function handleFolderContextMenu(node: FileTreeNode, event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select')) return;
    showExplorerContextMenu(event, buildFolderContextMenuItems(node));
  }

  onDestroy(clearPendingRename);
</script>

{#key interfaceLocale}
<aside class="rail" aria-label={t.explorer()} data-interface-locale={interfaceLocale}>
  <section
    bind:this={fileTreeElement}
    class="file-tree"
    aria-label={t.folderStructure()}
    bind:clientHeight={fileTreeViewportHeight}
    on:scroll={handleFileTreeScroll}
    on:contextmenu={handleExplorerBlankContextMenu}
  >
    <div class="tree-root">
      {#if currentFolderPath}
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <div
          role="button"
          tabindex="0"
          class="tree-folder-root-title"
          class:collapsed={!rootFolderExpanded}
          title={currentFolderPath}
          on:click={toggleRootFolder}
          on:contextmenu={(event) => showExplorerContextMenu(event, buildRootContextMenuItems())}
        >
          <span class="chevron-icon">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor"
              ><path
                d="M3 4.5l3 3 3-3"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              /></svg
            >
          </span>
          <FolderOpen size={14} />
          <span class="node-name">{getFolderName(currentFolderPath)}</span>
          <div class="folder-actions">
            <button
              type="button"
              class="action-btn"
              title={t.newFile()}
              on:click={(e) => startCreating(currentFolderPath, 'file', e)}
            >
              <FilePlus size={12} />
            </button>
            <button
              type="button"
              class="action-btn"
              title={t.newFolder()}
              on:click={(e) => startCreating(currentFolderPath, 'folder', e)}
            >
              <FolderPlus size={12} />
            </button>
            <button
              type="button"
              class="action-btn"
              title={t.refresh()}
              on:click={(event) => {
                event.stopPropagation();
                dispatch('refreshFolder');
              }}
            >
              <RefreshCw size={12} />
            </button>
            <button
              type="button"
              class="action-btn"
              title={t.collapseAll()}
              on:click={(event) => {
                event.stopPropagation();
                dispatch('collapseAll');
              }}
            >
              <ChevronsUp size={12} />
            </button>
          </div>
        </div>

        {#if creatingParentPath === currentFolderPath}
          <div
            class="tree-creating"
            style="padding-left: 34px"
            use:motionIn={{ kind: 'row', y: -4 }}
            transition:slide={{ duration: transitionDuration('row') }}
          >
            {#if creatingType === 'folder'}
              <FolderOpen size={13} />
            {:else}
              <FileText size={13} />
            {/if}
            <input
              bind:this={creatingInputRef}
              bind:value={creatingValue}
              on:blur={cancelCreating}
              on:keydown={handleCreatingKeydown}
              class="rename-input"
              use:clickOutside={cancelCreating}
              placeholder={creatingType === 'folder' ? t.newFolder() : t.untitledMarkdown()}
            />
          </div>
        {/if}

        {#if rootFolderExpanded}
          <div
            class="recent-tree recursive-tree-container virtual-tree-viewport"
            style="height: {virtualTreeHeight}px"
            data-selection-ready="false"
            use:explorerSelectionIndicator={{
              activePath: activeExplorerPath ?? '',
              top: activeExplorerRowTop,
              layoutKey: `${currentFolderPath}\u001e${visibleExplorerRowsSignature}`,
              renderKey: renderedExplorerRowsSignature,
            }}
          >
            <span class="explorer-selection-indicator" aria-hidden="true"></span>
            {#each virtualRows as row (row.key)}
              <div class="tree-virtual-row" style="transform: translateY({row.top}px)">
                {#if row.type === 'folder'}
                  {@const node = row.node}
                  {@const isExpanded = expandedFolders.has(node.path)}
                  {@const hasChildren = folderCanExpand(node)}
                  <div
                    class="tree-folder-wrapper"
                    class:expanded={isExpanded && hasChildren}
                    style="--tree-depth: {row.depth}"
                  >
                    <!-- svelte-ignore a11y-click-events-have-key-events -->
                    <div
                      role="button"
                      tabindex="0"
                      class="tree-folder nested-dir"
                      class:collapsed={!isExpanded}
                      class:empty={!hasChildren}
                      style="padding-left: {12 + row.depth * 12}px"
                      title={node.path}
                      on:click={() => hasChildren && toggleFolderCollapse(node.path)}
                      on:dblclick={(event) => handleFolderDoubleClick(node, event)}
                      on:contextmenu={(event) =>
                        handleFolderContextMenu(node, event)}
                    >
                      {#if hasChildren}
                        <span class="chevron-icon">
                          <svg
                            width="12"
                            height="12"
                            viewBox="0 0 12 12"
                            fill="none"
                            stroke="currentColor"
                            ><path
                              d="M3 4.5l3 3 3-3"
                              stroke-width="1.5"
                              stroke-linecap="round"
                              stroke-linejoin="round"
                            /></svg
                          >
                        </span>
                      {:else}
                        <span class="chevron-placeholder"></span>
                      {/if}
                      {#if node.loading}
                        <span class="tree-loading-icon" aria-hidden="true"
                          ><RefreshCw size={13} /></span
                        >
                      {:else}
                        <FolderOpen size={13} />
                      {/if}
                      {#if renamingPath === node.path}
                        <span class="rename-input-wrapper" use:motionIn={{ kind: 'micro', y: -2 }}>
                          <input
                            bind:this={renamingInputRef}
                            bind:value={renamingValue}
                            on:blur={cancelRenaming}
                            on:keydown={handleRenamingKeydown}
                            class="rename-input"
                            use:renamingClickOutside={cancelRenaming}
                            use:renameAutoSelect={{ isDir: node.is_dir }}
                            on:click|stopPropagation
                          />
                        </span>
                      {:else}
                        <span class="node-name">{node.name}</span>
                        <div class="folder-actions">
                          <button
                            type="button"
                            class="action-btn"
                            title={t.newFile()}
                            on:click={(e) => startCreating(node.path, 'file', e)}
                          >
                            <FilePlus size={12} />
                          </button>
                          <button
                            type="button"
                            class="action-btn"
                            title={t.newFolder()}
                            on:click={(e) => startCreating(node.path, 'folder', e)}
                          >
                            <FolderPlus size={12} />
                          </button>
                        </div>
                      {/if}
                    </div>
                  </div>
                {:else if row.type === 'creating'}
                  <div
                    class="tree-creating"
                    style="padding-left: {34 + row.depth * 12}px"
                    use:motionIn={{ kind: 'row', y: -4 }}
                  >
                    {#if creatingType === 'folder'}
                      <FolderOpen size={13} />
                    {:else}
                      <FileText size={13} />
                    {/if}
                    <input
                      bind:this={creatingInputRef}
                      bind:value={creatingValue}
                      on:blur={cancelCreating}
                      on:keydown={handleCreatingKeydown}
                      class="rename-input"
                      use:clickOutside={cancelCreating}
                      placeholder={creatingType === 'folder' ? t.newFolder() : t.untitledMarkdown()}
                    />
                  </div>
                {:else}
                  {@const node = row.node}
                  {#if renamingPath === node.path}
                    <div
                      class="tree-file tree-file-renaming"
                      style="padding-left: {34 + row.depth * 12}px"
                      use:motionIn={{ kind: 'micro', y: -2 }}
                    >
                      <FileText size={13} />
                      <input
                        bind:this={renamingInputRef}
                        bind:value={renamingValue}
                        on:blur={cancelRenaming}
                        on:keydown={handleRenamingKeydown}
                        class="rename-input"
                        use:renamingClickOutside={cancelRenaming}
                        use:renameAutoSelect={{ isDir: false }}
                        on:click|stopPropagation
                      />
                    </div>
                  {:else}
                    <button
                      type="button"
                      class="tree-file"
                      class:active={isActiveFilePath(node.path)}
                      class:selected={isSelectedFilePath(node.path)}
                      class:preview={isPreviewFilePath(node.path)}
                      style="padding-left: {34 + row.depth * 12}px"
                      title={node.path}
                      on:click={() => handleFileClick(node.path)}
                      on:dblclick={() => handleFileDblClick(node.path)}
                      on:contextmenu={(event) => handleFileContextMenu(node, event)}
                    >
                      <FileText size={13} />
                      <span>{node.name}</span>
                    </button>
                  {/if}
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {:else if hasStandaloneFile}
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <div
          role="button"
          tabindex="0"
          class="tree-folder-root-title"
          class:collapsed={!rootFolderExpanded}
          title={getDirectoryLabel(filePath)}
          on:click={toggleRootFolder}
          on:contextmenu={(event) =>
            showExplorerContextMenu(event, buildStandaloneContextMenuItems())}
        >
          <span class="chevron-icon">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor"
              ><path
                d="M3 4.5l3 3 3-3"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              /></svg
            >
          </span>
          <FolderOpen size={14} />
          <span class="node-name">{getFolderName(getDirectoryLabel(filePath))}</span>
          <div class="folder-actions">
            <button
              type="button"
              class="action-btn"
              title={t.refresh()}
              on:click={(event) => {
                event.stopPropagation();
                dispatch('refreshFolder');
              }}
            >
              <RefreshCw size={12} />
            </button>
            <button
              type="button"
              class="action-btn"
              title={t.collapseAll()}
              on:click={(event) => {
                event.stopPropagation();
                dispatch('collapseAll');
              }}
            >
              <ChevronsUp size={12} />
            </button>
          </div>
        </div>

        {#if rootFolderExpanded}
          <button
            type="button"
            class="tree-file active"
            title={filePath}
            use:pulseOnChange={dirty}
            on:contextmenu={(event) =>
              showExplorerContextMenu(event, buildStandaloneContextMenuItems())}
          >
            <FileText size={13} />
            <span>{fileName}</span>
          </button>
        {/if}
      {/if}
    </div>
  </section>
  <button
    type="button"
    class="sidebar-resizer"
    class:active={isResizing}
    aria-label={t.resizeSidebar()}
    on:mousedown={startResize}
  ></button>
</aside>
{/key}
