<script lang="ts">
  import type { RecentEntry } from '../../lib/desktop/tauriStorage';
  import type { SoftwareUpdateSnapshot } from '../../lib/desktop/tauriUpdater';
  import type {
    EditorCommand,
    EditorThemeOptions,
    InlinePendingMarks,
    ContextMenuRequest,
    EditorCore,
  } from '../../lib/editor-core';
  import type { FrontMatterBlock } from '../../lib/markdown/frontMatter';
  import type { DocumentStats, OutlineItem } from '../../lib/outline/outlineService';
  import type {
    MarkdownLintIssue,
    MarkdownLintRuleSet,
    MarkdownLintState,
  } from '../../lib/markdown-lint/types';
  import { ChevronDown } from '@lucide/svelte';
  import { onDestroy } from 'svelte';
  import type {
    EditorViewMode,
    ExternalFileChangeState,
    FileTreeNode,
    SplitActivePane,
    SplitViewLayout,
    Tab,
  } from '../types';
  import AppTitleBar from './AppTitleBar.svelte';
  import type { MarkdownSourceEditorHandle } from './markdownSourceEditor';
  import DocumentTabs from './DocumentTabs.svelte';
  import MobileDocumentsSidebar from './MobileDocumentsSidebar.svelte';
  import EmptyWorkspace from './EmptyWorkspace.svelte';
  import EditorToolbar from './EditorToolbar.svelte';
  import EditorWorkspace from './EditorWorkspace.svelte';
  import ExplorerSidebar from './ExplorerSidebar.svelte';
  import LinkQuickEditor from './LinkQuickEditor.svelte';
  import MarkdownMiniLargePreview from './MarkdownMiniLargePreview.svelte';
  import SearchReplacePanel from './SearchReplacePanel.svelte';
  import StatusBar from './StatusBar.svelte';
  import SegmentedTextEditorWorkspace from './SegmentedTextEditorWorkspace.svelte';
  import {
    toolbarVisibilityMotion,
    transitionDuration,
    workspaceSidebarMotion,
  } from '../actions/motion';
  import { t, type EffectiveInterfaceLocale } from '../i18n';
  import { suppressUnhandledContextMenu } from '../services/contextMenuPolicy';

  type StatsMetric = 'lines' | 'words' | 'visibleChars' | 'chars';
  type AppBootState = 'booting' | 'restoring-workspace' | 'opening-file' | 'ready';

  export let interfaceLocale: EffectiveInterfaceLocale;
  export let focusMode: boolean;
  export let toolbarHidden: boolean;
  export let toolbarShortcut: string;
  export let markdownMiniShortcut: string;
  export let markdownMiniActive: boolean;
  export let markdownMiniPinned: boolean;
  export let markdownMiniExternalChanged: boolean;
  export let isResizing: boolean;
  export let contentWidthPercent: number;
  export let fileInput: HTMLInputElement;
  export let sourcePane: HTMLElement;
  export let semanticPane: HTMLElement;
  export let sourceEditor: MarkdownSourceEditorHandle;
  export let editorHost: HTMLDivElement;
  export let editorCore: EditorCore;
  export let theme: 'light' | 'dark';
  export let editorTheme: EditorThemeOptions;
  export let desktopEnabled: boolean;
  export let activeMenu: string | null;
  export let recentFiles: RecentEntry[];
  export let missingRecentPaths: Set<string>;
  export let mode: EditorViewMode;
  export let splitViewLayout: SplitViewLayout;
  export let splitLeftPercent: number;
  export let splitActivePane: SplitActivePane;
  export let splitAlignmentGuideVisible: boolean;
  export let outlineVisible: boolean;
  export let currentFolderPath: string;
  export let rootFolderExpanded: boolean;
  export let folderTree: FileTreeNode[];
  export let expandedFolders: Set<string>;
  export let nativePath: string | null;
  export let dirty: boolean;
  export let fileName: string;
  export let filePath: string;
  export let sidebarWidth: number;
  export let tabs: Tab[];
  export let activeTabId: string;
  export let previewTabId: string | null;
  export let markdown: string;
  export let sourceDocumentId: string;
  export let largeDocumentMode: boolean;
  export let frontMatter: FrontMatterBlock | null;
  export let frontMatterEditing: boolean;
  export let frontMatterFocusRequest: number;
  export let frontMatterFocusTarget: 'default' | 'title-value';
  export let readonlyDocumentMode: boolean;
  export let outline: OutlineItem[];
  export let activeOutlineId: string;
  export let collapsedOutlineIds: Set<string>;
  export let visibleOutlineIds: Set<string>;
  export let stats: DocumentStats;
  export let writingStatsVisible: boolean;
  export let writingStatsMetric: StatsMetric;
  export let readingTimeVisible: boolean;
  export let markdownLintEnabled: boolean;
  export let markdownLintRuleSet: MarkdownLintRuleSet;
  export let markdownLintState: MarkdownLintState;
  export let zoomPercent: number;
  export let tablePickerOpen: boolean;
  export let linkPickerOpen: boolean;
  export let linkText: string;
  export let linkHref: string;
  export let linkError: string;
  export let linkCanRemove: boolean;
  export let linkPickerPositionStyle: string;
  export let searchPanelOpen: boolean;
  export let searchReplaceVisible: boolean;
  export let searchQuery: string;
  export let searchReplacement: string;
  export let searchCaseSensitive: boolean;
  export let searchWholeWord: boolean;
  export let searchBackwards: boolean;
  export let searchWrapAround: boolean;
  export let searchActiveIndex: number;
  export let searchMatchCount: number;
  export let autoSaveEnabled: boolean;
  export let autoSaveDelayMs: number;
  export let segmentedWorkspace: SegmentedTextEditorWorkspace | null = null;

  export let getCompactPath: (path: string) => string;
  export let getFolderName: (path: string) => string;
  export let getDirectoryLabel: (path: string) => string;
  export let toggleMenu: (menu: string) => void;
  export let closeMenu: (menu: string) => void;
  export let toggleTheme: () => void;
  export let exitApp: () => void;
  export let createNewWindow: () => void;
  export let createNewFile: () => void;
  export let openFileDialog: () => void;
  export let openFolderDialog: () => void;
  export let openRecentEntry: (path: string, entryType: 'file' | 'folder') => void;
  export let openPreviewFile: (path: string) => void | boolean | Promise<void | boolean>;
  export let pinPreviewFile: () => void;
  export let clearRecentEntriesList: () => void;
  export let removeRecentEntry: (path: string) => void;
  export let closeCurrentFile: () => void;
  export let closeCurrentWindow: () => void;
  export let saveMarkdownFile: (saveAs?: boolean) => void;
  export let runCommand: (command: EditorCommand) => void;
  export let pendingInlineMarks: InlinePendingMarks;
  export let openTablePicker: () => void;
  export let openLinkPicker: () => void;
  export let openSearchPanel: (replaceVisible?: boolean) => void;
  export let closeSearchPanel: () => void;
  export let updateSearchQuery: (event: Event) => void;
  export let updateSearchReplacement: (event: Event) => void;
  export let toggleSearchCaseSensitive: () => void;
  export let toggleSearchWholeWord: () => void;
  export let toggleSearchBackwards: () => void;
  export let toggleSearchWrapAround: () => void;
  export let toggleSearchReplaceVisible: () => void;
  export let findPreviousSearchMatch: () => void;
  export let findNextSearchMatch: () => void;
  export let countSearchMatches: () => void;
  export let replaceCurrentSearchMatch: () => void;
  export let replaceAllSearchMatches: () => void;
  export let editFrontMatter: () => void;
  export let showUnavailableFeature: (featureName: string) => void;
  export let closeTablePicker: () => void;
  export let closeLinkPicker: () => void;
  export let updateLinkText: (event: Event) => void;
  export let updateLinkHref: (event: Event) => void;
  export let applyLink: () => void;
  export let removeLink: () => void;
  export let insertTableWithSize: (rows: number, columns: number) => void;
  export let openSettings: () => void;
  export let setMode: (mode: EditorViewMode) => void;
  export let setSplitActivePane: (pane: SplitActivePane) => void;
  export let updateSplitLeftPercent: (percent: number, persist: boolean) => void;
  export let toggleSplitAlignmentGuide: () => void;
  export let toggleOutlineVisible: () => void;
  export let toggleFocusMode: () => void;
  export let toggleToolbar: () => void;
  export let toggleMarkdownMini: () => void;
  export let toggleMarkdownMiniPinned: () => void;
  export let toggleRootFolder: () => void;
  export let toggleFolderCollapse: (folderPath: string) => void;
  export let startResize: (event: MouseEvent) => void;
  export let exportHtml: () => void;
  export let exportPdf: () => void;
  export let softwareUpdateState: SoftwareUpdateSnapshot;
  export let openSoftwareUpdate: () => void;
  export let switchTab: (tabId: string) => void;
  export let closeTab: (tabId: string, event?: Event) => void;
  export let pinPreviewTab: () => void;
  export let updateContentWidth: (event: Event) => void;
  export let updateMarkdown: (markdown: string) => void;
  export let onSourceSelectionChange: (selectedMarkdown: string) => void;
  export let enterFrontMatterEdit: () => void;
  export let leaveFrontMatterEdit: () => void;
  export let updateFrontMatterContent: (content: string) => void;
  export let deleteFrontMatter: () => void;
  export let updateActiveOutlineFromSourceScroll: () => void;
  export let updateActiveOutlineFromSemanticScroll: () => void;
  export let handleEditorPaste: (event: ClipboardEvent) => void;
  export let handleEditorDrop: (event: DragEvent) => void;
  export let handleWorkspaceContextMenu: (event: MouseEvent) => void;
  export let openContextMenu: (request: ContextMenuRequest) => void;
  export let copyContextText: (text: string) => void | Promise<void>;
  export let revealContextPath: (path: string) => void | Promise<void>;
  export let isOutlineItemExpandable: (index: number) => boolean;
  export let toggleOutlineItemExpanded: (item: OutlineItem) => void;
  export let expandAllOutline: () => void;
  export let collapseAllOutline: () => void;
  export let collapseOutlineToDefaultLevel: () => void;
  export let jumpToOutlineItem: (item: OutlineItem) => void;
  export let moveOutlineSection: (request: {
    sourceIndex: number;
    targetIndex: number;
    placement: 'before' | 'inside' | 'after';
  }) => boolean;
  export let openMarkdownFile: (event: Event) => void;
  export let setWritingStatsMetric: (metric: StatsMetric) => void;
  export let onZoomChange: (percent: number) => void;
  export let retryMarkdownLint: () => void;
  export let onMarkdownLintIssueSelect: (issue: MarkdownLintIssue) => boolean;
  export let appBootState: AppBootState;
  const isMobileRuntime = /Android|iPhone|iPad|iPod/i.test(globalThis.navigator?.userAgent ?? '');
  let mobileDocumentsOpen = false;
  export let onSourceScroll: (() => void) | undefined = undefined;
  export let onSemanticScroll: (() => void) | undefined = undefined;

  $: hasOpenDocument = appBootState === 'ready' && tabs.length > 0 && Boolean(activeTabId);
  $: activeTab = tabs.find((tab) => tab.id === activeTabId) ?? null;
  $: effectiveToolbarHidden = toolbarHidden;
  const toolbarTransitionDuration = transitionDuration('panel');
  const toolbarRevealDelay = Math.round(toolbarTransitionDuration * 0.8);
  let toolbarOverflowVisible = !effectiveToolbarHidden;
  let toolbarOverflowFrame = 0;

  $: if (effectiveToolbarHidden) {
    toolbarOverflowVisible = false;
    cancelToolbarOverflowFrame();
  } else if (activeTab?.documentKind !== 'markdown' || markdownMiniActive) {
    // 工具栏未参与高度过渡时，直接恢复弹出层的溢出显示。
    toolbarOverflowVisible = true;
  }

  function handleToolbarTransitionEnd(event: TransitionEvent) {
    if (
      event.target !== event.currentTarget ||
      event.propertyName !== 'height' ||
      effectiveToolbarHidden
    ) {
      return;
    }

    cancelToolbarOverflowFrame();
    // 先让展开后的最终布局完整绘制一帧，再允许工具栏弹出层越界显示。
    toolbarOverflowFrame = requestAnimationFrame(() => {
      toolbarOverflowFrame = 0;
      if (!effectiveToolbarHidden) {
        toolbarOverflowVisible = true;
      }
    });
  }

  function cancelToolbarOverflowFrame() {
    if (!toolbarOverflowFrame) return;
    cancelAnimationFrame(toolbarOverflowFrame);
    toolbarOverflowFrame = 0;
  }

  onDestroy(cancelToolbarOverflowFrame);
