<script lang="ts">
  import { ChevronDown, FileJson2, FileText, FileType2, Plus, X } from '@lucide/svelte';
  import { createEventDispatcher, onMount, tick } from 'svelte';
  import {
    motionIn,
    pulseOnChange,
    tabIndicator,
    type TabIndicatorGeometry,
  } from '../actions/motion';
  import type {
    ContextMenuItem,
    ContextMenuRequest,
  } from '../../lib/editor-core/plugins/contextMenu';
  import type { Tab } from '../types';
  import { t } from '../i18n';

  export let interfaceLocale: string;
  export let tabs: Tab[];
  export let activeTabId: string;
  export let previewTabId: string | null = null;
  export let switchTab: (tabId: string) => void | Promise<void>;
  export let closeTab: (tabId: string, event?: Event) => void;
  export let pinPreviewTab: () => void;
  export let createNewFile: () => void;
  export let openFileDialog: () => void = () => undefined;
  export let openFolderDialog: () => void = () => undefined;
  export let openContextMenu: (request: ContextMenuRequest) => void = () => undefined;
  export let copyContextText: (text: string) => void | Promise<void> = () => undefined;
  export let revealContextPath: (path: string) => void | Promise<void> = () => undefined;
  export let currentFolderPath: string = '';
  export let appearanceKey = '';
  export let onIndicatorGeometry:
    | ((geometry: TabIndicatorGeometry | null) => void)
    | undefined = undefined;

  const dispatch = createEventDispatcher<{
    closeOtherTabs: { tabId: string };
    closeTabsToRight: { tabId: string };
    closeAllTabs: void;
  }>();

  const dropdownButtonWidth = 28;

  // 将绝对路径转为相对于当前工作目录的路径，用于 tooltip 显示
  function getRelativeDisplayPath(filePath: string, basePath: string): string {
    if (!basePath || !filePath) return filePath;
    const normalizedFile = filePath.replace(/\\/g, '/');
    const normalizedBase = basePath.replace(/\\/g, '/').replace(/\/+$/, '');
    if (normalizedFile.startsWith(normalizedBase + '/')) {
      return normalizedFile.slice(normalizedBase.length + 1);
    }
    return filePath;
  }
  let tabsContainer: HTMLDivElement;
  let measureArea: HTMLDivElement;
  let dropdownBtnEl: HTMLButtonElement;
  let showAddButton = true;
  let overflowState = false; // 是否有标签溢出（由测量函数控制）
  let showDropdown = false; // 下拉菜单是否展开（由用户点击控制）
  let hiddenTabs: Tab[] = [];
  let visibleRange = { start: 0, end: 0 };
  let resizeObserver: ResizeObserver | null = null;
  let measureQueued = false;
  let dropdownMenuStyle = '';
  let pendingActiveTabId: string | null = null;
  let visualActiveTabId = activeTabId;
  let tabMeasureKey = '';
  let activeTabIndex = -1;
  let activeTabOutsideVisibleRange = false;

  $: hiddenTabs = tabs.filter((_, i) => i < visibleRange.start || i >= visibleRange.end);

  // 构建标签右键菜单项
  function buildTabContextMenuItems(tab: Tab): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    const isPreview = previewTabId === tab.id;
    const tabIndex = tabs.findIndex((t) => t.id === tab.id);

    // 步骤1：基础关闭操作
    items.push({
      label: t.close(),
      icon: 'close',
      action: () => closeTab(tab.id),
      shortcut: isPreview ? undefined : 'Ctrl+W',
    });

    // 步骤2：批量关闭操作
    const otherTabs = tabs.filter((t) => t.id !== tab.id);
    if (otherTabs.length > 0) {
      items.push({
        label: t.closeOtherTabs(),
        icon: 'close',
        action: () => dispatch('closeOtherTabs', { tabId: tab.id }),
      });
    }

    const rightTabs = tabs.slice(tabIndex + 1);
    if (rightTabs.length > 0) {
      items.push({
        label: t.closeTabsToRight(),
        icon: 'close',
        action: () => dispatch('closeTabsToRight', { tabId: tab.id }),
      });
    }

    items.push({
      label: t.closeAllTabs(),
      icon: 'close',
      action: () => dispatch('closeAllTabs'),
      danger: true,
    });

    // 步骤3：路径相关操作
    const path = tab.nativePath || tab.filePath;
    if (path) {
      items.push({ label: '', action: () => {}, separator: true });
      items.push({
        label: t.copyPath(),
        icon: 'copy',
        action: () => copyContextText(path),
      });
      items.push({
        label: t.revealInFolder(),
        icon: 'folder',
        action: () => revealContextPath(path),
      });
    }

    return items;
  }

  function handleTabContextMenu(tab: Tab, event: MouseEvent) {
    event.preventDefault();
    openContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: buildTabContextMenuItems(tab),
    });
  }

  function handleTabStripContextMenu(event: MouseEvent) {
    if (event.defaultPrevented) return;
    const target = event.target as HTMLElement | null;
    event.preventDefault();
    if (!target || target.closest('button')) return;
    openContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: [
        { label: t.newFile(), icon: 'new-file', action: createNewFile },
        { label: t.openFileEllipsis(), icon: 'open', action: openFileDialog },
        { label: t.openFolderEllipsis(), icon: 'folder', action: openFolderDialog },
        { label: '', separator: true },
        {
          label: t.closeAllTabs(),
          icon: 'close',
          danger: true,
          action: () => dispatch('closeAllTabs'),
        },
      ],
    });
  }

  function queueMeasureTabs() {
    if (measureQueued) return;
    measureQueued = true;

    requestAnimationFrame(() => {
      measureQueued = false;
      measureAndComputeVisible();
    });
  }

  // 测量所有标签宽度并计算可见范围
  // 策略：从右到左依次计算能放下的标签（优先显示新标签），如果激活标签不在范围内则平移窗口
  function measureAndComputeVisible() {
    if (!tabsContainer || !measureArea) return;

    if (tabs.length === 0) {
      if (visibleRange.start !== 0 || visibleRange.end !== 0) {
        visibleRange = { start: 0, end: 0 };
      }
      showAddButton = true;
      overflowState = false;
      return;
    }

    const measured = measureArea.querySelectorAll<HTMLElement>('.doc-tab');
    const widths: number[] = [];
    measured.forEach((el) => widths.push(el.getBoundingClientRect().width));

    const available = tabsContainer.clientWidth;
    const reserveWidth = dropdownButtonWidth + 4;

    // 从右到左累加，优先显示右边的（最新的）标签
    let used = 0;
    let start = tabs.length;
    for (let i = tabs.length - 1; i >= 0; i--) {
      const w = widths[i] ?? 120;
      if (used + w > available - reserveWidth && start < tabs.length) break;
      used += w;
      start = i;
    }

    let end = tabs.length;
    // 如果激活标签不在可见范围内，平移窗口使其包含激活标签
    const activeIdx = tabs.findIndex((t) => t.id === activeTabId);
    if (activeIdx >= 0 && (activeIdx < start || activeIdx >= end)) {
      end = activeIdx + 1;
      used = 0;
      start = end;
      for (let i = end - 1; i >= 0; i--) {
        const w = widths[i] ?? 120;
        if (used + w > available - reserveWidth && start < end - 1) break;
        used += w;
        start = i;
      }
      // 在包含激活标签后，尽量向右扩展以充分利用空间
      used = 0;
      for (let i = start; i < end; i++) used += widths[i] ?? 120;
      while (end < tabs.length) {
        const nextW = widths[end] ?? 120;
        if (used + nextW > available - reserveWidth) break;
        used += nextW;
        end++;
      }
    }

    const nextStart = Math.min(start, Math.max(0, end - 1));
    const nextEnd = Math.max(start + 1, end);
    if (visibleRange.start !== nextStart || visibleRange.end !== nextEnd) {
      visibleRange = { start: nextStart, end: nextEnd };
    }

    // 根据溢出状态更新按钮显示，不干预用户手动打开的下拉菜单
    const isOverflowing = !(nextStart === 0 && nextEnd === tabs.length);
    if (isOverflowing) {
      showAddButton = false;
      overflowState = true;
    } else {
      showAddButton = true;
      overflowState = false;
      showDropdown = false;
    }
  }

  function updateDropdownPosition() {
    if (!dropdownBtnEl || !showDropdown) return;
    const rect = dropdownBtnEl.getBoundingClientRect();
    dropdownMenuStyle = `position:fixed;top:${rect.bottom + 4}px;left:${rect.left - 172 + rect.width}px;z-index:9999;`;
  }

  function toggleDropdown(event: Event) {
    event.stopPropagation();
    showDropdown = !showDropdown;
    if (showDropdown) {
      void tick().then(updateDropdownPosition);
    }
  }

  async function requestTabSwitch(tabId: string) {
    if (!tabId || tabId === activeTabId || pendingActiveTabId !== null) return;

    pendingActiveTabId = tabId;
    try {
      await switchTab(tabId);
    } finally {
      if (activeTabId !== tabId) {
        pendingActiveTabId = null;
      }
    }
  }

  function selectHiddenTab(tabId: string) {
    showDropdown = false;
    void requestTabSwitch(tabId);
  }

  // 点击外部关闭下拉菜单
  function handleWindowClick(event: MouseEvent) {
    if (!showDropdown) return;
    const target = event.target as Node;
    // 通过 portal 渲染的下拉菜单不在原 DOM 位置，需要额外判断
    const menuEl = document.querySelector('.tab-dropdown-menu-portal');
    const btnEl = dropdownBtnEl;
    if (btnEl && (btnEl === target || btnEl.contains(target))) return;
    if (menuEl && (menuEl === target || menuEl.contains(target))) return;
    showDropdown = false;
  }

  $: if (pendingActiveTabId === activeTabId) {
    pendingActiveTabId = null;
  }
  $: visualActiveTabId = pendingActiveTabId ?? activeTabId;
  $: tabMeasureKey = `${previewTabId ?? ''}|${tabs
    .map((tab) => `${tab.id}:${tab.documentKind}:${tab.fileName}:${tab.dirty ? '1' : '0'}`)
    .join('|')}`;
  $: activeTabIndex = tabs.findIndex((tab) => tab.id === activeTabId);
  $: activeTabOutsideVisibleRange =
    activeTabIndex >= 0 &&
    (activeTabIndex < visibleRange.start || activeTabIndex >= visibleRange.end);
  $: {
    tabMeasureKey;
    void tick().then(queueMeasureTabs);
  }
  $: if (activeTabOutsideVisibleRange) {
    void tick().then(queueMeasureTabs);
  }

  onMount(() => {
    resizeObserver = new ResizeObserver(queueMeasureTabs);
    resizeObserver.observe(tabsContainer);
    queueMeasureTabs();

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
    };
  });
