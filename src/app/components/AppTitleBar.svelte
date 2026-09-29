<script lang="ts">
  import { onMount } from 'svelte';
  import {
    ArrowDownLeft,
    Download,
    LockKeyhole,
    Moon,
    PanelLeftClose,
    PanelLeftOpen,
    PictureInPicture2,
    Pin,
    PinOff,
    Sun,
  } from '@lucide/svelte';
  import type { SoftwareUpdateSnapshot } from '../../lib/desktop/tauriUpdater';
  import type { RecentEntry } from '../../lib/desktop/tauriStorage';
  import {
    DIAGRAM_TEMPLATES,
    type DiagramType,
    type EditorCommand,
    type ContextMenuRequest,
  } from '../../lib/editor-core';
  import type { EditorViewMode } from '../types';
  import { clickOutside } from '../actions/clickOutside';
  import { getPlatformCapabilities } from '../services/platform';
  import { getDiagramTypeLabel, t } from '../i18n';
  import WindowsCaptionControls from './WindowsCaptionControls.svelte';

  export let interfaceLocale: string;
  export let theme: 'light' | 'dark';
  export let desktopEnabled: boolean;
  export let activeMenu: string | null;
  export let recentFiles: RecentEntry[];
  export let missingRecentPaths: Set<string>;
  export let mode: EditorViewMode;
  export let focusMode: boolean;
  export let toolbarHidden: boolean;
  export let toolbarShortcut: string;
  export let markdownMiniShortcut: string;
  export let markdownMiniAvailable: boolean;
  export let markdownMiniActive: boolean;
  export let markdownMiniPinned: boolean;
  export let markdownMiniExternalChanged: boolean;
  export let fileName: string;
  export let filePath: string;
  export let dirty: boolean;
  export let largeDocumentMode: boolean;
  export let getCompactPath: (path: string) => string;
  export let toggleMenu: (menu: string) => void;
  export let closeMenu: (menu: string) => void;
  export let toggleTheme: () => void;
  export let exitApp: () => void;
  export let createNewWindow: () => void;
  export let createNewFile: () => void;
  export let openFileDialog: () => void;
  export let openFolderDialog: () => void;
  export let openRecentEntry: (path: string, entryType: 'file' | 'folder') => void;
  export let saveMarkdownFile: (saveAs?: boolean) => void;
  export let clearRecentEntriesList: () => void;
  export let removeRecentEntry: (path: string) => void;
  export let closeCurrentFile: () => void;
  export let closeCurrentWindow: () => void;
  export let runCommand: (command: EditorCommand) => void;
  export let openTablePicker: () => void;
  export let openLinkPicker: () => void;
  export let editFrontMatter: () => void;
  export let showUnavailableFeature: (featureName: string) => void;
  export let setMode: (mode: EditorViewMode) => void;
  export let toggleOutlineVisible: () => void;
  export let outlineVisible: boolean;
  export let toggleFocusMode: () => void;
  export let toggleToolbar: () => void;
  export let toggleMarkdownMini: () => void;
  export let toggleMarkdownMiniPinned: () => void;
  export let openSettings: () => void;
  export let exportHtml: () => void;
  export let exportPdf: () => void;
  export let softwareUpdateState: SoftwareUpdateSnapshot;
  export let openSoftwareUpdate: () => void;
  export let openContextMenu: (request: ContextMenuRequest) => void = () => undefined;

  const WINDOW_STATE_SYNC_DELAY_MS = 80;
  const MENU_VIEWPORT_MARGIN_PX = 8;

  let platformCapabilities = getPlatformCapabilities();
  let isFullscreen = false;
  let unlistenResized: (() => void) | null = null;
  let windowStateSyncTimer: number | null = null;
  let windowStateRequestId = 0;
  let canSyncWindowState = false;
  let windowStateListenerReady = false;

  // Windows 高 DPI 会压缩 WebView 的逻辑视口；菜单打开或尺寸变化时始终贴合当前视口。
  function keepDropdownInViewport(node: HTMLElement) {
    let frameId: number | null = null;
    const nestedTrigger = node.closest<HTMLElement>('.nested-trigger');

    function fitDropdown() {
      frameId = null;
      node.classList.remove('opens-left', 'viewport-scroll');
      node.style.setProperty('--dropdown-shift-x', '0px');
      node.style.setProperty('--dropdown-shift-y', '0px');
      node.style.removeProperty('--dropdown-max-height');

      let rect = node.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;

      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      const availableHeight = Math.max(0, viewportHeight - MENU_VIEWPORT_MARGIN_PX * 2);

      if (rect.height > availableHeight) {
        node.classList.add('viewport-scroll');
        node.style.setProperty('--dropdown-max-height', `${availableHeight}px`);
        rect = node.getBoundingClientRect();
      }

      if (node.classList.contains('nested') && nestedTrigger) {
        const triggerRect = nestedTrigger.getBoundingClientRect();
        const rightSpace = viewportWidth - MENU_VIEWPORT_MARGIN_PX - triggerRect.right;
        const leftSpace = triggerRect.left - MENU_VIEWPORT_MARGIN_PX;
        node.classList.toggle('opens-left', rightSpace < rect.width + 2 && leftSpace > rightSpace);
        rect = node.getBoundingClientRect();
      }

      let shiftX = 0;
      if (rect.right > viewportWidth - MENU_VIEWPORT_MARGIN_PX) {
        shiftX = viewportWidth - MENU_VIEWPORT_MARGIN_PX - rect.right;
      }
      if (rect.left + shiftX < MENU_VIEWPORT_MARGIN_PX) {
        shiftX += MENU_VIEWPORT_MARGIN_PX - (rect.left + shiftX);
      }

      let shiftY = 0;
      if (rect.bottom > viewportHeight - MENU_VIEWPORT_MARGIN_PX) {
        shiftY = viewportHeight - MENU_VIEWPORT_MARGIN_PX - rect.bottom;
      }
      if (rect.top + shiftY < MENU_VIEWPORT_MARGIN_PX) {
        shiftY += MENU_VIEWPORT_MARGIN_PX - (rect.top + shiftY);
      }

      node.style.setProperty('--dropdown-shift-x', `${shiftX}px`);
      node.style.setProperty('--dropdown-shift-y', `${shiftY}px`);
    }

    function scheduleFit() {
      if (frameId !== null) window.cancelAnimationFrame(frameId);
      frameId = window.requestAnimationFrame(fitDropdown);
    }

    const resizeObserver =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleFit);
    resizeObserver?.observe(node);
    window.addEventListener('resize', scheduleFit);
    nestedTrigger?.addEventListener('pointerenter', scheduleFit);
    scheduleFit();

    return {
      destroy() {
        if (frameId !== null) window.cancelAnimationFrame(frameId);
        resizeObserver?.disconnect();
        window.removeEventListener('resize', scheduleFit);
        nestedTrigger?.removeEventListener('pointerenter', scheduleFit);
      },
    };
  }

  $: shouldShowWindowMenu = platformCapabilities.showsInAppWindowMenu;

  async function syncWindowState() {
    if (!desktopEnabled || !canSyncWindowState || !platformCapabilities.isMac) {
      return;
    }

    const requestId = ++windowStateRequestId;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const appWindow = getCurrentWindow();
      const fullscreen = await appWindow.isFullscreen();

      if (canSyncWindowState && platformCapabilities.isMac && requestId === windowStateRequestId) {
        isFullscreen = fullscreen;
      }
    } catch {
      // ignore
    }
  }

  function scheduleWindowStateSync() {
    if (!canSyncWindowState || !platformCapabilities.isMac) return;
    if (windowStateSyncTimer !== null) {
      window.clearTimeout(windowStateSyncTimer);
    }
    windowStateSyncTimer = window.setTimeout(() => {
      windowStateSyncTimer = null;
      void syncWindowState();
    }, WINDOW_STATE_SYNC_DELAY_MS);
  }

  async function setupWindowStateListener() {
    if (
      !desktopEnabled ||
      !canSyncWindowState ||
      !platformCapabilities.isMac ||
      windowStateListenerReady
    ) {
      return;
    }

    windowStateListenerReady = true;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      if (!canSyncWindowState) return;

      const appWindow = getCurrentWindow();
      const unlisten = await appWindow.onResized(scheduleWindowStateSync);

      if (canSyncWindowState) {
        unlistenResized = unlisten;
        await syncWindowState();
      } else {
        unlisten();
      }
    } catch {
      windowStateListenerReady = false;
    }
  }

  $: void setupWindowStateListener();

  onMount(() => {
    platformCapabilities = getPlatformCapabilities();
    canSyncWindowState = true;
    void setupWindowStateListener();

    return () => {
      canSyncWindowState = false;
      windowStateRequestId += 1;
      if (windowStateSyncTimer !== null) {
        window.clearTimeout(windowStateSyncTimer);
        windowStateSyncTimer = null;
      }
      if (unlistenResized) {
        unlistenResized();
        unlistenResized = null;
      }
      windowStateListenerReady = false;
    };
  });

  async function handleDrag(e: MouseEvent) {
    if (!desktopEnabled || e.buttons !== 1) {
      return;
    }

    const target = e.target as HTMLElement;

    // 排除交互元素，避免影响按钮点击
    if (
      target.closest('button') ||
      target.closest('.titlebar-right') ||
      target.closest('.titlebar-menu')
    ) {
      return;
    }

    // 只在拖动区域触发
    if (!target.closest('[data-drag-region]')) {
      return;
    }

    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const appWindow = getCurrentWindow();

      if (e.detail === 2) {
        if (markdownMiniActive) return;
        // 双击最大化/还原
        await appWindow.toggleMaximize();
      } else if (e.detail === 1) {
        await appWindow.startDragging();
      }
    } catch {
      // ignore
    }
  }

  async function handleTitlebarContextMenu(event: MouseEvent) {
    event.preventDefault();
    const target = event.target as HTMLElement | null;
    if (
      !target ||
      target.closest('button, .titlebar-menu, .dropdown-menu, input, textarea, select') ||
      !desktopEnabled ||
      !platformCapabilities.usesCustomWindowsTitlebar ||
      !target.closest('[data-drag-region], [data-tauri-drag-region]')
    ) {
      return;
    }

    let maximized = false;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      maximized = await getCurrentWindow().isMaximized();
    } catch {
      // 查询失败时仍提供可用的窗口菜单，最大化项使用默认文案。
    }

    const items = markdownMiniActive
      ? [
          {
            label: t.minimizeWindow(),
            icon: 'minimize' as const,
            action: minimizeCurrentWindow,
          },
          {
            label: markdownMiniPinned ? t.markdownMiniUnpin() : t.markdownMiniPin(),
            icon: 'focus' as const,
            active: markdownMiniPinned,
            action: toggleMarkdownMiniPinned,
          },
          { label: '', separator: true },
          {
            label: t.markdownMiniReturn(),
            icon: 'restore' as const,
            action: toggleMarkdownMini,
          },
        ]
      : [
          {
            label: t.minimizeWindow(),
            icon: 'minimize' as const,
            action: minimizeCurrentWindow,
          },
          {
            label: maximized ? t.restoreWindow() : t.maximizeWindow(),
            icon: maximized ? ('restore' as const) : ('maximize' as const),
            action: toggleCurrentWindowMaximized,
          },
          { label: '', separator: true },
          {
            label: t.closeWindow(),
            icon: 'close' as const,
            danger: true,
            action: closeCurrentWindow,
          },
        ];

    openContextMenu({ x: event.clientX, y: event.clientY, items });
  }

  async function minimizeCurrentWindow() {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().minimize();
    } catch {
      // 窗口状态可能在菜单打开后变化，失败时保持当前窗口状态。
    }
  }

  async function toggleCurrentWindowMaximized() {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().toggleMaximize();
    } catch {
      // 窗口状态可能在菜单打开后变化，失败时保持当前窗口状态。
    }
  }

  function finish(action: () => void, menu: string) {
    action();
    closeMenu(menu);
  }

  function comingSoon(featureName: string, menu: string) {
    showUnavailableFeature(featureName);
    closeMenu(menu);
  }

  function insertBlankDiagram(menu: string) {
    runCommand({ type: 'insertMermaidBlock' });
    closeMenu(menu);
  }

  function insertDiagram(diagramType: DiagramType, menu: string) {
    runCommand({ type: 'insertDiagramBlock', diagramType });
    closeMenu(menu);
  }
