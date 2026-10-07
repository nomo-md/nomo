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
  import type { Tab, WorkspaceItem } from '../types';
  import {
    createSingleWorkspaceItem,
    findWorkspaceItemForTab,
    getWorkspaceItemFocusedTabId,
    getWorkspaceItemTabIds,
  } from '../services/workspaceItems';
  import type { NativeTabDragEvent, TabDropZone } from '../services/tabTransfer';
  import { t } from '../i18n';

  export let interfaceLocale: string;
  export let tabs: Tab[];
  export let activeTabId: string;
  export let workspaceItems: WorkspaceItem[] = [];
  export let activeItemId = '';
  export let selectWorkspaceItem: ((itemId: string) => void | Promise<void>) | undefined = undefined;
  export let closeWorkspaceItem: ((itemId: string, event?: Event) => void | Promise<void>) | undefined = undefined;
  export let getWorkspaceItemActions: (itemId: string) => ContextMenuItem[] = () => [];
  export let onWorkspaceItemPointerDown: ((itemId: string, event: PointerEvent) => void) | undefined = undefined;
  export let onDropZones: (zones: TabDropZone[]) => void = () => undefined;
  export let dropHighlight: NativeTabDragEvent | null = null;
  export let workspaceInteractionDisabled = false;
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

  type DisplayTab = Tab & { workspaceItem: WorkspaceItem; preview: boolean; tooltip: string };
  let displayItems: WorkspaceItem[] = [];
  let displayTabs: DisplayTab[] = [];
  let displayActiveTabId = '';
  let dropInsertionLeft: number | null = null;

  function projectDisplayTabs(
    items: WorkspaceItem[], documents: Tab[], previewId: string | null, basePath: string,
  ): DisplayTab[] {
    return items.flatMap((item) => {
      const members = getWorkspaceItemTabIds(item).flatMap((id) => {
        const tab = documents.find((candidate) => candidate.id === id);
        return tab ? [tab] : [];
      });
      const tab = members[0];
      if (!tab || members.length !== getWorkspaceItemTabIds(item).length) return [];
      return [{
        ...tab,
        id: item.id,
        fileName: members.map((member) => member.fileName).join('｜'),
        dirty: members.some((member) => member.dirty),
        workspaceItem: item,
        preview: item.kind === 'single' && item.tabId === previewId,
        tooltip: members.map((member) =>
          getRelativeDisplayPath(member.nativePath || member.filePath, basePath)).join('\n'),
      }];
    });
  }

  $: displayItems = workspaceItems.length > 0 ? workspaceItems : tabs.map((tab) => createSingleWorkspaceItem(tab.id));
  $: displayTabs = projectDisplayTabs(displayItems, tabs, previewTabId, currentFolderPath);
  $: displayActiveTabId = activeItemId || findWorkspaceItemForTab(displayItems, activeTabId)?.id || '';

  function requestCloseTab(tab: DisplayTab, event?: Event) {
    if (workspaceInteractionDisabled) return;
    if (closeWorkspaceItem) void closeWorkspaceItem(tab.id, event);
    else closeTab(getWorkspaceItemFocusedTabId(tab.workspaceItem), event);
  }

  function getBatchCloseId(tab: DisplayTab) {
    return workspaceItems.length > 0 ? tab.id : getWorkspaceItemFocusedTabId(tab.workspaceItem);
  }

  function handleTabPointerDown(tab: DisplayTab, event: PointerEvent) {
    if (workspaceInteractionDisabled || event.button !== 0 ||
      (event.target as HTMLElement | null)?.closest('.close-tab-btn')) return;
    onWorkspaceItemPointerDown?.(tab.id, event);
  }

  function guardMenuItems(items: ContextMenuItem[]): ContextMenuItem[] {
    return items.map((item) => ({
      ...item,
      disabled: item.disabled || workspaceInteractionDisabled,
      children: item.children ? guardMenuItems(item.children) : undefined,
      action: item.action ? () => !workspaceInteractionDisabled && item.action?.() : undefined,
    }));
  }

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
  let hiddenTabs: DisplayTab[] = [];
  let visibleRange = { start: 0, end: 0 };
  let resizeObserver: ResizeObserver | null = null;
  let measureQueued = false;
  let dropdownMenuStyle = '';
  let pendingActiveTabId: string | null = null;
  let visualActiveTabId = displayActiveTabId;
  let tabMeasureKey = '';
  let activeTabIndex = -1;
  let activeTabOutsideVisibleRange = false;

  $: hiddenTabs = displayTabs.filter((_, i) => i < visibleRange.start || i >= visibleRange.end);

  // 构建标签右键菜单项
  function buildTabContextMenuItems(tab: DisplayTab): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    const isPreview = tab.preview;
    const tabIndex = displayTabs.findIndex((t) => t.id === tab.id);

    // 步骤1：基础关闭操作
    items.push({
      label: t.close(),
      icon: 'close',
      action: () => requestCloseTab(tab),
      shortcut: isPreview ? undefined : 'Ctrl+W',
    });

    // 步骤2：批量关闭操作
    const otherTabs = displayTabs.filter((t) => t.id !== tab.id);
    if (otherTabs.length > 0) {
      items.push({
        label: t.closeOtherTabs(),
        icon: 'close',
        action: () => dispatch('closeOtherTabs', { tabId: getBatchCloseId(tab) }),
      });
    }

    const rightTabs = displayTabs.slice(tabIndex + 1);
    if (rightTabs.length > 0) {
      items.push({
        label: t.closeTabsToRight(),
        icon: 'close',
        action: () => dispatch('closeTabsToRight', { tabId: getBatchCloseId(tab) }),
      });
    }

    items.push({
      label: t.closeAllTabs(),
      icon: 'close',
      action: () => dispatch('closeAllTabs'),
      danger: true,
    });

    const workspaceActions = getWorkspaceItemActions(tab.id);
    if (workspaceActions.length > 0) {
      items.push({ label: '', separator: true }, ...workspaceActions);
    }

    // 组合包含两条路径，路径操作按成员提供，避免误用另一篇文档。
    if (tab.workspaceItem.kind === 'comparison') {
      const members = getWorkspaceItemTabIds(tab.workspaceItem).flatMap((id) => {
        const member = tabs.find((candidate) => candidate.id === id);
        return member ? [member] : [];
      }).filter((member) => member.nativePath || member.filePath);
      if (members.length > 0) {
        items.push({ label: '', separator: true });
        items.push({
          label: t.copyPath(), icon: 'copy', children: members.map((member) => ({
            label: member.fileName,
            action: () => copyContextText(member.nativePath || member.filePath),
          })),
        });
        items.push({
          label: t.revealInFolder(), icon: 'folder', children: members.map((member) => ({
            label: member.fileName,
            action: () => revealContextPath(member.nativePath || member.filePath),
          })),
        });
      }
      return guardMenuItems(items);
    }

    // 路径相关操作
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

    return guardMenuItems(items);
  }

  function handleTabContextMenu(tab: DisplayTab, event: MouseEvent) {
    event.preventDefault();
    if (workspaceInteractionDisabled) return;
    openContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: buildTabContextMenuItems(tab),
    });
  }

  function handleTabStripContextMenu(event: MouseEvent) {
    if (event.defaultPrevented || workspaceInteractionDisabled) return;
    const target = event.target as HTMLElement | null;
    event.preventDefault();
    if (!target || target.closest('button')) return;
    openContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: guardMenuItems([
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
      ]),
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

    if (displayTabs.length === 0) {
      if (visibleRange.start !== 0 || visibleRange.end !== 0) {
        visibleRange = { start: 0, end: 0 };
      }
      showAddButton = true;
      overflowState = false;
      void tick().then(registerDropZones);
      return;
    }

    const measured = measureArea.querySelectorAll<HTMLElement>('.doc-tab');
    const widths: number[] = [];
    measured.forEach((el) => widths.push(el.getBoundingClientRect().width));

    const available = tabsContainer.clientWidth;
    const reserveWidth = dropdownButtonWidth + 4;

    // 从右到左累加，优先显示右边的（最新的）标签
    let used = 0;
    let start = displayTabs.length;
    for (let i = displayTabs.length - 1; i >= 0; i--) {
      const w = widths[i] ?? 120;
      if (used + w > available - reserveWidth && start < displayTabs.length) break;
      used += w;
      start = i;
    }

    let end = displayTabs.length;
    // 如果激活标签不在可见范围内，平移窗口使其包含激活标签
    const activeIdx = displayTabs.findIndex((t) => t.id === displayActiveTabId);
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
      while (end < displayTabs.length) {
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
    const isOverflowing = !(nextStart === 0 && nextEnd === displayTabs.length);
    if (isOverflowing) {
      showAddButton = false;
      overflowState = true;
    } else {
      showAddButton = true;
      overflowState = false;
      showDropdown = false;
    }
    void tick().then(registerDropZones);
  }

  function registerDropZones() {
    if (!tabsContainer) return;
    if (workspaceInteractionDisabled) {
      onDropZones([]);
      return;
    }
    const zones: TabDropZone[] = [];
    const elements = tabsContainer.querySelectorAll<HTMLElement>('[data-workspace-item-id]');
    const strip = tabsContainer.getBoundingClientRect();
    let lastRight = strip.left;
    for (const element of elements) {
      const itemId = element.dataset.workspaceItemId;
      const index = displayTabs.findIndex((tab) => tab.id === itemId);
      const tab = displayTabs[index];
      if (!tab) continue;
      const rect = element.getBoundingClientRect();
      const canCombine = tab.workspaceItem.kind === 'single' && tab.documentKind === 'markdown';
      const edgeWidth = rect.width * (canCombine ? 0.25 : 0.5);
      zones.push({
        id: `${itemId}:before`, x: rect.left, y: rect.top, width: edgeWidth, height: rect.height,
        placement: { kind: 'insert', insertionIndex: index },
      });
      if (canCombine) zones.push({
        id: `${itemId}:combine`, x: rect.left + edgeWidth, y: rect.top,
        width: rect.width * 0.5, height: rect.height,
        placement: { kind: 'combine', targetItemId: tab.id },
      });
      zones.push({
        id: `${itemId}:after`, x: rect.right - edgeWidth, y: rect.top, width: edgeWidth,
        height: rect.height, placement: { kind: 'insert', insertionIndex: index + 1 },
      });
      lastRight = Math.max(lastRight, rect.right);
    }
    if (lastRight < strip.right) zones.push({
      id: 'strip:end', x: lastRight, y: strip.top, width: strip.right - lastRight,
      height: strip.height, placement: { kind: 'insert', insertionIndex: displayTabs.length },
    });
    onDropZones(zones);
    updateDropInsertionPosition();
  }

  function updateDropInsertionPosition() {
    dropInsertionLeft = null;
    if (!tabsContainer || workspaceInteractionDisabled || dropHighlight?.phase !== 'move' ||
      dropHighlight.placement?.kind !== 'insert') return;
    const index = dropHighlight.placement.insertionIndex;
    const visibleElements = [...tabsContainer.querySelectorAll<HTMLElement>('[data-workspace-item-id]')];
    const next = visibleElements.find((element) => element.dataset.workspaceItemId === displayTabs[index]?.id);
    const previous = visibleElements.find((element) => element.dataset.workspaceItemId === displayTabs[index - 1]?.id);
    const edge = next?.getBoundingClientRect().left ?? previous?.getBoundingClientRect().right ??
      (index === displayTabs.length
        ? visibleElements.at(-1)?.getBoundingClientRect().right ?? tabsContainer.getBoundingClientRect().left
        : undefined);
    if (edge !== undefined) dropInsertionLeft = edge - tabsContainer.getBoundingClientRect().left;
  }

  function isCombineTarget(itemId: string, highlight: NativeTabDragEvent | null, disabled: boolean) {
    return !disabled && highlight?.phase === 'move' &&
      highlight.placement?.kind === 'combine' && highlight.placement.targetItemId === itemId;
  }

  function updateDropdownPosition() {
    if (!dropdownBtnEl || !showDropdown) return;
    const rect = dropdownBtnEl.getBoundingClientRect();
    dropdownMenuStyle = `position:fixed;top:${rect.bottom + 4}px;left:${rect.left - 172 + rect.width}px;z-index:9999;`;
  }

  function toggleDropdown(event: Event) {
    event.stopPropagation();
    if (workspaceInteractionDisabled) return;
    showDropdown = !showDropdown;
    if (showDropdown) {
      void tick().then(updateDropdownPosition);
    }
  }

  async function requestTabSwitch(tabId: string) {
    if (workspaceInteractionDisabled || !tabId || tabId === displayActiveTabId || pendingActiveTabId !== null) return;
    const item = displayItems.find((candidate) => candidate.id === tabId);
    if (!item) return;

    pendingActiveTabId = tabId;
    try {
      if (selectWorkspaceItem) await selectWorkspaceItem(tabId);
      else await switchTab(getWorkspaceItemFocusedTabId(item));
    } finally {
      if (displayActiveTabId !== tabId) {
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

  $: if (pendingActiveTabId === displayActiveTabId) {
    pendingActiveTabId = null;
  }
  $: visualActiveTabId = pendingActiveTabId ?? displayActiveTabId;
  $: tabMeasureKey = `${previewTabId ?? ''}|${displayTabs
    .map((tab) => `${tab.id}:${tab.documentKind}:${tab.fileName}:${tab.dirty ? '1' : '0'}`)
    .join('|')}`;
  $: activeTabIndex = displayTabs.findIndex((tab) => tab.id === displayActiveTabId);
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
  $: {
    visibleRange;
    displayTabs;
    workspaceInteractionDisabled;
    void tick().then(registerDropZones);
  }
  $: {
    dropHighlight;
    void tick().then(updateDropInsertionPosition);
  }

  onMount(() => {
    resizeObserver = new ResizeObserver(queueMeasureTabs);
    resizeObserver.observe(tabsContainer);
    queueMeasureTabs();

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
      onDropZones([]);
    };
  });
</script>

<svelte:window on:click={handleWindowClick} on:resize={queueMeasureTabs} />

{#key interfaceLocale}
  <!-- 隐藏的测量区域：渲染所有标签以测量宽度 -->
  <div class="tab-measure-area" bind:this={measureArea}>
    {#each displayTabs as tab (tab.id)}
      <button
        type="button"
        class="doc-tab"
        class:comparison={tab.workspaceItem.kind === 'comparison'}
        class:preview={tab.preview}
        title={tab.tooltip}
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
      {#if dropInsertionLeft !== null}
        <span class="tab-drop-insertion-marker" style:left={`${dropInsertionLeft}px`} aria-hidden="true"></span>
      {/if}
      {#each displayTabs.slice(visibleRange.start, visibleRange.end) as tab (tab.id)}
        <button
          type="button"
          class="doc-tab"
          class:comparison={tab.workspaceItem.kind === 'comparison'}
          class:active={visualActiveTabId === tab.id}
          class:preview={tab.preview}
          class:drop-combine={isCombineTarget(tab.id, dropHighlight, workspaceInteractionDisabled)}
          data-workspace-item-id={tab.id}
          aria-disabled={workspaceInteractionDisabled}
          title={tab.tooltip}
          use:motionIn={{ kind: 'row', y: 5 }}
          on:click={() => void requestTabSwitch(tab.id)}
          on:pointerdown={(event) => handleTabPointerDown(tab, event)}
          on:dblclick={() => {
            if (!workspaceInteractionDisabled && tab.preview) pinPreviewTab();
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
            on:click|stopPropagation={(event) => requestCloseTab(tab, event)}
            on:keydown|stopPropagation={(event) => {
              if (event.key === 'Enter') requestCloseTab(tab, event);
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
            disabled={workspaceInteractionDisabled}
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
          disabled={workspaceInteractionDisabled}
          on:click={() => !workspaceInteractionDisabled && createNewFile()}
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
          class:active={displayActiveTabId === tab.id}
          class:preview={tab.preview}
          role="menuitem"
          title={tab.tooltip}
          disabled={workspaceInteractionDisabled}
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

<style>
  .doc-tab.drop-combine {
    background: color-mix(in srgb, var(--md-editor-accent) 15%, transparent);
    box-shadow: inset 0 0 0 2px var(--md-editor-accent);
    border-radius: var(--md-editor-radius-md);
  }

  .tab-drop-insertion-marker {
    position: absolute;
    top: 3px;
    bottom: 2px;
    z-index: 5;
    width: 3px;
    border-radius: 2px;
    background: var(--md-editor-accent);
    pointer-events: none;
    transform: translateX(-1.5px);
  }
</style>