</script>

<svelte:window on:contextmenu={suppressUnhandledContextMenu} />

<div
  class="app-layout"
  class:mobile-runtime={isMobileRuntime}
  class:focus-mode={focusMode}
  class:markdown-mini-mode={markdownMiniActive}
  class:resizing={isResizing}
  style={`--md-editor-content-width-percent: ${contentWidthPercent}`}
>
  <input
    bind:this={fileInput}
    class="file-input"
    type="file"
    accept=".md,.markdown,text/markdown"
    on:change={openMarkdownFile}
  />

  {#if desktopEnabled}
    {#if !isMobileRuntime}
      <AppTitleBar
      {interfaceLocale}
      {theme}
      {desktopEnabled}
      {activeMenu}
      {recentFiles}
      {missingRecentPaths}
      {mode}
      {focusMode}
      toolbarHidden={effectiveToolbarHidden}
      {toolbarShortcut}
      {markdownMiniShortcut}
      markdownMiniAvailable={activeTab?.documentKind === 'markdown'}
      {markdownMiniActive}
      {markdownMiniPinned}
      {markdownMiniExternalChanged}
      {fileName}
      {filePath}
      {dirty}
      {largeDocumentMode}
      {outlineVisible}
      {getCompactPath}
      {toggleMenu}
      {closeMenu}
      {toggleTheme}
      {exitApp}
      {createNewWindow}
      {createNewFile}
      {openFileDialog}
      {openFolderDialog}
      {openRecentEntry}
      {saveMarkdownFile}
      {clearRecentEntriesList}
      {removeRecentEntry}
      {closeCurrentFile}
      {closeCurrentWindow}
      {runCommand}
      {openTablePicker}
      {openLinkPicker}
      {editFrontMatter}
      {showUnavailableFeature}
      {setMode}
      {toggleOutlineVisible}
      {toggleFocusMode}
      {toggleToolbar}
      {toggleMarkdownMini}
      {toggleMarkdownMiniPinned}
      {openSettings}
      {exportHtml}
      {exportPdf}
      {softwareUpdateState}
      {openSoftwareUpdate}
      {openContextMenu}
      />
    {/if}
  {/if}

  <main
    class="workspace"
    style="--sidebar-width: {sidebarWidth}px"
    use:workspaceSidebarMotion={{ focusMode, isResizing }}
  >
    <ExplorerSidebar
      {interfaceLocale}
      {currentFolderPath}
      {rootFolderExpanded}
      {folderTree}
      {expandedFolders}
      {nativePath}
      {dirty}
      {fileName}
      {filePath}
      {isResizing}
      {getFolderName}
      {getDirectoryLabel}
      {toggleRootFolder}
      {toggleFolderCollapse}
      {openPreviewFile}
      {pinPreviewFile}
      previewNativePath={previewTabId
        ? (tabs.find((t) => t.id === previewTabId)?.nativePath ?? null)
        : null}
      {startResize}
      {openContextMenu}
      copyContextText={copyContextText}
      on:createNode
      on:renameNode
      on:refreshFolder
      on:collapseAll
      on:deleteNode
      on:revealError
    />

    <section
      class="editor-shell"
      class:markdown-mini={markdownMiniActive}
      class:markdown-mini-large={markdownMiniActive && largeDocumentMode}
      class:has-open-document={appBootState === 'ready' && hasOpenDocument}
      class:no-open-document={appBootState === 'ready' && !hasOpenDocument}
      class:toolbar-hidden={effectiveToolbarHidden}
      style={`--toolbar-transition-duration: ${toolbarTransitionDuration}ms; --toolbar-reveal-delay: ${toolbarRevealDelay}ms`}
      aria-label={t.semanticEditorArea()}
    >
      {#if hasOpenDocument && !isMobileRuntime}
        <DocumentTabs
          {interfaceLocale}
          {tabs}
          {activeTabId}
          {previewTabId}
          {switchTab}
          {closeTab}
          {pinPreviewTab}
          {createNewFile}
          {openFileDialog}
          {openFolderDialog}
          {openContextMenu}
          copyContextText={copyContextText}
          revealContextPath={revealContextPath}
          {currentFolderPath}
          on:closeOtherTabs
          on:closeTabsToRight
          on:closeAllTabs
        />
      {/if}

      {#if appBootState !== 'ready'}
        <div class="startup-loading" role="status" aria-live="polite">
          <span>正在恢复工作区...</span>
        </div>
      {:else if hasOpenDocument}
        <div class="editor-card">
          {#if activeTab?.documentKind === 'markdown'}
            <div
              class="editor-toolbar-region"
              class:collapsed={effectiveToolbarHidden}
              class:overflow-visible={toolbarOverflowVisible}
              on:transitionend={handleToolbarTransitionEnd}
            >
              <div
                aria-hidden={!toolbarOverflowVisible || markdownMiniActive}
                inert={!toolbarOverflowVisible || markdownMiniActive}
                use:toolbarVisibilityMotion={{ hidden: effectiveToolbarHidden }}
              >
                <EditorToolbar
                  {interfaceLocale}
                  {mode}
                  {largeDocumentMode}
                  {contentWidthPercent}
                  {outlineVisible}
                  {toolbarShortcut}
                  {runCommand}
                  {pendingInlineMarks}
                  {tablePickerOpen}
                  {openTablePicker}
                  {closeTablePicker}
                  {openLinkPicker}
                  {insertTableWithSize}
                  {updateContentWidth}
                  {setMode}
                  {splitAlignmentGuideVisible}
                  {toggleSplitAlignmentGuide}
                  {toggleOutlineVisible}
                  {toggleToolbar}
                  mobile={isMobileRuntime}
                  openMobileDocuments={() => (mobileDocumentsOpen = true)}
                  inactive={!toolbarOverflowVisible || markdownMiniActive}
                  openSearchPanel={() => openSearchPanel(false)}
                />
              </div>
            </div>

            {#if toolbarHidden}
              <button
                class="toolbar-reveal-button"
                type="button"
                title={`${t.showToolbar()} (${toolbarShortcut})`}
                aria-label={t.showToolbar()}
                on:click={toggleToolbar}
              >
                <ChevronDown size={15} />
              </button>
            {/if}

            <SearchReplacePanel
              {interfaceLocale}
              open={searchPanelOpen}
              replaceVisible={searchReplaceVisible}
              query={searchQuery}
              replacement={searchReplacement}
              caseSensitive={searchCaseSensitive}
              wholeWord={searchWholeWord}
              backwards={searchBackwards}
              wrapAround={searchWrapAround}
              activeIndex={searchActiveIndex}
              matchCount={searchMatchCount}
              readonly={readonlyDocumentMode}
              updateQuery={updateSearchQuery}
              updateReplacement={updateSearchReplacement}
              toggleCaseSensitive={toggleSearchCaseSensitive}
              toggleWholeWord={toggleSearchWholeWord}
              toggleBackwards={toggleSearchBackwards}
              toggleWrapAround={toggleSearchWrapAround}
              toggleReplaceVisible={toggleSearchReplaceVisible}
              findPrevious={findPreviousSearchMatch}
              findNext={findNextSearchMatch}
              countMatches={countSearchMatches}
              replaceCurrent={replaceCurrentSearchMatch}
              replaceAll={replaceAllSearchMatches}
              close={closeSearchPanel}
            />

            <EditorWorkspace
              {interfaceLocale}
              bind:sourcePane
              bind:semanticPane
              bind:sourceEditor
              bind:editorHost
              {editorCore}
              {mode}
              {splitViewLayout}
              {splitLeftPercent}
              {splitActivePane}
              {splitAlignmentGuideVisible}
              {markdown}
              {sourceDocumentId}
              {largeDocumentMode}
              {frontMatter}
              {frontMatterEditing}
              {frontMatterFocusRequest}
              {frontMatterFocusTarget}
              {readonlyDocumentMode}
              {outlineVisible}
              {outline}
              {activeOutlineId}
              {collapsedOutlineIds}
              {visibleOutlineIds}
              {updateMarkdown}
              {setSplitActivePane}
              {updateSplitLeftPercent}
              {onSourceSelectionChange}
              {enterFrontMatterEdit}
              {leaveFrontMatterEdit}
              {updateFrontMatterContent}
              {deleteFrontMatter}
              {updateActiveOutlineFromSourceScroll}
              {updateActiveOutlineFromSemanticScroll}
              {onSourceScroll}
              {onSemanticScroll}
              {handleEditorPaste}
              {handleEditorDrop}
              {handleWorkspaceContextMenu}
              {openContextMenu}
              copyContextText={copyContextText}
              {isOutlineItemExpandable}
              {toggleOutlineItemExpanded}
              {expandAllOutline}
              {collapseAllOutline}
              {collapseOutlineToDefaultLevel}
              {toggleOutlineVisible}
              {jumpToOutlineItem}
              {moveOutlineSection}
            />

            {#if markdownMiniActive && largeDocumentMode}
              <MarkdownMiniLargePreview {markdown} {nativePath} {editorTheme} />
            {/if}

            <LinkQuickEditor
              {interfaceLocale}
              open={linkPickerOpen}
              text={linkText}
              href={linkHref}
              error={linkError}
              canRemove={linkCanRemove}
              positionStyle={linkPickerPositionStyle}
              updateText={updateLinkText}
              updateHref={updateLinkHref}
              {applyLink}
              {removeLink}
              {closeLinkPicker}
            />
          {:else if activeTab?.documentKind === 'text' || activeTab?.documentKind === 'json'}
            {#key activeTab.sessionId}
              <SegmentedTextEditorWorkspace
                bind:this={segmentedWorkspace}
                {interfaceLocale}
                tab={activeTab}
                {autoSaveEnabled}
                {autoSaveDelayMs}
                on:stateChange
                on:status
              />
            {/key}
          {/if}
        </div>
      {:else}
        <EmptyWorkspace {interfaceLocale} {createNewFile} {openFileDialog} {openFolderDialog} />
      {/if}
    </section>

    {#if hasOpenDocument && activeTab?.documentKind === 'markdown' && (writingStatsVisible || markdownLintEnabled)}
      <StatusBar
        {interfaceLocale}
        {stats}
        {writingStatsVisible}
        activeMetric={writingStatsMetric}
        {readingTimeVisible}
        {zoomPercent}
        {markdownLintEnabled}
        {markdownLintRuleSet}
        {markdownLintState}
        onMetricChange={setWritingStatsMetric}
        {onZoomChange}
        onRetryMarkdownLint={retryMarkdownLint}
        {onMarkdownLintIssueSelect}
      />
    {/if}
  </main>

  {#if isMobileRuntime}
    <MobileDocumentsSidebar
      bind:open={mobileDocumentsOpen}
      {tabs}
      {activeTabId}
      {recentFiles}
      showTrigger={!hasOpenDocument || effectiveToolbarHidden}
      openRecentEntry={(path) => void openRecentEntry(path, 'file')}
      removeRecentEntry={removeRecentEntry}
      switchTab={switchTab}
    />
  {/if}
</div>

<style>
  .startup-loading {
    grid-row: 1 / -1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
    color: var(--md-editor-muted-fg);
    font-size: 14px;
    user-select: none;
    -webkit-user-select: none;
  }
</style>