</script>

{#key interfaceLocale}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header
    class="titlebar"
    data-interface-locale={interfaceLocale}
    class:is-mac={platformCapabilities.isMac}
    class:is-win={platformCapabilities.isWindows}
    class:is-fullscreen={isFullscreen}
    on:contextmenu={handleTitlebarContextMenu}
  >
    {#if markdownMiniActive}
      <div
        class="titlebar-row top-row markdown-mini-titlebar-row"
        data-tauri-drag-region="deep"
        role="toolbar"
        tabindex="0"
        aria-label={t.markdownMiniWindow()}
      >
        <div class="markdown-mini-identity">
          <span class="markdown-mini-file-name" title={filePath}>{fileName}</span>
          {#if dirty}
            <span class="markdown-mini-dot is-dirty" title={t.unsavedChanges()}></span>
          {/if}
          {#if markdownMiniExternalChanged}
            <span class="markdown-mini-dot is-external" title={t.markdownMiniExternalChanged()}
            ></span>
          {/if}
          {#if largeDocumentMode}
            <span class="markdown-mini-readonly" title={t.markdownMiniLargeReadonly()}>
              <LockKeyhole size={12} />
            </span>
          {/if}
        </div>
        <span class="titlebar-spacer"></span>
        <div class="markdown-mini-actions">
          <button
            type="button"
            class:active={markdownMiniPinned}
            title={markdownMiniPinned ? t.markdownMiniUnpin() : t.markdownMiniPin()}
            aria-label={markdownMiniPinned ? t.markdownMiniUnpin() : t.markdownMiniPin()}
            aria-pressed={markdownMiniPinned}
            on:click={toggleMarkdownMiniPinned}
          >
            {#if markdownMiniPinned}<Pin size={14} />{:else}<PinOff size={14} />{/if}
          </button>
          {#if !(desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar)}
            <button
              type="button"
              class="return-button"
              title={`${t.markdownMiniReturn()} (${markdownMiniShortcut})`}
              aria-label={t.markdownMiniReturn()}
              on:click={toggleMarkdownMini}
            >
              <ArrowDownLeft size={15} />
            </button>
          {/if}
        </div>
        {#if desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar}
          <WindowsCaptionControls variant="return" onClose={toggleMarkdownMini} />
        {/if}
      </div>
    {/if}
    <div
      class="titlebar-row top-row"
      data-drag-region
      role="presentation"
      on:mousedown={handleDrag}
    >
      <button
        class="icon-btn sidebar-toggle-btn"
        title={focusMode ? t.showExplorerSidebar() : t.hideExplorerSidebar()}
        aria-label={focusMode ? t.showExplorerSidebar() : t.hideExplorerSidebar()}
        aria-pressed={!focusMode}
        on:click={toggleFocusMode}
      >
        {#if focusMode}
          <PanelLeftOpen size={16} />
        {:else}
          <PanelLeftClose size={16} />
        {/if}
      </button>

      {#if shouldShowWindowMenu}
        <div class="titlebar-left" data-drag-region>
          <span class="app-name" data-drag-region>Nomo</span>
        </div>

        <nav class="titlebar-menu">
          <div
            class="menu-item"
            class:active={activeMenu === 'file'}
            use:clickOutside={() => closeMenu('file')}
          >
            <button class="menu-btn" on:click|stopPropagation={() => toggleMenu('file')}
              >{t.file()}</button
            >
            {#if activeMenu === 'file'}
              <div class="dropdown-menu" use:keepDropdownInViewport>
                <button on:click={() => finish(createNewFile, 'file')}
                  >{t.newMarkdown()} <span class="shortcut">Ctrl + N</span></button
                >
                <button on:click={() => finish(createNewWindow, 'file')}
                  >{t.newWindow()} <span class="shortcut">Ctrl + Shift + N</span></button
                >
                <div class="divider"></div>
                <button on:click={() => finish(openFileDialog, 'file')}
                  >{t.openFileEllipsis()} <span class="shortcut">Ctrl + O</span></button
                >
                <button on:click={() => finish(openFolderDialog, 'file')}
                  >{t.openFolderEllipsis()} <span class="shortcut">Ctrl + Shift + O</span></button
                >

                <div class="nested-trigger">
                  <span>{t.openRecent()}</span>
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor"
                    ><path
                      d="M3 1l4 4-4 4"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    /></svg
                  >
                  <div class="dropdown-menu nested recent-submenu" use:keepDropdownInViewport>
                    {#each recentFiles.slice(0, 10) as recent}
                      {@const isMissing = missingRecentPaths.has(recent.path)}
                      <div class="recent-menu-row">
                        <button
                          class="recent-entry"
                          class:recent-folder={recent.entryType === 'folder'}
                          class:recent-missing={isMissing}
                          disabled={isMissing}
                          title={isMissing
                            ? `${recent.path} (${t.pathInvalidRemove()})`
                            : recent.path}
                          on:click={() =>
                            isMissing
                              ? finish(() => removeRecentEntry(recent.path), 'file')
                              : finish(
                                  () => openRecentEntry(recent.path, recent.entryType),
                                  'file',
                                )}
                        >
                          <span class="recent-icon">
                            {#if recent.entryType === 'folder'}
                              📁
                            {:else}
                              📄
                            {/if}
                          </span>
                          <span class="recent-label" data-missing-label={t.unavailableSuffix()}>
                            {recent.title ?? getCompactPath(recent.path)}
                          </span>
                        </button>
                        <button
                          type="button"
                          class="recent-remove"
                          aria-label={`${t.removeRecentLabel()} ${recent.path}`}
                          title={t.removeRecentLabel()}
                          on:click={() => removeRecentEntry(recent.path)}>×</button
                        >
                      </div>
                    {/each}
                    {#if recentFiles.length === 0}
                      <span class="disabled-item">{t.noRecentFiles()}</span>
                    {/if}
                    {#if recentFiles.length > 0}
                      <div class="divider"></div>
                      <button
                        class="recent-clear"
                        on:click={() => finish(clearRecentEntriesList, 'file')}
                      >
                        {t.clearRecentFiles()}
                      </button>
                    {/if}
                  </div>
                </div>

                <div class="divider"></div>
                <button on:click={() => finish(() => saveMarkdownFile(), 'file')}
                  >{t.save()} <span class="shortcut">Ctrl + S</span></button
                >
                <button on:click={() => finish(() => saveMarkdownFile(true), 'file')}
                  >{t.saveAs()} <span class="shortcut">Ctrl + Shift + S</span></button
                >
                <div class="divider"></div>
                <button on:click={() => finish(exportHtml, 'file')}>{t.exportHtml()}</button>
                <button on:click={() => finish(exportPdf, 'file')}>{t.exportPdf()}</button>
                <div class="divider"></div>
                <button on:click={() => finish(closeCurrentFile, 'file')}
                  >{t.closeCurrentFile()} <span class="shortcut">Ctrl + W</span></button
                >
                <button on:click={() => finish(closeCurrentWindow, 'file')}
                  >{t.closeWindow()} <span class="shortcut">Alt + F4</span></button
                >
                <div class="divider"></div>
                <button on:click={() => finish(exitApp, 'file')}>{t.quit()}</button>
              </div>
            {/if}
          </div>

          <div
            class="menu-item"
            class:active={activeMenu === 'edit'}
            use:clickOutside={() => closeMenu('edit')}
          >
            <button class="menu-btn" on:click|stopPropagation={() => toggleMenu('edit')}
              >{t.editMenu()}</button
            >
            {#if activeMenu === 'edit'}
              <div class="dropdown-menu" use:keepDropdownInViewport>
                <button on:click={() => finish(() => runCommand({ type: 'undo' }), 'edit')}
                  >{t.undo()} <span class="shortcut">Ctrl + Z</span></button
                >
                <button on:click={() => finish(() => runCommand({ type: 'redo' }), 'edit')}
                  >{t.redo()} <span class="shortcut">Ctrl + Y</span></button
                >
              </div>
            {/if}
          </div>

          <div
            class="menu-item"
            class:active={activeMenu === 'paragraph'}
            use:clickOutside={() => closeMenu('paragraph')}
          >
            <button class="menu-btn" on:click|stopPropagation={() => toggleMenu('paragraph')}
              >{t.paragraph()}</button
            >
            {#if activeMenu === 'paragraph'}
              <div class="dropdown-menu" use:keepDropdownInViewport>
                <div class="nested-trigger">
                  <span>{t.heading()}</span>
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor"
                    ><path
                      d="M3 1l4 4-4 4"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    /></svg
                  >
                  <div class="dropdown-menu nested" use:keepDropdownInViewport>
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 1 }), 'paragraph')}
                      >{t.heading1()} <span class="shortcut">Ctrl + 1</span></button
                    >
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 2 }), 'paragraph')}
                      >{t.heading2()} <span class="shortcut">Ctrl + 2</span></button
                    >
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 3 }), 'paragraph')}
                      >{t.heading3()} <span class="shortcut">Ctrl + 3</span></button
                    >
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 4 }), 'paragraph')}
                      >{t.heading4()} <span class="shortcut">Ctrl + 4</span></button
                    >
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 5 }), 'paragraph')}
                      >{t.heading5()} <span class="shortcut">Ctrl + 5</span></button
                    >
                    <button
                      on:mousedown|preventDefault
                      on:click={() =>
                        finish(() => runCommand({ type: 'setHeading', level: 6 }), 'paragraph')}
                      >{t.heading6()} <span class="shortcut">Ctrl + 6</span></button
                    >
                  </div>
                </div>
                <button
                  on:mousedown|preventDefault
                  on:click={() => finish(() => runCommand({ type: 'setParagraph' }), 'paragraph')}
                  >{t.paragraph()} <span class="shortcut">Ctrl + 0</span></button
                >
                <button
                  on:mousedown|preventDefault
                  on:click={() =>
                    finish(() => runCommand({ type: 'increaseHeadingLevel' }), 'paragraph')}
                  >{t.liftHeading()} <span class="shortcut">Ctrl + =</span></button
                >
                <button
                  on:mousedown|preventDefault
                  on:click={() =>
                    finish(() => runCommand({ type: 'decreaseHeadingLevel' }), 'paragraph')}
                  >{t.sinkHeading()} <span class="shortcut">Ctrl + -</span></button
                >
                <div class="divider"></div>
                <button on:click={() => finish(openTablePicker, 'paragraph')}
                  >{t.table()} <span class="shortcut">Ctrl + Shift + T</span></button
                >
                <button
                  on:click={() =>
                    finish(
                      () => runCommand({ type: 'insertCodeBlock', language: 'ts' }),
                      'paragraph',
                    )}>{t.codeBlock()} <span class="shortcut">Ctrl + Shift + K</span></button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'insertMathBlock', tex: '' }), 'paragraph')}
                  >{t.insertMathBlock()} <span class="shortcut">Ctrl + Shift + M</span></button
                >
                <div class="divider"></div>
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'toggleBlockquote' }), 'paragraph')}
                  >{t.quote()} <span class="shortcut">Ctrl + Shift + Q</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'insertCallout' }), 'paragraph')}
                  >{t.callout()} <span class="shortcut">Ctrl + Shift + A</span></button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'insertCommentBlock' }), 'paragraph')}
                  >{t.inlineComment()}</button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'toggleOrderedList' }), 'paragraph')}
                  >{t.orderedList()} <span class="shortcut">Ctrl + Shift + [</span></button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'toggleBulletList' }), 'paragraph')}
                  >{t.unorderedList()} <span class="shortcut">Ctrl + Shift + ]</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'toggleTaskList' }), 'paragraph')}
                  >{t.taskList()} <span class="shortcut">Ctrl + Shift + X</span></button
                >
                <div class="divider"></div>
                <button on:click={() => comingSoon(t.insertParagraphBefore(), 'paragraph')}
                  >{t.paragraphBefore()} <span class="shortcut">Ctrl + Shift + Enter</span></button
                >
                <button on:click={() => comingSoon(t.insertParagraphAfter(), 'paragraph')}
                  >{t.paragraphAfter()} <span class="shortcut">Ctrl + Enter</span></button
                >
                <div class="divider"></div>
                <div class="nested-trigger">
                  <span>{t.diagram()}</span>
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor"
                    ><path
                      d="M3 1l4 4-4 4"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    /></svg
                  >
                  <div class="dropdown-menu nested" use:keepDropdownInViewport>
                    <button on:click={() => insertBlankDiagram('paragraph')}>
                      {t.blankDiagram()} <span class="shortcut">mermaid</span>
                    </button>
                    <div class="divider"></div>
                    {#each DIAGRAM_TEMPLATES as template}
                      <button on:click={() => insertDiagram(template.type, 'paragraph')}>
                        {getDiagramTypeLabel(template.type)}
                        <span class="shortcut">{template.type}</span>
                      </button>
                    {/each}
                  </div>
                </div>
                <button
                  on:click={() => finish(() => runCommand({ type: 'insertFootnote' }), 'paragraph')}
                  >{t.footnote()}</button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'insertHorizontalRule' }), 'paragraph')}
                  >{t.horizontalRule()} <span class="shortcut">Ctrl + Shift + H</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'insertToc' }), 'paragraph')}
                  >{t.toc()}</button
                >
                <button on:click={() => finish(editFrontMatter, 'paragraph')}
                  >{t.frontMatter()}</button
                >
              </div>
            {/if}
          </div>

          <div
            class="menu-item"
            class:active={activeMenu === 'format'}
            use:clickOutside={() => closeMenu('format')}
          >
            <button class="menu-btn" on:click|stopPropagation={() => toggleMenu('format')}
              >{t.format()}</button
            >
            {#if activeMenu === 'format'}
              <div class="dropdown-menu" use:keepDropdownInViewport>
                <button on:click={() => finish(() => runCommand({ type: 'toggleBold' }), 'format')}
                  >{t.bold()} <span class="shortcut">Ctrl + B</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'toggleItalic' }), 'format')}
                  >{t.italic()} <span class="shortcut">Ctrl + I</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'toggleUnderline' }), 'format')}
                  >{t.underline()} <span class="shortcut">Ctrl + U</span></button
                >
                <button on:click={() => finish(() => runCommand({ type: 'toggleCode' }), 'format')}
                  >{t.inlineCode()} <span class="shortcut">Ctrl + `</span></button
                >
                <button on:click={() => comingSoon(t.inlineMath(), 'format')}
                  >{t.inlineMath()}</button
                >
                <div class="divider"></div>
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'toggleStrikethrough' }), 'format')}
                  >{t.strikethrough()} <span class="shortcut">Alt + Shift + 5</span></button
                >
                <button
                  on:click={() => finish(() => runCommand({ type: 'toggleHighlight' }), 'format')}
                  >{t.highlight()}</button
                >
                <button
                  on:click={() =>
                    finish(() => runCommand({ type: 'insertCommentInline' }), 'format')}
                  >{t.inlineComment()}</button
                >
                <div class="divider"></div>
                <button on:click={() => finish(openLinkPicker, 'format')}
                  >{t.link()} <span class="shortcut">Ctrl + K</span></button
                >
                <button on:click={() => comingSoon(t.imageMenu(), 'format')}>{t.imageMenu()}</button
                >
                <div class="divider"></div>
                <button
                  on:click={() => finish(() => runCommand({ type: 'clearInlineStyles' }), 'format')}
                  >{t.clearStyle()} <span class="shortcut">Ctrl + \</span></button
                >
              </div>
            {/if}
          </div>

          <div
            class="menu-item"
            class:active={activeMenu === 'view'}
            use:clickOutside={() => closeMenu('view')}
          >
            <button class="menu-btn" on:click|stopPropagation={() => toggleMenu('view')}
              >{t.view()}</button
            >
            {#if activeMenu === 'view'}
              <div class="dropdown-menu" use:keepDropdownInViewport>
                <button
                  role="menuitemradio"
                  aria-checked={mode === 'semantic'}
                  disabled={largeDocumentMode}
                  on:click={() => finish(() => setMode('semantic'), 'view')}
                  >{t.semanticEditing()}</button
                >
                <button
                  role="menuitemradio"
                  aria-checked={mode === 'source'}
                  on:click={() => finish(() => setMode('source'), 'view')}
                  >{t.sourceMode()} <span class="shortcut">Ctrl + E</span></button
                >
                <button
                  role="menuitemradio"
                  aria-checked={mode === 'split'}
                  disabled={largeDocumentMode}
                  on:click={() => finish(() => setMode('split'), 'view')}>{t.splitMode()}</button
                >
                <button on:click={() => finish(toggleOutlineVisible, 'view')}
                  >{outlineVisible ? t.hideOutline() : t.showOutline()}</button
                >
                <button on:click={() => finish(toggleToolbar, 'view')}>
                  {toolbarHidden ? t.showToolbar() : t.hideToolbar()}
                  <span class="shortcut">{toolbarShortcut.split('+').join(' + ')}</span>
                </button>
                <button on:click={() => finish(toggleTheme, 'view')}
                  >{t.switchTheme()} <span class="shortcut">Ctrl + Shift + L</span></button
                >
                <button on:click={() => finish(toggleFocusMode, 'view')}
                  >{t.showHideExplorer()} <span class="shortcut">Ctrl + Shift + F</span></button
                >
              </div>
            {/if}
          </div>

          <div class="menu-item">
            <button
              class="menu-btn"
              on:click|stopPropagation={() => finish(openSettings, 'settings')}
              >{t.settings()}</button
            >
          </div>
        </nav>
      {/if}

      <span class="titlebar-spacer" data-drag-region></span>
      <div class="titlebar-right">
        {#if ['available', 'downloading', 'downloaded'].includes(softwareUpdateState.status)}
          <button
            class="icon-btn software-update-icon-btn"
            class:downloading={softwareUpdateState.status === 'downloading'}
            title={softwareUpdateState.status === 'downloaded'
              ? t.softwareUpdateWaitingInstall()
              : t.softwareUpdateNoticeTitle()}
            aria-label={softwareUpdateState.status === 'downloaded'
              ? t.softwareUpdateWaitingInstall()
              : t.softwareUpdateNoticeTitle()}
            on:click={openSoftwareUpdate}
          >
            <Download size={15} />
            {#if softwareUpdateState.status === 'available'}
              <span class="software-update-dot" aria-hidden="true"></span>
            {:else if softwareUpdateState.status === 'downloading'}
              <span class="software-update-progress" aria-hidden="true">
                {softwareUpdateState.progress?.percent ?? 0}
              </span>
            {:else}
              <span class="software-update-ready-dot" aria-hidden="true"></span>
            {/if}
          </button>
        {/if}

        {#if markdownMiniAvailable}
          <button
            class="icon-btn markdown-mini-icon-btn"
            class:active={markdownMiniActive}
            title={`${t.markdownMiniShortcut()} (${markdownMiniShortcut})`}
            aria-label={t.markdownMiniShortcut()}
            aria-pressed={markdownMiniActive}
            on:click={toggleMarkdownMini}
          >
            <PictureInPicture2 size={15} />
          </button>
        {/if}

        <button
          class="icon-btn theme-toggle-icon-btn"
          title={t.switchTheme()}
          aria-label={t.switchTheme()}
          on:click={toggleTheme}
        >
          {#if theme === 'light'}
            <Moon size={14} />
          {:else}
            <Sun size={14} />
          {/if}
        </button>
      </div>
      {#if desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar && !markdownMiniActive}
        <WindowsCaptionControls onClose={closeCurrentWindow} />
      {/if}
    </div>
  </header>
{/key}