</script>

<svelte:window on:click={handleWindowClick} />

{#key interfaceLocale}
  <!-- 隐藏的测量区域：渲染所有标签以测量宽度 -->
  <div class="tab-measure-area" bind:this={measureArea}>
    {#each tabs as tab (tab.id)}
      <button
        type="button"
        class="doc-tab"
        class:preview={previewTabId === tab.id}
        title={getRelativeDisplayPath(tab.filePath, currentFolderPath)}
      >
        {#if tab.documentKind === 'json'}
          <FileJson2 size={13} />
        {:else if tab.documentKind === 'text'}
          <FileType2 size={13} />
        {:else}
          <FileText size={13} />
        {/if}
        <span class="tab-title">{tab.fileName}</span>
        {#if tab.dirty}
          <span class="dirty-indicator"></span>
        {/if}
        <span class="close-tab-btn"><X size={12} /></span>
      </button>
    {/each}
  </div>

  <header
    class="topbar"
    role="navigation"
    aria-label={t.documentTabs()}
    data-interface-locale={interfaceLocale}
    on:contextmenu={handleTabStripContextMenu}
  >
    <div
      class="tabs-container"
      bind:this={tabsContainer}
      data-indicator-ready="false"
      use:tabIndicator={{
        activeTabId: visualActiveTabId,
        visibleStart: visibleRange.start,
        visibleEnd: visibleRange.end,
        layoutKey: `${tabMeasureKey}|${appearanceKey}`,
        onGeometry: onIndicatorGeometry,
      }}
    >
      <span class="tab-active-indicator" aria-hidden="true"></span>
      {#each tabs.slice(visibleRange.start, visibleRange.end) as tab (tab.id)}
        <button
          type="button"
          class="doc-tab"
          class:active={visualActiveTabId === tab.id}
          class:preview={previewTabId === tab.id}
          title={getRelativeDisplayPath(tab.filePath, currentFolderPath)}
          use:motionIn={{ kind: 'row', y: 5 }}
          on:click={() => void requestTabSwitch(tab.id)}
          on:dblclick={() => {
            if (previewTabId === tab.id) pinPreviewTab();
          }}
          on:contextmenu={(event) => handleTabContextMenu(tab, event)}
        >
          {#if tab.documentKind === 'json'}
            <FileJson2 size={13} />
          {:else if tab.documentKind === 'text'}
            <FileType2 size={13} />
          {:else}
            <FileText size={13} />
          {/if}
          <span class="tab-title">{tab.fileName}</span>
          {#if tab.dirty}
            <span
              class="dirty-indicator"
              title={t.unsavedChanges()}
              use:motionIn={{ kind: 'micro', y: 0, scale: 0.8 }}
              use:pulseOnChange={tab.dirty}
            ></span>
          {/if}
          <span
            class="close-tab-btn"
            role="button"
            tabindex="0"
            title={t.closeTab()}
            on:click|stopPropagation={(event) => closeTab(tab.id, event)}
            on:keydown|stopPropagation={(event) => {
              if (event.key === 'Enter') closeTab(tab.id, event);
            }}
          >
            <X size={12} />
          </span>
        </button>
      {/each}
      {#if overflowState}
        <div class="tab-overflow-dropdown">
          <button
            type="button"
            class="tab-dropdown-btn"
            title={t.showHiddenTabs()}
            aria-label={t.showHiddenTabs()}
            bind:this={dropdownBtnEl}
            on:click={toggleDropdown}
          >
            <ChevronDown size={14} />
          </button>
        </div>
      {/if}
      {#if showAddButton}
        <button
          type="button"
          class="tab-add"
          title={t.newFile()}
          aria-label={t.newFile()}
          on:click={createNewFile}
        >
          <Plus size={16} />
        </button>
      {/if}
    </div>
  </header>

  <!-- Portal 下拉菜单：渲染到 body 避免被 overflow:hidden 裁剪 -->
  {#if showDropdown && hiddenTabs.length > 0}
    <div class="tab-dropdown-menu tab-dropdown-menu-portal" style={dropdownMenuStyle} role="menu">
      {#each hiddenTabs as tab (tab.id)}
        <button
          type="button"
          class="tab-dropdown-item"
          class:active={activeTabId === tab.id}
          class:preview={previewTabId === tab.id}
          role="menuitem"
          title={getRelativeDisplayPath(tab.filePath, currentFolderPath)}
          use:motionIn={{ kind: 'row', y: -3 }}
          on:click={() => selectHiddenTab(tab.id)}
          on:contextmenu={(event) => handleTabContextMenu(tab, event)}
        >
          {#if tab.documentKind === 'json'}
            <FileJson2 size={13} />
          {:else if tab.documentKind === 'text'}
            <FileType2 size={13} />
          {:else}
            <FileText size={13} />
          {/if}
          <span class="tab-dropdown-item-name">{tab.fileName}</span>
          {#if tab.dirty}
            <span class="dirty-indicator" title={t.unsavedChanges()}></span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}

{/key}
