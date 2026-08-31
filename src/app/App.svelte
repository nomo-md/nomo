<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import {
    listenDesktopFileDrops,
    listenDesktopMenuCommands,
    listenDesktopOpenDocuments,
    listenDesktopOpenFolder,
    isTauriRuntime,
    listAppSettings,
    installSampleDocument,
    openExternalLink,
    openLocalAttachment,
    updateAppSetting,
    updateAppSettings,
    readWorkspaceDraft,
    deleteWorkspaceDraft,
    rememberRecentEntry,
    checkPathsExist,
    statMarkdownFile,
    pickDocumentPathWithDialog,
    deleteFile,
    revealInExplorer,
    type RecentEntry,
    type RecentEntryType,
    clearRecentEntries,
  } from '../lib/desktop/tauriStorage';
  import {
    createEditorCore,
    getImageLoader,
    setCodeBlockDiagramRenderer,
    setCodeBlockMathRenderer,
    setCodeBlockTokenizer,
    setImageLoader,
    type EditorChangeEvent,
    type EditorCommand,
    type EditorAnchorRect,
    type EditorImageDeletionEvent,
    type InlinePendingMarks,
    type EditorMode,
    type EditorPasteMode,
    type EditorSearchMatch,
    type EditorSelectionEvent,
    type EditorThemeOptions,
  } from '../lib/editor-core';
  import {
    analyzeMarkdown,
    calculateDocumentStats,
    type DocumentStats,
    type OutlineItem,
  } from '../lib/outline/outlineService';
  import { normalizeMarkdownEncoding } from '../lib/services/storage';
  import {
    extractFrontMatterBlock,
    removeFrontMatter,
    replaceFrontMatterContent,
    type FrontMatterBlock,
  } from '../lib/markdown/frontMatter';
  import AppShell from './components/AppShell.svelte';
  import type { MarkdownSourceEditorHandle } from './components/markdownSourceEditor';
  import type SegmentedTextEditorWorkspaceComponent from './components/SegmentedTextEditorWorkspace.svelte';
  import FolderOpenDialog from './components/FolderOpenDialog.svelte';
  import {
    createEmptyExternalFileChange,
    normalizeExternalFileChange,
    type ExternalFileChangeState,
    type FileTreeNode,
    type PersistedWorkspaceState,
    type PersistedWorkspaceTab,
    type MarkdownTabState,
    type EditorViewMode,
    type SplitActivePane,
    type SplitViewLayout,
    type Tab,
  } from './types';
  import {
    getCompactPath,
    getDirectoryLabel,
    getFolderName,
    sameNativePath,
    pathEqualsOrDescendsFrom,
  } from './utils/pathLabels';
  import {
    executeDesktopCommand as executeDesktopAppCommand,
    handleGlobalShortcut as handleGlobalAppShortcut,
    type AppCommandHandlers,
  } from './services/appCommands';
  import {
    closeAppWindow as closeDesktopWindow,
    activateDocumentWindow,
    createAppWindow,
    enterMarkdownMiniMode,
    exitApp as exitDesktopApp,
    exitMarkdownMiniMode,
    openSettingsWindow,
    refreshInterfaceLanguageChrome,
    prepareOpenTargetWindow,
    setMarkdownMiniModePinned,
    syncWindowOpenTargets,
    updateAppWindowTitle,
    type OpenTarget,
  } from './services/desktopWindow';
  import { routeOpenTarget } from './services/openTargetRouting';
  import { createImageInsertionHandlers } from './services/imageInsertion';
  import { readEditorClipboard, writeEditorClipboard } from './services/clipboard';
  import { createDesktopImageLoader } from './services/desktopImageLoader';
  import { isOutlineItemVisible as getOutlineItemVisible } from './services/outlineState';
  import { writeRecoveryDraft as writeRecoveryDraftToStorage } from './services/recoveryDraft';
  import {
    createBlankTab,
    createTabForDocument,
    getDocumentKindFromPath,
    isMarkdownTab,
    isReusableUntitledTab,
    isSegmentedTextTab,
    writeActiveTabState,
  } from './services/tabs';
  import {
    createPersistedWorkspaceState,
    createRuntimeTabFromPersisted,
    migrateWorkspaceSetting,
    partitionPersistedWorkspaceTabsForRestore,
    persistWorkspaceDrafts,
    type WorkspaceDraftPersistenceCache,
  } from './services/workspacePersistence';
  import {
    findDroppedDocumentPath,
    readMarkdownFromPath,
    rememberNativeFolder,
    pickFolderPath,
  } from './services/documentFiles';
  import {
    closeActiveMenu,
    createSidebarResizeHandlers,
    getNextActiveMenu,
  } from './services/appUiState';
  import { createEditorSettingsController } from './services/editorSettingsController';
  import ContextMenu from './components/ContextMenu.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import UnsavedConfirmDialog from './components/UnsavedConfirmDialog.svelte';
  import ExternalChangeDialog from './components/ExternalChangeDialog.svelte';
  import CloseWindowBehaviorDialog from './components/CloseWindowBehaviorDialog.svelte';
  import SoftwareUpdateDialog from './components/SoftwareUpdateDialog.svelte';
  import SoftwareUpdateNotice from './components/SoftwareUpdateNotice.svelte';
  import type { SoftwareUpdateSnapshot } from '../lib/desktop/tauriUpdater';
  import type {
    ContextMenuOpenEvent,
    ContextMenuItem,
    ContextMenuRequest,
    ContextMenuTarget,
  } from '../lib/editor-core/plugins/contextMenu';
  import {
    DEFAULT_APP_PREFERENCES,
    SETTINGS_UPDATED_EVENT,
    applyCodeBlockLineNumberSetting,
    applyEditorLayoutSettings,
    applyTypographySettings,
    applyZoomSetting,
    loadAppPreferences,
    normalizeAppPreferences,
    type AppPreferences,
    type AppPreferencesPatch,
    type CloseWindowBehavior,
    type CodeBlockIndentPreference,
    type ExternalFileChangeBehavior,
    type InterfaceLanguagePreference,
    type OpenDefaultBehavior,
    type SettingsUpdatedPayload,
    type ShortcutPreferences,
  } from './services/settings';
  import {
    applyThemeRuntime,
    getBrowserSystemScheme,
    isColorScheme,
    listenForSystemThemeChanges,
    readEffectiveSystemScheme,
    readThemeBootSnapshot,
    resolveTheme,
    writeThemeBootSnapshot,
  } from './services/themeManager';
  import { getPlatformCapabilities } from './services/platform';
  import type { ThemeMode } from '../lib/theme/types';
  import { applyInterfaceLanguagePreference, t, type EffectiveInterfaceLocale } from './i18n';
  import { createFolderExplorerController } from './services/folderExplorerController';
  import { createDocumentActionsController } from './services/documentActionsController';
  import {
    FIRST_RUN_SAMPLE_DOCUMENT_OPEN_ERROR_KEY,
    FIRST_RUN_SAMPLE_DOCUMENT_OPENED_KEY,
    shouldMarkFirstRunSampleHandled,
    shouldOpenFirstRunSample,
    type FirstRunSampleState,
  } from './services/firstRunSample';
  import { createOutlineInteractionController } from './services/outlineInteractionController';
  import { createEditorInteractionController } from './services/editorInteractionController';
  import {
    disposeSoftwareUpdateCoordinator,
    initializeSoftwareUpdateCoordinator,
    runSoftwareUpdateCheck,
    softwareUpdateState,
    startSoftwareUpdateDownload,
    startSoftwareUpdateInstall,
  } from './services/softwareUpdate';
  import { createSoftwareUpdateSummary } from './services/softwareUpdateReleaseNotes';
  import {
    confirmAction,
    confirmDialogStore,
    resolveConfirmDialog,
    dismissConfirmDialog,
    type ConfirmDialogState,
  } from './services/confirmAction';
  import {
    getSemanticScrollAnchor,
    getSourceScrollAnchor,
    restoreSemanticReadingPosition,
    restoreSourceReadingPosition,
    scrollSemanticToAnchor,
    scrollSourceToAnchor,
    setScrollTop,
    type OutlineScrollAnchor,
  } from './services/outlineNavigation';
  import {
    flushReadingPositions,
    getReadingPosition,
    loadReadingPositions,
    saveReadingPositionToMemory,
    saveReadingPositionToMemoryOnly,
    type ReadingPosition,
    type ReadingPositionMode,
  } from './services/readingPosition';
  import {
    findTextMatches,
    replaceAllTextMatches,
    replaceTextRange,
  } from './services/searchReplace';
  import { createKatexMathRenderer } from '../lib/services/katexMathRenderer';
  import { createMermaidDiagramRenderer } from '../lib/services/mermaidDiagramRenderer';
  import { createShikiCodeTokenizer } from '../lib/services/shikiCodeTokenizer';
  import {
    DEFAULT_IMAGE_HANDLING_SETTINGS,
    type ImageContext,
    type ImageHandlingSettings,
  } from '../lib/services/render';
  import { disableLogger, enableLogger, logInfo } from '../lib/services/logger';
  import { createTauriSegmentedDocumentPort } from '../lib/text-editor/tauriPort';
  import type {
    OpenSegmentedDocumentResult,
    SegmentedExternalChangeResult,
  } from '../lib/text-editor/protocol';
  import type { SegmentedEditorMetadata } from '../lib/text-editor/SegmentedTextEditorCore';
  import type {
    MarkdownLintInput,
    MarkdownLintIssue,
    MarkdownLintRuleSet,
    MarkdownLintState,
  } from '../lib/markdown-lint/types';
  import { createMarkdownLintState } from '../lib/markdown-lint/types';
  import { segmentedSessionRegistry } from '../lib/text-editor/sessionRegistry';
  import { openDocumentByPath } from './services/documentRouter';
  import { flushSegmentedDocumentBeforeTransition } from './services/segmentedDocumentLifecycle';
  import { reconcileSegmentedExternalChangeCheck } from './services/segmentedExternalChangeReconciliation';
  import { reconcileSegmentedSaveState } from './services/segmentedSaveReconciliation';
  import { getOpenDocumentRenameBlock } from './services/documentRenameGuard';
  import { createMarkdownLintController } from './services/markdownLintController';
  import { EditorLinkResolutionError, resolveEditorLink } from './services/documentLinkNavigation';

  const RECOVERY_KEY = 'nomo-save-recovery';
  const segmentedDocumentPort = createTauriSegmentedDocumentPort();
  type WritingStatsMetric = 'lines' | 'words' | 'visibleChars' | 'chars';
  type CloseWindowAction = Exclude<CloseWindowBehavior, 'ask-every-time'>;
  type CloseWindowChoiceResult = { behavior: CloseWindowAction; remember: boolean } | null;
  type OpenTargetChoiceResult = {
    choice: 'current-window' | 'new-window';
    remember: boolean;
  } | null;
  type ZoomScrollAnchor = {
    pane: HTMLElement;
    element: HTMLElement;
    elementRatio: number;
    clientY: number;
  };

  function logCloseDiagnostics(message: string, data?: Record<string, unknown>) {
    logInfo('CloseGuard', message, data);
    // eslint-disable-next-line no-console
    console.info('[CloseGuard]', message, data ?? '');
  }

  setCodeBlockTokenizer(createShikiCodeTokenizer());
  setCodeBlockDiagramRenderer(createMermaidDiagramRenderer());
  setCodeBlockMathRenderer(createKatexMathRenderer());
  setImageLoader(createDesktopImageLoader());

  let markdown = '',
    savedMarkdown = '',
    dirty = false,
    version = 0;
  const bootAppearance = readThemeBootSnapshot() ?? DEFAULT_APP_PREFERENCES;
  const initialResolvedTheme = resolveTheme(
    bootAppearance,
    document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light',
  );
  let mode: EditorViewMode = DEFAULT_APP_PREFERENCES.editorMode;
  let preferredEditorMode: EditorViewMode = DEFAULT_APP_PREFERENCES.editorMode;
  let splitViewLayout: SplitViewLayout = DEFAULT_APP_PREFERENCES.splitViewLayout;
  let splitLeftPercent = DEFAULT_APP_PREFERENCES.splitLeftPercent;
  let splitActivePane: SplitActivePane = 'semantic';
  let splitAlignmentGuideVisible = false;
  let themeMode: ThemeMode = bootAppearance.themeMode;
  let colorThemeId = bootAppearance.colorThemeId;
  let documentStyleId = bootAppearance.documentStyleId;
  let theme: 'light' | 'dark' = initialResolvedTheme.effectiveScheme;
  let currentEditorTheme: EditorThemeOptions = initialResolvedTheme.editorTheme;
  let interfaceLanguage: InterfaceLanguagePreference = DEFAULT_APP_PREFERENCES.interfaceLanguage;
  let interfaceLocale: EffectiveInterfaceLocale =
    applyInterfaceLanguagePreference(interfaceLanguage);
  let fileName = '',
    filePath = '';
  let nativePath: string | null = null;
  let statusMessage = '';
  let desktopEnabled = false;
  let recentFiles: RecentEntry[] = [];
  let missingRecentPaths = new Set<string>();
  let outline: OutlineItem[] = [];
  let outlineVisible = DEFAULT_APP_PREFERENCES.outlineVisible,
    activeOutlineId = outline[0]?.id ?? '';
  let collapsedOutlineIds = new Set<string>();
  let visibleOutlineIds = new Set(outline.map((item) => item.id));
  let suppressOutlineScrollUntil = 0;
  let stats: DocumentStats = analyzeMarkdown('').stats;
  let selectedStats: DocumentStats | null = null;
  $: effectiveStats = selectedStats ?? stats;
  let writingStatsVisible = DEFAULT_APP_PREFERENCES.writingStatsVisible;
  let writingStatsMetric: WritingStatsMetric = DEFAULT_APP_PREFERENCES.writingStatsMetric;
  let readingTimeVisible = DEFAULT_APP_PREFERENCES.readingTimeVisible;
  let markdownLintEnabled = DEFAULT_APP_PREFERENCES.markdownLintEnabled;
  let markdownLintRuleSet: MarkdownLintRuleSet = DEFAULT_APP_PREFERENCES.markdownLintRuleSet;
  let markdownLintState: MarkdownLintState = createMarkdownLintState('disabled');
  let fontSize = DEFAULT_APP_PREFERENCES.fontSize,
    lineHeight = DEFAULT_APP_PREFERENCES.lineHeight,
    contentWidthPercent = DEFAULT_APP_PREFERENCES.contentWidthPercent,
    focusMode = DEFAULT_APP_PREFERENCES.sidebarHidden,
    toolbarHidden = DEFAULT_APP_PREFERENCES.toolbarHidden;
  let largeDocumentLimit = DEFAULT_APP_PREFERENCES.largeDocumentLimit;
  let autoSaveDelayMs = DEFAULT_APP_PREFERENCES.autoSaveDelayMs;
  let createSnapshotBeforeSave = DEFAULT_APP_PREFERENCES.createSnapshotBeforeSave;
  let defaultCodeBlockLanguage = DEFAULT_APP_PREFERENCES.defaultCodeBlockLanguage;
  let defaultDiagramType = DEFAULT_APP_PREFERENCES.defaultDiagramType;
  let zoomPercent = DEFAULT_APP_PREFERENCES.zoomPercent;
  let ctrlWheelZoomEnabled = DEFAULT_APP_PREFERENCES.ctrlWheelZoomEnabled;
  let outlineDefaultExpandLevel = DEFAULT_APP_PREFERENCES.outlineDefaultExpandLevel;
  let codeBlockLineNumbersVisible = DEFAULT_APP_PREFERENCES.codeBlockLineNumbersVisible;
  let codeBlockIndent: CodeBlockIndentPreference = DEFAULT_APP_PREFERENCES.codeBlockIndent;
  let inlineCodeRenderingEnabled = DEFAULT_APP_PREFERENCES.inlineCodeRenderingEnabled;
  let copyMarkdownSyntaxEnabled = DEFAULT_APP_PREFERENCES.copyMarkdownSyntaxEnabled;
  let shortcutPreferences: ShortcutPreferences = { ...DEFAULT_APP_PREFERENCES.shortcutPreferences };
  let imageSettings: ImageHandlingSettings = { ...DEFAULT_IMAGE_HANDLING_SETTINGS };
  let openDefaultBehavior: OpenDefaultBehavior = DEFAULT_APP_PREFERENCES.openDefaultBehavior;
  let pendingOpenChoice: OpenTarget | null = null;
  let pendingOpenChoiceResolver: ((result: OpenTargetChoiceResult) => void) | null = null;
  let editorHost: HTMLDivElement,
    fileInput: HTMLInputElement,
    sourceEditor: MarkdownSourceEditorHandle,
    semanticPane: HTMLElement,
    sourcePane: HTMLElement;
  let segmentedWorkspace: SegmentedTextEditorWorkspaceComponent | null = null;
  let mountedEditorHost: HTMLDivElement | null = null;
  let pendingSourceScrollTop: number | null = null;
  let splitSemanticRefreshTimer: number | null = null;
  let splitSemanticRefreshGeneration = 0;
  let refreshEditorViewportLayout: () => void = () => undefined;
  let largeDocumentMode = false,
    readonlyDocumentMode = false,
    diskReadonly = false,
    externalFileChange: ExternalFileChangeState = createEmptyExternalFileChange(),
    lastKnownModifiedAt = 0;
  let desktopUnlisteners: Array<() => void> = [];
  let criticalDesktopEventsReady = false;
  let pendingExternalOpenPaths: string[] = [];
  let currentFolderPath = '',
    folderTree: FileTreeNode[] = [];
  let startupFolderPath = '';
  let startupFolderLoadScheduled = false;
  let startupFolderLoadInProgress = false;
  let expandedFolders = new Set<string>();
  let tablePickerOpen = false;
  let linkPickerOpen = false;
  let linkText = '';
  let linkHref = '';
  let linkError = '';
  let linkCanRemove = false;
  let linkDraftTitle: string | null = null;
  let linkPickerPositionStyle = '';
  let linkOpening = false;
  let linkOpeningTimer: number | null = null;
  let linkOpeningToken = 0;
  let toastMessage = '';
  let toastTimer: number | null = null;
  let softwareUpdateSnapshot: SoftwareUpdateSnapshot = {
    status: 'idle',
    currentVersion: '',
    installationKind: 'unsupported',
  };
  let softwareUpdateNoticeVisible = false;
  let softwareUpdateDialogOpen = false;
  let softwareUpdateDismissedVersion = '';
  let softwareUpdateAutoCheckEnabled = DEFAULT_APP_PREFERENCES.softwareUpdateAutoCheckEnabled;
  let softwareUpdateStartupTimer: number | null = null;
  let softwareUpdateNoticeSignature = '';
  let unsubscribeSoftwareUpdate: (() => void) | null = null;

  /** 后台/非活动标签没有自己的状态栏，错误必须同时进入全局 toast 才对用户可见。 */
  function showVisibleError(error: unknown, fallback: string) {
    const message = error instanceof Error ? error.message : fallback;
    statusMessage = message;
    showToast(message, 3500);
  }

  let pendingInlineMarks: InlinePendingMarks = createEmptyPendingInlineMarks();
  let frontMatterEditing = false;
  let frontMatterFocusRequest = 0;
  let scrollDebounceTimer: number | null = null;
  let readingPositionRestoreGeneration = 0;
  let pendingReadingPositionRestore: {
    generation: number;
    tabId: string;
    filePath: string;
    mode: ReadingPositionMode;
  } | null = null;
  const programmaticReadingScrollTokens = new Map<ReadingPositionMode, number>();
  const sessionReadingPositions = new Map<string, ReadingPosition>();
  let contentAnalysisTimer: number | null = null;
  let frontMatterFocusTarget: 'default' | 'title-value' = 'default';
  let frontMatter: FrontMatterBlock | null = extractFrontMatterBlock(markdown);
  let searchPanelOpen = false;
  let searchReplaceVisible = false;
  let searchQuery = '';
  let searchReplacement = '';
  let searchCaseSensitive = false;
  let searchWholeWord = false;
  let searchBackwards = false;
  let searchWrapAround = true;
  let searchMatches: EditorSearchMatch[] = [];
  let searchActiveIndex = 0;
  let searchMatchCount = 0;
  let lastSearchSignature = '';
  let searchDebounceTimer: number | null = null;

  const CONTENT_ANALYSIS_DEBOUNCE_MS = 120;
  const SEARCH_DEBOUNCE_MS = 150;
  const SPLIT_SEMANTIC_REFRESH_DEBOUNCE_MS = 150;

  // 上下文菜单状态
  let contextMenuX = 0;
  let contextMenuY = 0;
  let contextMenuItems: ContextMenuItem[] = [];
  let contextMenuOpen = false;
  let contextMenuVersion = 0;

  // 删除确认对话框状态
  let deleteConfirmOpen = false;
  let deleteConfirmPath = '';
  let deleteConfirmIsDir = false;
  let deleteConfirmName = '';
  // 外部文件变更弹框状态
  let externalChangeDialogOpen = false;
  let externalChangeDialogState: ExternalFileChangeState | null = null;
  let externalChangeDialogTargetTabId: string | null = null;
  let externalChangeDialogTargetSessionId: string | null = null;
  let externalChangeDialogToken: string | null = null;
  // “忽略”只对当前会话中的同一次磁盘身份生效；新身份仍必须重新提示。
  const ignoredSegmentedExternalChanges = new Map<string, string>();

  let closeWindowChoiceDialogOpen = false;
  let rememberCloseWindowChoice = true;
  let closeWindowChoiceResolver: ((choice: CloseWindowChoiceResult) => void) | null = null;
  let closeWindowChoicePromise: Promise<CloseWindowChoiceResult> | null = null;

  interface StartupDraftConflict {
    tabId: string;
    fileName: string;
    filePath: string;
    nativePath: string;
    draftId: string;
    draftMarkdown: string;
    diskMarkdown: string;
    diskModifiedAt: number;
    diskReadonly: boolean;
    diskLargeDocumentMode: boolean;
  }

  let startupDraftConflict: StartupDraftConflict | null = null;
  let legacyInstallerPromptOpen = false;

  async function openWindowsInstalledAppsForLegacyNomo() {
    legacyInstallerPromptOpen = false;
    if (!desktopEnabled) return;
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_windows_installed_apps').catch(() => undefined);
  }

  // 订阅自定义确认对话框（Svelte 5 Runes 模式 $store 语法不工作，需手动订阅）
  let confirmDialogState: ConfirmDialogState = {
    open: false,
    title: '',
    message: '',
    confirmLabel: t.discardChanges(),
    cancelLabel: t.cancel(),
    saveLabel: '',
  };
  const _unsubConfirmStore = confirmDialogStore.subscribe((v) => {
    confirmDialogState = v;
  });

  let tabs: Tab[] = [];
  let activeTabId = '';
  let previewTabId: string | null = null;
  let previewOpenGeneration = 0;
  let lastMarkdownLintSignature = '';
  let lastMarkdownLintImmediateKey = '';
  const markdownLintController = createMarkdownLintController((state) => {
    markdownLintState = state;
  });

  $: scheduleMarkdownLint(
    markdownLintEnabled,
    markdownLintRuleSet,
    activeTabId,
    version,
    markdown,
    largeDocumentLimit,
    largeDocumentMode,
  );

  function getMarkdownLintInput(): MarkdownLintInput | null {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!markdownLintEnabled || !isMarkdownTab(activeTab)) return null;
    return {
      tabId: activeTabId,
      version,
      markdown,
      ruleSet: markdownLintRuleSet,
      largeDocumentLimit,
      largeDocumentMode,
    };
  }

  function scheduleMarkdownLint(
    enabled: boolean,
    ruleSet: MarkdownLintRuleSet,
    tabId: string,
    documentVersion: number,
    content: string,
    documentLimit: number,
    isLargeDocument: boolean,
  ) {
    const activeTab = tabs.find((tab) => tab.id === tabId);
    const signature =
      enabled && isMarkdownTab(activeTab)
        ? `${tabId}:${documentVersion}:${ruleSet}:${documentLimit}:${isLargeDocument}`
        : `disabled:${enabled}:${tabId}`;
    if (signature === lastMarkdownLintSignature) return;
    lastMarkdownLintSignature = signature;

    if (!enabled || !isMarkdownTab(activeTab)) {
      lastMarkdownLintImmediateKey = '';
      markdownLintController.disable();
      return;
    }

    const immediateKey = `${tabId}:${ruleSet}:${enabled}`;
    const delayMs = immediateKey === lastMarkdownLintImmediateKey ? 500 : 0;
    lastMarkdownLintImmediateKey = immediateKey;
    markdownLintController.schedule(
      {
        tabId,
        version: documentVersion,
        markdown: content,
        ruleSet,
        largeDocumentLimit: documentLimit,
        largeDocumentMode: isLargeDocument,
      },
      delayMs,
    );
  }

  function retryMarkdownLint() {
    const input = getMarkdownLintInput();
    if (input) markdownLintController.checkNow(input);
  }

  function revealMarkdownLintIssue(issue: MarkdownLintIssue): boolean {
    if (getActiveEditorMode() === 'semantic') return editor.revealMarkdownLine(issue.lineNumber);
    if (!sourceEditor) return false;

    const lineStarts = [0];
    for (let index = 0; index < markdown.length; index += 1) {
      if (markdown.charCodeAt(index) === 10) lineStarts.push(index + 1);
    }
    const lineStart = lineStarts[issue.lineNumber - 1];
    if (lineStart === undefined) return false;
    const columnOffset = Math.max(0, (issue.columnNumber ?? 1) - 1);
    const from = Math.min(markdown.length, lineStart + columnOffset);
    const to = Math.min(markdown.length, from + Math.max(1, issue.rangeLength ?? 1));
    sourceEditor.focus();
    sourceEditor.revealRange(from, to);
    return true;
  }

  /** 任何离开当前导航上下文的动作都必须让迟到的预览请求自行丢弃。 */
  function invalidatePendingPreviewOpen() {
    previewOpenGeneration += 1;
    return previewOpenGeneration;
  }

  type AppBootState = 'booting' | 'restoring-workspace' | 'opening-file' | 'ready';
  let appBootState: AppBootState = 'booting';
  let filePreviewEnabled = DEFAULT_APP_PREFERENCES.filePreviewEnabled;
  let autoSaveEnabled = DEFAULT_APP_PREFERENCES.autoSaveEnabled;
  let closeWindowBehavior = DEFAULT_APP_PREFERENCES.closeWindowBehavior;
  let externalFileChangeBehavior = DEFAULT_APP_PREFERENCES.externalFileChangeBehavior;
  let windowLabel = '';
  let lastWindowOpenTargetsSignature = '';
  let openTargetOperationQueue = Promise.resolve();
  let developerMode = DEFAULT_APP_PREFERENCES.developerMode;

  let markdownMiniActive = false;
  let markdownMiniPinned = true;
  let markdownMiniTransitioning = false;
  let markdownMiniPreviousMode: EditorViewMode | null = null;

  function hasPersistableReadingPositionPath(path: string) {
    return Boolean(desktopEnabled && path && path !== t.untitledMarkdown());
  }

  function pruneSessionReadingPositions() {
    const currentTabIds = new Set(tabs.map((tab) => tab.id));
    for (const tabId of sessionReadingPositions.keys()) {
      if (!currentTabIds.has(tabId)) {
        sessionReadingPositions.delete(tabId);
      }
    }
  }

  function getCurrentWindowOpenTargetsSnapshot() {
    const filePaths = Array.from(
      new Set(tabs.map((tab) => tab.nativePath).filter((path): path is string => Boolean(path))),
    ).sort();
    return {
      folderPath: currentFolderPath || null,
      filePaths,
    };
  }

  async function syncCurrentWindowOpenTargetsNow() {
    if (!desktopEnabled || !windowLabel || appBootState !== 'ready') return;
    const snapshot = getCurrentWindowOpenTargetsSnapshot();
    const signature = JSON.stringify([snapshot.folderPath, snapshot.filePaths]);
    await syncWindowOpenTargets(desktopEnabled, snapshot);
    lastWindowOpenTargetsSignature = signature;
  }

  $: if (desktopEnabled && windowLabel && appBootState === 'ready') {
    const snapshot = getCurrentWindowOpenTargetsSnapshot();
    const filePaths = snapshot.filePaths;
    const signature = JSON.stringify([currentFolderPath || null, filePaths]);
    if (signature !== lastWindowOpenTargetsSignature) {
      lastWindowOpenTargetsSignature = signature;
      void syncWindowOpenTargets(desktopEnabled, snapshot).catch(() => {
        if (lastWindowOpenTargetsSignature === signature) {
          lastWindowOpenTargetsSignature = '';
        }
      });
    }
  }

  // 防抖持久化工作区状态，避免每次按键都触发两次 IPC 调用 + 磁盘写入
  let _persistTimer: number | null = null;
  let _workspaceDraftTimer: number | null = null;
  let _workspaceDraftWritePromise: Promise<void> | null = null;
  const WORKSPACE_PERSIST_DEBOUNCE_MS = 500;
  const WORKSPACE_DRAFT_PERSIST_DEBOUNCE_MS = 2000;
  const workspaceDraftPersistenceCache: WorkspaceDraftPersistenceCache = new Map();
  const lastPersistedWorkspaceJsonByKey = new Map<string, string>();
  let workspaceRestoreGeneration = 0;
  let workspaceRestorePreparation: Promise<void> | null = null;
  let deferredWorkspaceRestore: Promise<void> | null = null;
  let persistAfterWorkspaceRestore = false;

  function persistWorkspaceState() {
    pruneSessionReadingPositions();
    if (!desktopEnabled || !windowLabel) return;
    // 延迟会话尚未加入 tabs 时不得写回残缺工作区；完成后会统一触发一次持久化。
    if (workspaceRestorePreparation || deferredWorkspaceRestore) {
      persistAfterWorkspaceRestore = true;
      return;
    }

    if (_persistTimer !== null) {
      window.clearTimeout(_persistTimer);
    }

    _persistTimer = window.setTimeout(() => {
      _persistTimer = null;
      persistWorkspaceStateNow().catch(() => undefined);
    }, WORKSPACE_PERSIST_DEBOUNCE_MS);
  }

  /** 立即刷新待持久化的工作区状态（用于关闭窗口/退出应用等场景） */
  async function flushPersistWorkspaceState() {
    await workspaceRestorePreparation;
    await deferredWorkspaceRestore;
    syncActiveTabMarkdownFromEditor();
    if (_persistTimer !== null) {
      window.clearTimeout(_persistTimer);
      _persistTimer = null;
    }
    if (!desktopEnabled || !windowLabel) return;
    await persistWorkspaceDraftsNow('changed');
    await persistWorkspaceStateNow({ ensureDraftIds: false });
  }

  async function persistWorkspaceStateNow(options?: { ensureDraftIds?: boolean }) {
    if (!desktopEnabled || !windowLabel) return;
    await workspaceRestorePreparation;
    await deferredWorkspaceRestore;
    syncActiveTabMarkdownFromEditor();
    if (options?.ensureDraftIds !== false) {
      await persistWorkspaceDraftsNow('missing-only');
    }
    const state = await createPersistedWorkspaceState({
      tabs,
      activeTabId,
      currentFolderPath,
      desktopEnabled,
      preservedDraftIds: getPendingStartupConflictDraftIds(),
      draftWritePolicy: 'skip',
    });
    const workspaceEntries: Record<string, unknown> = {
      [`workspaceTabs:${windowLabel}`]: state,
    };
    if (currentFolderPath) {
      workspaceEntries[`workspaceTabs:folder:${currentFolderPath}`] = state;
    }
    await updateWorkspaceStateSettings(workspaceEntries);
  }

  async function persistFolderWorkspaceState(
    folderPath: string,
    folderTabs: Tab[],
    folderActiveTabId: string,
  ) {
    if (!desktopEnabled || !folderPath || !windowLabel) return;
    const state = await createPersistedWorkspaceState({
      tabs: folderTabs,
      activeTabId: folderActiveTabId,
      currentFolderPath: folderPath,
      desktopEnabled,
      preservedDraftIds: getPendingStartupConflictDraftIds(),
      draftWritePolicy: 'missing-only',
    });
    await updateWorkspaceStateSettings({ [`workspaceTabs:folder:${folderPath}`]: state });
  }

  function schedulePersistWorkspaceDrafts() {
    if (!desktopEnabled || !windowLabel) return;
    if (_workspaceDraftTimer !== null) {
      window.clearTimeout(_workspaceDraftTimer);
    }
    _workspaceDraftTimer = window.setTimeout(() => {
      _workspaceDraftTimer = null;
      persistWorkspaceDraftsNow('changed')
        .then((result) => {
          if (result.changedDraftIds) {
            persistWorkspaceState();
          }
        })
        .catch(() => undefined);
    }, WORKSPACE_DRAFT_PERSIST_DEBOUNCE_MS);
  }

  async function persistWorkspaceDraftsNow(policy: 'changed' | 'missing-only') {
    syncActiveTabMarkdownFromEditor();
    if (policy === 'changed' && _workspaceDraftTimer !== null) {
      window.clearTimeout(_workspaceDraftTimer);
      _workspaceDraftTimer = null;
    }
    if (_workspaceDraftWritePromise) {
      await _workspaceDraftWritePromise;
    }

    let result = { changed: false, changedDraftIds: false };
    _workspaceDraftWritePromise = persistWorkspaceDrafts({
      tabs,
      desktopEnabled,
      policy,
      preservedDraftIds: getPendingStartupConflictDraftIds(),
      cache: workspaceDraftPersistenceCache,
    })
      .then((value) => {
        result = value;
      })
      .catch(() => undefined);

    try {
      await _workspaceDraftWritePromise;
      return result;
    } finally {
      _workspaceDraftWritePromise = null;
    }
  }

  async function updateWorkspaceStateSettings(entries: Record<string, unknown>) {
    const changedEntries: Record<string, unknown> = {};
    const changedJsonByKey = new Map<string, string>();

    for (const [key, value] of Object.entries(entries)) {
      const nextJson = JSON.stringify(value);
      if (lastPersistedWorkspaceJsonByKey.get(key) === nextJson) {
        continue;
      }
      changedEntries[key] = value;
      changedJsonByKey.set(key, nextJson);
    }

    if (Object.keys(changedEntries).length === 0) {
      return;
    }

    try {
      await updateAppSettings(changedEntries);
    } catch {
      return;
    }
    for (const [key, json] of changedJsonByKey.entries()) {
      lastPersistedWorkspaceJsonByKey.set(key, json);
    }
  }

  function getPendingStartupConflictDraftIds() {
    return startupDraftConflict ? new Set([startupDraftConflict.draftId]) : undefined;
  }

  async function restoreFolderWorkspaceState(folderPath: string) {
    if (!desktopEnabled || !folderPath) return;
    const settings = await listAppSettings().catch(() => []);
    const setting = settings.find((s) => s.key === `workspaceTabs:folder:${folderPath}`);
    if (!setting) return;
    const result = await migrateWorkspaceSetting(setting).catch(() => null);
    if (!result) return;
    if (result.migrated) {
      await updateAppSetting(`workspaceTabs:folder:${folderPath}`, result.state).catch(
        () => undefined,
      );
    }
    await restorePersistedWorkspaceState(result.state);
  }

  async function restorePersistedWorkspaceState(state: PersistedWorkspaceState) {
    if (typeof state.currentFolderPath === 'string' && state.currentFolderPath.length > 0) {
      currentFolderPath = state.currentFolderPath;
      startupFolderPath = state.currentFolderPath;
    }
    if (state.tabs.length === 0) return;

    const generation = ++workspaceRestoreGeneration;
    let resolveRestore: () => void = () => undefined;
    // 门禁必须在首个磁盘读取前建立，避免 500ms 持久化定时器写回半恢复状态。
    const restoreBarrier = new Promise<void>((resolve) => {
      resolveRestore = resolve;
    });
    deferredWorkspaceRestore = restoreBarrier;
    let deferredRestoreStarted = false;
    const { immediateTabs, deferredTabs } = partitionPersistedWorkspaceTabsForRestore(
      state.tabs,
      state.activeTabId,
    );
    const restoredTabs: Tab[] = [];
    try {
      for (const persistedTab of immediateTabs) {
        const restoredTab = await restorePersistedWorkspaceTab(persistedTab);
        if (restoredTab) {
          restoredTabs.push(restoredTab);
        }
        if (generation !== workspaceRestoreGeneration) {
          await discardRestoredSegmentedTabs(restoredTabs);
          return;
        }
      }

      if (restoredTabs.length === 0 && deferredTabs.length > 0) {
        const fallback = deferredTabs.shift()!;
        const restoredFallback = await restorePersistedWorkspaceTab(fallback);
        if (restoredFallback) restoredTabs.push(restoredFallback);
      }
      if (generation !== workspaceRestoreGeneration || restoredTabs.length === 0) {
        await discardRestoredSegmentedTabs(restoredTabs);
        return;
      }
      const order = new Map(state.tabs.map((tab, index) => [tab.id, index]));
      restoredTabs.sort((left, right) => order.get(left.id)! - order.get(right.id)!);
      tabs = restoredTabs;
      activeTabId = tabs.some((tab) => tab.id === state.activeTabId)
        ? state.activeTabId
        : tabs[0].id;
      const activeTab = tabs.find((tab) => tab.id === activeTabId) || tabs[0];
      activeTabId = activeTab.id;
      loadTabState(activeTab);

      if (deferredTabs.length > 0) {
        deferredRestoreStarted = true;
        void restoreDeferredWorkspaceTabs(deferredTabs, order, generation)
          .catch((error) => {
            if (generation === workspaceRestoreGeneration) {
              statusMessage = error instanceof Error ? error.message : t.openRecentFailed();
            }
          })
          .finally(() => {
            finishWorkspaceRestore(restoreBarrier, resolveRestore, generation);
          });
      }
    } finally {
      if (!deferredRestoreStarted) {
        finishWorkspaceRestore(restoreBarrier, resolveRestore, generation);
      }
    }
  }

  function finishWorkspaceRestore(
    restoreBarrier: Promise<void>,
    resolveRestore: () => void,
    generation: number,
  ) {
    resolveRestore();
    if (deferredWorkspaceRestore !== restoreBarrier) return;
    deferredWorkspaceRestore = null;
    if (generation === workspaceRestoreGeneration || persistAfterWorkspaceRestore) {
      persistAfterWorkspaceRestore = false;
      // 即使没有 deferred tab，也要以完整恢复结果纠正恢复期间被请求的持久化。
      persistWorkspaceState();
    }
  }

  async function discardRestoredSegmentedTabs(restoredTabs: Array<Tab | null>) {
    for (const restoredTab of restoredTabs) {
      if (!isSegmentedTextTab(restoredTab)) continue;
      // 导航取消不等于用户放弃恢复内容；关闭会话但保留 durable recovery。
      await segmentedDocumentPort.closeSession(restoredTab.sessionId, false).catch(() => undefined);
      segmentedSessionRegistry.delete(restoredTab.sessionId);
    }
  }

  async function cancelDeferredWorkspaceRestore() {
    await workspaceRestorePreparation;
    workspaceRestoreGeneration += 1;
    const pendingRestore = deferredWorkspaceRestore;
    if (pendingRestore) {
      await pendingRestore;
    }
  }

  function beginWorkspaceRestorePreparation() {
    let resolvePreparation: () => void = () => undefined;
    const preparation = new Promise<void>((resolve) => {
      resolvePreparation = resolve;
    });
    workspaceRestorePreparation = preparation;
    return () => {
      if (workspaceRestorePreparation === preparation) {
        workspaceRestorePreparation = null;
      }
      resolvePreparation();
      if (
        !workspaceRestorePreparation &&
        !deferredWorkspaceRestore &&
        persistAfterWorkspaceRestore
      ) {
        persistAfterWorkspaceRestore = false;
        persistWorkspaceState();
      }
    };
  }

  async function restoreDeferredWorkspaceTabs(
    persistedTabs: PersistedWorkspaceTab[],
    order: Map<string, number>,
    generation: number,
  ) {
    for (const persistedTab of persistedTabs) {
      const restoredTab = await restorePersistedWorkspaceTab(persistedTab);
      if (generation !== workspaceRestoreGeneration) {
        await discardRestoredSegmentedTabs([restoredTab]);
        return;
      }
      if (!restoredTab || tabs.some((tab) => tab.id === restoredTab.id)) continue;
      tabs = [...tabs, restoredTab].sort(
        (left, right) =>
          (order.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
          (order.get(right.id) ?? Number.MAX_SAFE_INTEGER),
      );
      // 每次让出事件循环，确保活动文档可先完成挂载和首屏绘制。
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    }
  }

  async function restorePersistedWorkspaceTab(
    persistedTab: PersistedWorkspaceTab,
  ): Promise<Tab | null> {
    if (persistedTab.documentKind !== 'markdown') {
      if (!desktopEnabled || !persistedTab.nativePath) {
        return null;
      }
      try {
        const opened = await segmentedDocumentPort.open(persistedTab.nativePath);
        if (opened.documentKind !== persistedTab.documentKind) {
          await segmentedDocumentPort.closeSession(opened.sessionId, false).catch(() => undefined);
          statusMessage = t.openRecentFailed();
          return null;
        }
        segmentedSessionRegistry.register(opened);
        if (opened.recoveryConflictPath) {
          statusMessage = t.segmentedRecoveryConflict({ path: opened.recoveryConflictPath });
        }
        return createRuntimeTabFromPersisted(
          persistedTab,
          {
            sessionId: opened.sessionId,
            revision: opened.revision,
            persistedRevision: opened.persistedRevision,
            indexProgress: opened.firstWindow.indexProgress,
            readonly: opened.readonly,
            recoveryConflictPath: opened.recoveryConflictPath ?? null,
          },
          { diskReadonly: opened.filesystemReadonly ?? false },
        );
      } catch (error) {
        showVisibleError(error, t.openRecentFailed());
        return null;
      }
    }

    const draft = persistedTab.draftId
      ? await readWorkspaceDraft(persistedTab.draftId).catch(() => null)
      : null;

    if (!persistedTab.nativePath) {
      if (!draft) {
        return createRuntimeTabFromPersisted(persistedTab, '', {
          savedMarkdown: '',
          dirty: false,
          lastKnownModifiedAt: 0,
          largeDocumentMode: false,
          readonlyDocumentMode: false,
          diskReadonly: false,
        });
      }
      return createRuntimeTabFromPersisted(persistedTab, draft.markdown, {
        savedMarkdown: '',
        dirty: true,
        lastKnownModifiedAt: 0,
        largeDocumentMode: draft.markdown.length > largeDocumentLimit,
        readonlyDocumentMode: draft.markdown.length > largeDocumentLimit,
        diskReadonly: false,
      });
    }

    const { document, error } = await readMarkdownFromPath(
      persistedTab.nativePath,
      t.openRecentFailed(),
    );
    if (!document) {
      if (draft) {
        statusMessage = error || t.openRecentFailed();
        return createRuntimeTabFromPersisted(persistedTab, draft.markdown, {
          savedMarkdown: '',
          dirty: true,
          largeDocumentMode: draft.markdown.length > largeDocumentLimit,
          readonlyDocumentMode: draft.markdown.length > largeDocumentLimit,
          // 磁盘读取失败时无法确认旧工作区文件的真实编码，禁止覆盖原路径并要求另存为。
          diskReadonly: true,
        });
      }
      statusMessage = error || t.openRecentFailed();
      return createRuntimeTabFromPersisted(persistedTab, '', {
        savedMarkdown: '',
        dirty: false,
        largeDocumentMode: false,
        readonlyDocumentMode: true,
        diskReadonly: false,
      });
    }

    const diskLargeDocument =
      document.markdown.length > largeDocumentLimit || document.sizeBytes > largeDocumentLimit;
    if (!draft) {
      return createRuntimeTabFromPersisted(persistedTab, document.markdown, {
        savedMarkdown: document.markdown,
        encoding: document.encoding,
        dirty: false,
        lastKnownModifiedAt: document.modifiedAt,
        largeDocumentMode: diskLargeDocument,
        readonlyDocumentMode: diskLargeDocument,
        diskReadonly: document.readonly,
      });
    }

    const changedOnDisk =
      persistedTab.lastKnownModifiedAt > 0 &&
      document.modifiedAt !== persistedTab.lastKnownModifiedAt;
    if (changedOnDisk) {
      const diskTab = createRuntimeTabFromPersisted(persistedTab, document.markdown, {
        savedMarkdown: document.markdown,
        encoding: document.encoding,
        dirty: false,
        lastKnownModifiedAt: document.modifiedAt,
        largeDocumentMode: diskLargeDocument,
        readonlyDocumentMode: diskLargeDocument,
        diskReadonly: document.readonly,
      });
      queueStartupDraftConflict({
        tabId: persistedTab.id,
        fileName: persistedTab.fileName,
        filePath: persistedTab.filePath,
        nativePath: persistedTab.nativePath,
        draftId: draft.draftId,
        draftMarkdown: draft.markdown,
        diskMarkdown: document.markdown,
        diskModifiedAt: document.modifiedAt,
        diskReadonly: document.readonly,
        diskLargeDocumentMode: diskLargeDocument,
      });
      return diskTab;
    }

    return createRuntimeTabFromPersisted(persistedTab, draft.markdown, {
      savedMarkdown: document.markdown,
      encoding: document.encoding,
      dirty: true,
      lastKnownModifiedAt: document.modifiedAt,
      largeDocumentMode: draft.markdown.length > largeDocumentLimit,
      readonlyDocumentMode: draft.markdown.length > largeDocumentLimit,
      diskReadonly: document.readonly,
    });
  }

  function queueStartupDraftConflict(conflict: StartupDraftConflict) {
    if (!startupDraftConflict) {
      startupDraftConflict = conflict;
    }
  }

  function applyStartupConflictDraft() {
    const conflict = startupDraftConflict;
    if (!conflict) return;
    const targetTab = tabs.find((tab) => tab.id === conflict.tabId);
    if (!targetTab) {
      startupDraftConflict = null;
      return;
    }

    Object.assign(targetTab, {
      markdown: conflict.draftMarkdown,
      savedMarkdown: conflict.diskMarkdown,
      dirty: true,
      draftId: conflict.draftId,
      lastKnownModifiedAt: conflict.diskModifiedAt,
      largeDocumentMode: conflict.draftMarkdown.length > largeDocumentLimit,
      readonlyDocumentMode: conflict.draftMarkdown.length > largeDocumentLimit,
      diskReadonly: conflict.diskReadonly,
    });
    tabs = [...tabs];
    switchTab(conflict.tabId);
    startupDraftConflict = null;
    persistWorkspaceState();
  }

  function applyStartupConflictDiskVersion() {
    const conflict = startupDraftConflict;
    if (!conflict) return;
    const targetTab = tabs.find((tab) => tab.id === conflict.tabId);
    if (targetTab) {
      Object.assign(targetTab, {
        markdown: conflict.diskMarkdown,
        savedMarkdown: conflict.diskMarkdown,
        dirty: false,
        draftId: null,
        lastKnownModifiedAt: conflict.diskModifiedAt,
        largeDocumentMode: conflict.diskLargeDocumentMode,
        readonlyDocumentMode: conflict.diskLargeDocumentMode,
        diskReadonly: conflict.diskReadonly,
      });
      tabs = [...tabs];
      loadTabState(targetTab);
    }
    deleteWorkspaceDraft(conflict.draftId).catch(() => undefined);
    startupDraftConflict = null;
    persistWorkspaceState();
  }

  async function saveStartupConflictDraftAs() {
    const conflict = startupDraftConflict;
    if (!conflict) return;
    applyStartupConflictDraft();
    await tick();
    await saveMarkdownFile(true);
  }

  function flushActiveEditorView() {
    if (mode === 'split' && splitActivePane === 'source') {
      refreshSplitSemanticView();
    }
  }

  function syncActiveTabMarkdownFromEditor() {
    if (!activeTabId) return markdown;
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isMarkdownTab(activeTab)) {
      // 分段标签的正文只存在于 Rust session 与局部 CodeMirror window，禁止触发 Markdown flush。
      return markdown;
    }
    flushActiveEditorView();
    const currentMarkdown = editor.flushMarkdown();
    if (currentMarkdown !== markdown) {
      markdown = currentMarkdown;
    }
    return currentMarkdown;
  }

  async function toggleMarkdownMini() {
    if (markdownMiniActive) {
      await requestMarkdownMiniReturn();
      return;
    }
    await enterCurrentWindowMarkdownMini();
  }

  async function enterCurrentWindowMarkdownMini() {
    if (markdownMiniActive || markdownMiniTransitioning || !desktopEnabled || !windowLabel) return;

    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isMarkdownTab(activeTab)) return;

    syncActiveTabMarkdownFromEditor();
    markdownMiniTransitioning = true;
    markdownMiniPreviousMode = mode;
    activeMenu = null;
    closeToolbarTransientPanels();
    if (searchPanelOpen) closeSearchPanel();
    if (externalChangeDialogOpen) closeExternalChangeDialog();
    frontMatterEditing = false;
    markdownMiniActive = true;

    const shouldUseSemanticEditor = !largeDocumentMode && mode !== 'semantic';
    const nativeTransition = enterMarkdownMiniMode(desktopEnabled, markdownMiniPinned);

    try {
      await tick();
      await Promise.all([
        nativeTransition,
        shouldUseSemanticEditor ? changeEditorMode('semantic', false) : Promise.resolve(true),
      ]);
      requestAnimationFrame(() => {
        refreshEditorViewportLayout();
        if (!largeDocumentMode && !readonlyDocumentMode) editor.focus();
      });
    } catch (error) {
      await exitMarkdownMiniMode(desktopEnabled).catch(() => undefined);
      markdownMiniActive = false;
      const previousMode = markdownMiniPreviousMode;
      markdownMiniPreviousMode = null;
      if (previousMode && mode !== previousMode) {
        await changeEditorMode(previousMode, false);
      }
      showVisibleError(error, 'Markdown 小窗打开失败');
    } finally {
      markdownMiniTransitioning = false;
      await tick();
      refreshEditorViewportLayout();
    }
  }

  async function toggleMarkdownMiniPinned() {
    if (!markdownMiniActive || markdownMiniTransitioning) return;
    const nextPinned = !markdownMiniPinned;
    try {
      await setMarkdownMiniModePinned(desktopEnabled, nextPinned);
      markdownMiniPinned = nextPinned;
    } catch (error) {
      showVisibleError(error, 'Markdown 小窗置顶状态切换失败');
    }
  }
  async function requestMarkdownMiniReturn(options?: { showExternalChange?: boolean }) {
    if (!markdownMiniActive) return true;
    if (markdownMiniTransitioning) return false;
    markdownMiniTransitioning = true;
    const previousMode = markdownMiniPreviousMode;

    try {
      await Promise.all([
        exitMarkdownMiniMode(desktopEnabled),
        previousMode && mode !== previousMode
          ? changeEditorMode(previousMode, false)
          : Promise.resolve(true),
      ]);
      markdownMiniActive = false;
      markdownMiniPreviousMode = null;
      await tick();
      refreshEditorViewportLayout();
      if (getActiveEditorMode() === 'semantic' && !readonlyDocumentMode) editor.focus();

      if (options?.showExternalChange !== false && externalFileChange.type !== 'none') {
        openExternalChangeDialog(externalFileChange);
      }
      return true;
    } catch (error) {
      showVisibleError(error, 'Markdown 小窗返回失败');
      return false;
    } finally {
      markdownMiniTransitioning = false;
    }
  }
  // 保存当前活跃 Tab 的状态
  function saveActiveTabState() {
    if (!activeTabId) return;

    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (isSegmentedTextTab(activeTab)) {
      activeTab.dirty = dirty;
      activeTab.lastKnownModifiedAt = lastKnownModifiedAt;
      activeTab.diskReadonly = diskReadonly;
      activeTab.externalFileChange = externalFileChange;
      tabs = [...tabs];
      persistWorkspaceState();
      return;
    }

    const currentMarkdown = syncActiveTabMarkdownFromEditor();
    saveCurrentReadingPositionToMemoryOnly();

    tabs = writeActiveTabState(tabs, activeTabId, {
      markdown: currentMarkdown,
      savedMarkdown: dirty ? savedMarkdown : currentMarkdown,
      dirty,
      version,
      fileName,
      filePath,
      nativePath,
      largeDocumentMode,
      readonlyDocumentMode,
      diskReadonly,
      externalFileChange,
      lastKnownModifiedAt,
    });
    persistWorkspaceState();
  }

  function getCurrentReadingAnchor(modeToSave: ReadingPositionMode): OutlineScrollAnchor | null {
    if (modeToSave === 'semantic') {
      return semanticPane
        ? getSemanticScrollAnchor(outline, semanticPane, semanticPane.scrollTop)
        : null;
    }
    if (!sourcePane) {
      return null;
    }
    return getSourceScrollAnchor(
      outline,
      sourcePane.scrollTop,
      getSourceLineHeight(),
      sourceEditor,
      sourcePane,
    );
  }

  function getActiveEditorMode(): EditorMode {
    return mode === 'split' ? splitActivePane : mode;
  }

  function getCoreModeForView(nextMode: EditorViewMode): EditorMode {
    return nextMode === 'source' ? 'source' : 'semantic';
  }

  function clearSplitSemanticRefreshTimer() {
    if (splitSemanticRefreshTimer !== null) {
      window.clearTimeout(splitSemanticRefreshTimer);
      splitSemanticRefreshTimer = null;
    }
  }

  function refreshSplitSemanticView() {
    clearSplitSemanticRefreshTimer();
    const generation = ++splitSemanticRefreshGeneration;
    if (mode !== 'split') return;

    editor.refreshSemanticView();
    void tick().then(() => {
      if (mode !== 'split' || generation !== splitSemanticRefreshGeneration) return;
      const editorGrid =
        sourcePane?.closest<HTMLElement>('.editor-grid') ??
        semanticPane?.closest<HTMLElement>('.editor-grid');
      editorGrid?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
    });
  }

  function scheduleSplitSemanticRefresh() {
    if (mode !== 'split') return;
    clearSplitSemanticRefreshTimer();
    const generation = ++splitSemanticRefreshGeneration;
    splitSemanticRefreshTimer = window.setTimeout(() => {
      splitSemanticRefreshTimer = null;
      if (mode !== 'split' || generation !== splitSemanticRefreshGeneration) return;
      refreshSplitSemanticView();
    }, SPLIT_SEMANTIC_REFRESH_DEBOUNCE_MS);
  }

  function setSplitActivePane(nextPane: SplitActivePane) {
    if (mode !== 'split' || splitActivePane === nextPane) return;

    if (nextPane === 'source') {
      // ProseMirror 的序列化是延迟的；源码区接管前必须先取得最新 Markdown。
      const latestMarkdown = editor.getMarkdown();
      // 是否有变化由源码编辑器按相同换行规则判断；切栏不制造全文替换。
      sourceEditor?.setMarkdown(latestMarkdown, { addToHistory: false });
    } else {
      // 源码输入会延迟重建语义 DOM；语义区接管前强制刷新，避免旧 DOM 覆盖新内容。
      refreshSplitSemanticView();
    }
    splitActivePane = nextPane;
  }

  function updateSplitLeftPercent(nextPercent: number, persist: boolean) {
    splitLeftPercent = Math.min(75, Math.max(25, Math.round(nextPercent * 10) / 10));
    if (persist) {
      void updateAppSetting('splitLeftPercent', splitLeftPercent).catch(() => undefined);
    }
  }

  function getReadingPositionForTab(
    tab: MarkdownTabState,
    preferredMode: ReadingPositionMode,
  ): ReadingPosition | undefined {
    if (hasPersistableReadingPositionPath(tab.filePath)) {
      sessionReadingPositions.delete(tab.id);
      return getReadingPosition(tab.filePath, preferredMode);
    }
    return sessionReadingPositions.get(tab.id);
  }

  function saveReadingPositionForTab(
    tab: MarkdownTabState,
    modeToSave: ReadingPositionMode,
    anchor: OutlineScrollAnchor | null,
    persist: boolean,
  ) {
    if (hasPersistableReadingPositionPath(tab.filePath)) {
      if (persist) {
        saveReadingPositionToMemory(tab.filePath, modeToSave, anchor);
      } else {
        saveReadingPositionToMemoryOnly(tab.filePath, modeToSave, anchor);
      }
      return;
    }

    sessionReadingPositions.set(tab.id, {
      anchor,
      anchorMode: modeToSave,
      updatedAt: Date.now(),
    });
  }

  function saveCurrentReadingPositionToMemoryOnly(
    modeToSave: ReadingPositionMode = getActiveEditorMode(),
    anchor: OutlineScrollAnchor | null = getCurrentReadingAnchor(modeToSave),
  ) {
    const activeTab = tabs.find(
      (tab): tab is MarkdownTabState => tab.id === activeTabId && isMarkdownTab(tab),
    );
    if (!activeTab) return;
    saveReadingPositionForTab(activeTab, modeToSave, anchor, false);
  }

  function clearReadingPositionSaveTimer() {
    if (scrollDebounceTimer !== null) {
      window.clearTimeout(scrollDebounceTimer);
      scrollDebounceTimer = null;
    }
  }

  async function flushCurrentReadingPosition() {
    saveCurrentReadingPositionToMemoryOnly();
    await flushReadingPositions();
  }

  let isSwitchingTab = false;

  // 加载指定 Tab 的状态并更新编辑器
  function loadTabState(tab: Tab) {
    clearSplitSemanticRefreshTimer();
    splitSemanticRefreshGeneration += 1;
    clearReadingPositionSaveTimer();
    cancelPendingReadingPositionRestore();
    selectedStats = null;
    isSwitchingTab = true;
    try {
      dirty = tab.dirty;
      fileName = tab.fileName;
      filePath = tab.filePath;
      nativePath = tab.nativePath;
      diskReadonly = tab.diskReadonly;
      externalFileChange = normalizeExternalFileChange(tab.externalFileChange);
      tab.externalFileChange = externalFileChange;
      lastKnownModifiedAt = tab.lastKnownModifiedAt;

      if (isSegmentedTextTab(tab)) {
        // 只重置 Markdown 派生 UI，不读取或分析 TXT/JSON 正文；实际窗口由分段工作区按需加载。
        markdown = '';
        savedMarkdown = '';
        version = 0;
        largeDocumentMode = false;
        readonlyDocumentMode = tab.diskReadonly;
        outline = [];
        activeOutlineId = '';
        if (tab.recoveryConflictPath) {
          statusMessage = t.segmentedRecoveryConflict({ path: tab.recoveryConflictPath });
        }
        return;
      }

      markdown = tab.markdown;
      savedMarkdown = tab.savedMarkdown ?? tab.markdown;
      version = tab.version;
      largeDocumentMode = tab.largeDocumentMode;
      readonlyDocumentMode = tab.readonlyDocumentMode;
      const nextViewMode: EditorViewMode = largeDocumentMode ? 'source' : preferredEditorMode;
      const nextReadingMode: ReadingPositionMode =
        nextViewMode === 'split' ? splitActivePane : nextViewMode;
      const storedPosition = getReadingPositionForTab(tab, nextReadingMode);
      const restoreGeneration = readingPositionRestoreGeneration;

      if (editor) {
        editor.updateOptions({
          readonly: readonlyDocumentMode,
          mode: getCoreModeForView(nextViewMode),
        });
        mode = nextViewMode;
        editor.setMarkdown(markdown, {
          reason: 'switch-tab',
          dirty: tab.dirty,
          savedMarkdown,
        });
      }

      // 步骤：先归零，避免新标签继承旧标签超出范围的 scrollTop；布局稳定后再恢复自身锚点。
      // editor.setMarkdown() 更新了 DOM 内容，但 scrollTop 仍保留旧标签页的值。
      // 若 scrollTop 远超新内容的 scrollHeight，macOS WebKit 会渲染空白页，
      // Windows Chromium 则显示在底部。
      setProgrammaticReadingScrollTop('semantic', semanticPane, 0);
      setProgrammaticReadingScrollTop('source', sourcePane, 0);

      const analysis = analyzeMarkdown(markdown);
      outline = analysis.outline;
      activeOutlineId = outline[0]?.id ?? '';
      applyOutlineDefaultExpansion();
      stats = analysis.stats;
      syncSourceTextareaHeight();
      scheduleReadingPositionRestore(tab, nextReadingMode, storedPosition, restoreGeneration);
    } finally {
      isSwitchingTab = false;
    }
  }

  // 切换活动标签页
  let tabSwitchInProgress = false;

  async function switchTab(tabId: string) {
    invalidatePendingPreviewOpen();
    if (!tabId || activeTabId === tabId) return;
    if (tabSwitchInProgress) return;
    if (markdownMiniActive) {
      if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
      if (activeTabId === tabId) return;
    }
    // 冲突对话框只属于发起检查的活动标签，切换后不能让操作落到另一文档。
    closeExternalChangeDialog();
    tabSwitchInProgress = true;
    try {
      const currentTab = tabs.find((tab) => tab.id === activeTabId);
      // 切换前必须把当前 CodeMirror 增量和恢复日志都落到 Rust，不能让隐藏组件持有未提交正文。
      await flushSegmentedDocumentBeforeTransition(
        currentTab,
        segmentedWorkspace,
        segmentedDocumentPort,
      );
      saveActiveTabState();
      if (isMarkdownTab(currentTab)) {
        void flushReadingPositions();
      }
      const targetTab = tabs.find((tab) => tab.id === tabId);
      if (targetTab) {
        activeTabId = tabId;
        persistWorkspaceState();
        loadTabState(targetTab);
        updateWindowTitle();
        // 切换后若标签关联了本地文件，展开资源管理器中对应的文件夹路径
        if (targetTab.nativePath && currentFolderPath) {
          expandAncestors(targetTab.nativePath, currentFolderPath);
        }
      }
    } catch (error) {
      showVisibleError(error, t.saveFileFailed());
    } finally {
      tabSwitchInProgress = false;
    }
  }

  function handleSemanticScroll() {
    if (
      programmaticReadingScrollTokens.has('semantic') ||
      semanticPane?.dataset.nomoSyncScroll === 'true'
    ) {
      return;
    }
    cancelPendingReadingPositionRestore();
    debounceReadingPositionSave('semantic');
  }

  function handleSourceScroll() {
    if (
      programmaticReadingScrollTokens.has('source') ||
      sourcePane?.dataset.nomoSyncScroll === 'true'
    ) {
      return;
    }
    cancelPendingReadingPositionRestore();
    debounceReadingPositionSave('source');
  }

  function debounceReadingPositionSave(modeToSave: ReadingPositionMode) {
    const activeTab = tabs.find(
      (tab): tab is MarkdownTabState => tab.id === activeTabId && isMarkdownTab(tab),
    );
    if (!activeTab) return;
    const anchor = getCurrentReadingAnchor(modeToSave);
    saveReadingPositionForTab(activeTab, modeToSave, anchor, false);
    clearReadingPositionSaveTimer();
    if (!hasPersistableReadingPositionPath(activeTab.filePath)) return;

    const targetPath = activeTab.filePath;
    scrollDebounceTimer = window.setTimeout(() => {
      scrollDebounceTimer = null;
      saveReadingPositionToMemory(targetPath, modeToSave, anchor);
    }, 1500);
  }

  function cancelPendingReadingPositionRestore() {
    readingPositionRestoreGeneration += 1;
    pendingReadingPositionRestore = null;
  }

  function setProgrammaticReadingScrollTop(
    targetMode: ReadingPositionMode,
    pane: HTMLElement | undefined,
    scrollTop: number,
  ) {
    if (!pane) return;
    suppressProgrammaticReadingScroll(targetMode);
    setScrollTop(pane, scrollTop);
  }

  function suppressProgrammaticReadingScroll(targetMode: ReadingPositionMode) {
    const token = (programmaticReadingScrollTokens.get(targetMode) ?? 0) + 1;
    programmaticReadingScrollTokens.set(targetMode, token);
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (programmaticReadingScrollTokens.get(targetMode) === token) {
          programmaticReadingScrollTokens.delete(targetMode);
        }
      });
    });
  }

  function scheduleReadingPositionRestore(
    tab: MarkdownTabState,
    targetMode: ReadingPositionMode,
    position: ReadingPosition | undefined,
    generation: number,
    attemptsRemaining = 120,
  ) {
    const anchor = position?.anchor;
    if (!anchor) return;

    pendingReadingPositionRestore = {
      generation,
      tabId: tab.id,
      filePath: tab.filePath,
      mode: targetMode,
    };

    const tryRestore = async (remainingAttempts: number) => {
      await tick();
      await waitForAnimationFrame();

      const pending = pendingReadingPositionRestore;
      const targetStillActive =
        pending?.generation === generation &&
        readingPositionRestoreGeneration === generation &&
        pending.tabId === tab.id &&
        pending.filePath === tab.filePath &&
        pending.mode === targetMode &&
        activeTabId === tab.id &&
        filePath === tab.filePath &&
        getActiveEditorMode() === targetMode;
      if (!targetStillActive) {
        if (pending?.generation === generation) {
          pendingReadingPositionRestore = null;
        }
        return;
      }

      const pane = targetMode === 'semantic' ? semanticPane : sourcePane;
      const contentReady =
        targetMode === 'semantic'
          ? Boolean(semanticPane?.querySelector('.ProseMirror'))
          : Boolean(sourceEditor && sourceEditor.getMarkdown() === markdown);
      const expectsNonTop =
        anchor.documentProgress > 0.001 ||
        anchor.sourceLine > 1 ||
        (typeof anchor.scrollTop === 'number' && anchor.scrollTop > 1) ||
        (typeof anchor.offsetFromTop === 'number' && Math.abs(anchor.offsetFromTop) > 1);
      const layoutReady =
        pane != null && (!expectsNonTop || Math.max(0, pane.scrollHeight - pane.clientHeight) > 0);

      if (!pane || !contentReady || !layoutReady) {
        if (remainingAttempts > 0) {
          requestAnimationFrame(() => void tryRestore(remainingAttempts - 1));
        } else {
          pendingReadingPositionRestore = null;
        }
        return;
      }

      pendingReadingPositionRestore = null;
      const restore = () => {
        if (targetMode === 'semantic') {
          restoreSemanticReadingPosition(outline, semanticPane, anchor, {
            anchorMode: position.anchorMode,
            behavior: 'instant',
          });
          return;
        }
        restoreSourceReadingPosition(outline, sourcePane, sourceEditor, anchor, {
          anchorMode: position.anchorMode,
          behavior: 'instant',
        });
      };
      suppressProgrammaticReadingScroll(targetMode);
      restore();
    };

    requestAnimationFrame(() => void tryRestore(attemptsRemaining));
  }

  // 顶级目录展开与收起状态
  let rootFolderExpanded = true;
  const folderExplorer = createFolderExplorerController({
    getDesktopEnabled: () => desktopEnabled,
    getFolderTree: () => folderTree,
    setFolderTree: (value) => {
      folderTree = value;
    },
    getExpandedFolders: () => expandedFolders,
    setExpandedFolders: (value) => {
      expandedFolders = value;
    },
    getRootFolderExpanded: () => rootFolderExpanded,
    setRootFolderExpanded: (value) => {
      rootFolderExpanded = value;
    },
    getCurrentFolderPath: () => currentFolderPath,
    setCurrentFolderPath: (value) => {
      currentFolderPath = value;
      persistWorkspaceState();
    },
    setStatusMessage: (value) => {
      statusMessage = value;
    },
  });
  const expandAncestors = folderExplorer.expandAncestors;
  const toggleFolderCollapse = folderExplorer.toggleFolderCollapse;
  const toggleRootFolder = folderExplorer.toggleRootFolder;
  const removeMissingExplorerPaths = folderExplorer.removeMissingPaths;
  const syncLoadedExplorerFolders = folderExplorer.syncLoadedFolders;

  // 侧边栏宽度拉伸状态与函数
  let sidebarWidth = 250;
  let isResizing = false;
  const sidebarResize = createSidebarResizeHandlers({
    setResizing: (value) => {
      isResizing = value;
    },
    setSidebarWidth: (value) => {
      sidebarWidth = value;
    },
  });
  const startResize = sidebarResize.startResize;

  let activeMenu: string | null = null;

  function toggleMenu(menu: string) {
    activeMenu = getNextActiveMenu(activeMenu, menu);
  }

  function closeMenu(menu: string) {
    activeMenu = closeActiveMenu(activeMenu, menu);
  }

  const exitApp = () => requestExitApp();
  const createNewWindow = () => createAppWindow(desktopEnabled);

  function resolveFolderName(path: string): string {
    const normalized = path.replace(/\\/g, '/').replace(/\/$/, '');
    const idx = normalized.lastIndexOf('/');
    return idx >= 0 ? normalized.slice(idx + 1) || path : path;
  }

  function sameFileSystemPath(left: string, right: string) {
    return (
      left.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase() ===
      right.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase()
    );
  }

  // 步骤：关闭全部标签页前统一确认未保存内容，确认后不自动创建空白标签。
  async function closeAllTabsWithConfirmation(options?: { skipPersist?: boolean }) {
    invalidatePendingPreviewOpen();
    const dirtyTabs = getDirtyTabs(tabs);
    let discardChanges = false;
    if (dirtyTabs.length > 0) {
      const names = dirtyTabs.map((t) => t.fileName).join('、');
      const ok = await confirmAction(t.unsavedChangesCloseTabs({ names }));
      if (ok === false) return false;
      discardChanges = true;
    }

    invalidatePendingPreviewOpen();
    // 先让在途恢复 open 收口并清理，再取最终 tabs 快照关闭会话。
    await cancelDeferredWorkspaceRestore();
    await closeAllSegmentedSessions(discardChanges);
    clearAllTabsWithoutCreatingBlank(options);
    return true;
  }

  async function flushAllSegmentedSessions() {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (isSegmentedTextTab(activeTab)) {
      await segmentedWorkspace?.flushPendingEdits();
    }
    for (const tab of tabs) {
      if (isSegmentedTextTab(tab)) {
        await segmentedDocumentPort.flushJournal(tab.sessionId, tab.revision);
      }
    }
  }

  async function closeAllSegmentedSessions(discardDirty: boolean) {
    await closeSegmentedSessions(tabs, discardDirty);
  }

  async function closeSegmentedSessions(candidateTabs: Tab[], discardDirty: boolean) {
    const activeCandidate = candidateTabs.find((tab) => tab.id === activeTabId);
    if (isSegmentedTextTab(activeCandidate)) {
      await segmentedWorkspace?.flushPendingEdits();
    }
    for (const tab of candidateTabs) {
      if (!isSegmentedTextTab(tab)) continue;
      await segmentedDocumentPort.flushJournal(tab.sessionId, tab.revision);
      await segmentedDocumentPort.closeSession(tab.sessionId, discardDirty && tab.dirty);
      if (discardDirty && tab.dirty) {
        tab.dirty = false;
        tab.revision = tab.persistedRevision;
      }
      segmentedSessionRegistry.delete(tab.sessionId);
    }
  }

  function clearAllTabsWithoutCreatingBlank(options?: { skipPersist?: boolean }) {
    invalidatePendingPreviewOpen();
    cancelPendingReadingPositionRestore();
    sessionReadingPositions.clear();
    closeExternalChangeDialog();
    workspaceRestoreGeneration += 1;
    isSwitchingTab = true;
    try {
      tabs = [];
      activeTabId = '';
      previewTabId = null;
      markdown = '';
      savedMarkdown = '';
      fileName = '';
      filePath = '';
      nativePath = null;
      dirty = false;
      version = 0;
      lastKnownModifiedAt = 0;
      largeDocumentMode = false;
      readonlyDocumentMode = false;
      diskReadonly = false;
      externalFileChange = createEmptyExternalFileChange();
      outline = [];
      if (editor) {
        editor.setMarkdown('', { reason: 'switch-tab', dirty: false, savedMarkdown: '' });
      }
    } finally {
      isSwitchingTab = false;
    }
    updateWindowTitle();
    if (!options?.skipPersist) {
      persistWorkspaceState();
    }
  }

  async function openFolderInCurrentWindow(folderPath: string) {
    if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
    if (!currentFolderPath || !sameFileSystemPath(currentFolderPath, folderPath)) {
      // 切换文件夹前保存当前文件夹状态，避免清空标签后的空状态覆盖已有记录
      if (currentFolderPath && tabs.length > 0) {
        await persistFolderWorkspaceState(currentFolderPath, tabs, activeTabId).catch(
          () => undefined,
        );
      }
      if (!(await closeAllTabsWithConfirmation({ skipPersist: true }))) {
        return;
      }
    }
    const finishRestorePreparation = beginWorkspaceRestorePreparation();
    try {
      currentFolderPath = folderPath;
      await loadFolder(folderPath);
      await restoreFolderWorkspaceState(folderPath);
      await rememberNativeFolder(folderPath);
      await refreshRecentFiles();
    } finally {
      finishRestorePreparation();
    }
  }

  function isReusableInitialWindow() {
    if (
      appBootState !== 'ready' ||
      currentFolderPath ||
      workspaceRestorePreparation ||
      deferredWorkspaceRestore ||
      isSwitchingTab ||
      markdownMiniActive ||
      markdownMiniTransitioning ||
      dirty ||
      nativePath
    ) {
      return false;
    }
    if (tabs.length === 0) {
      return true;
    }
    return tabs.length === 1 && isReusableUntitledTab(tabs[0]);
  }

  async function openTargetInCurrentWindow(target: OpenTarget) {
    if (target.kind === 'folder') {
      await openFolderInCurrentWindow(target.path);
      return;
    }
    for (const path of target.paths) {
      await openFilePathInCurrentWindow(path);
    }
  }

  function enqueueOpenTargetOperation<T>(operation: () => Promise<T>) {
    const result = openTargetOperationQueue.then(operation);
    openTargetOperationQueue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  function openTargetWithBehavior(target: OpenTarget) {
    return enqueueOpenTargetOperation(() =>
      routeOpenTarget(target, {
        syncTargets: syncCurrentWindowOpenTargetsNow,
        prepare: (remaining, create) => prepareOpenTargetWindow(desktopEnabled, remaining, create),
        activateCurrent: () => activateDocumentWindow(desktopEnabled),
        openCurrent: openTargetInCurrentWindow,
        createWindow: async (label) => {
          const created = await createAppWindow(desktopEnabled, label);
          if (!created) {
            // 接收窗口不再提前显示；创建失败时仍需让用户看到原有错误提示。
            await activateDocumentWindow(desktopEnabled);
            statusMessage = target.kind === 'folder' ? t.loadFolderTreeFailed() : t.openFileFailed();
          }
        },
        isReusableInitialWindow,
        getBehavior: () => openDefaultBehavior,
        requestChoice: requestOpenTargetChoice,
        rememberBehavior: async (behavior) => {
          openDefaultBehavior = behavior;
          await updateAppSetting('openDefaultBehavior', behavior).catch(() => undefined);
        },
      }),
    );
  }

  function openFolderWithBehavior(folderPath: string) {
    return openTargetWithBehavior({ kind: 'folder', path: folderPath });
  }

  function requestOpenTargetChoice(target: OpenTarget) {
    pendingOpenChoice = target;
    return new Promise<OpenTargetChoiceResult>((resolve) => {
      pendingOpenChoiceResolver = resolve;
    });
  }

  function resolveOpenTargetChoice(result: OpenTargetChoiceResult) {
    const resolve = pendingOpenChoiceResolver;
    pendingOpenChoice = null;
    pendingOpenChoiceResolver = null;
    resolve?.(result);
  }

  function handleOpenTargetChoice(
    event: CustomEvent<{ choice: 'current-window' | 'new-window'; remember: boolean }>,
  ) {
    resolveOpenTargetChoice(event.detail);
  }

  function getOpenTargetDialogPath(target: OpenTarget | null) {
    if (!target) return '';
    return target.kind === 'folder' ? target.path : target.paths.join('\n');
  }

  function getOpenTargetDialogName(target: OpenTarget | null) {
    if (!target) return '';
    if (target.kind === 'folder') return resolveFolderName(target.path);
    const firstName = target.paths[0]?.replace(/\\/g, '/').split('/').pop() ?? '';
    return target.paths.length === 1 ? firstName : `${firstName}…`;
  }

  async function openFolderDialog() {
    if (!desktopEnabled) return;
    const { folderPath, error } = await pickFolderPath();
    if (error) {
      statusMessage = error;
    }
    if (folderPath) {
      await openFolderWithBehavior(folderPath);
    }
  }

  async function openRecentEntry(path: string, entryType: RecentEntryType) {
    if (!desktopEnabled) return;

    if (entryType === 'folder') {
      if (!(await ensureExplorerPathExists(path, t.folderMissing()))) {
        return;
      }
      await openFolderWithBehavior(path);
      return;
    }

    if (!(await ensureExplorerPathExists(path, t.fileMissing()))) {
      return;
    }
    await openTargetWithBehavior({ kind: 'documents', paths: [path] });
  }

  async function clearRecentEntriesList() {
    if (!desktopEnabled) return;
    await clearRecentEntries().catch(() => undefined);
    await refreshRecentFiles();
  }

  async function removeRecentEntry(path: string) {
    if (!desktopEnabled) return;
    // 当前后端没有单条删除命令，通过清除全部 + 重新写入保留条目实现
    const current = recentFiles.filter((entry) => entry.path !== path);
    await clearRecentEntries().catch(() => undefined);
    for (const entry of current) {
      if (entry.entryType === 'file') {
        await rememberRecentEntry(entry.path, 'file', entry.title ?? null, entry.wordCount).catch(
          () => undefined,
        );
      } else {
        await rememberRecentEntry(entry.path, 'folder', null, 0).catch(() => undefined);
      }
    }
    await refreshRecentFiles();
  }

  async function closeCurrentFile() {
    if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
    const activeTab = tabs.find((t) => t.id === activeTabId);
    if (!activeTab) return;
    await closeTab(activeTab.id);
  }

  async function closeCurrentWindow() {
    if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
    const closeBehavior = await resolveCloseWindowBehaviorForCloseRequest();
    if (!closeBehavior) {
      return;
    }

    // 步骤：关闭窗口前检查未保存的脏标签，弹出确认对话框（三按钮模式，与关闭标签页 UI 一致）
    let discardDirtySegmented = false;
    if (closeBehavior === 'close-window') {
      const dirtyTabs = getDirtyTabs(tabs);
      if (dirtyTabs.length > 0) {
        const names = dirtyTabs.map((t) => t.fileName).join('、');
        const hasSaveable = dirtyTabs.some((t) => t.nativePath);
        const result = await confirmAction(t.unsavedChangesCloseWindow({ names }), {
          okLabel: t.discardChanges(),
          cancelLabel: t.cancel(),
          saveLabel: hasSaveable ? t.save() : undefined,
        });
        if (result === false) return;
        // 用户选择保存：保存当前活动文件后继续关闭
        if (result === 'save') {
          if (!(await saveMarkdownFile(false))) return;
        } else {
          discardDirtySegmented = true;
        }
      }
    }

    // 关闭窗口前先收口延迟恢复；否则在途 open 会在会话快照之后泄漏。
    if (closeBehavior === 'close-window') {
      invalidatePendingPreviewOpen();
      await cancelDeferredWorkspaceRestore();
      await closeAllSegmentedSessions(discardDirtySegmented);
    } else {
      await workspaceRestorePreparation;
      await deferredWorkspaceRestore;
      await flushAllSegmentedSessions();
    }
    await flushPersistWorkspaceState();
    await flushCurrentReadingPosition();
    const shouldHideToTray = closeBehavior === 'close-to-tray';
    await closeDesktopWindow(desktopEnabled, shouldHideToTray);
  }

  async function resolveCloseWindowBehaviorForCloseRequest(): Promise<CloseWindowAction | null> {
    if (!desktopEnabled) {
      return 'close-window';
    }
    if (closeWindowBehavior !== 'ask-every-time') {
      return closeWindowBehavior;
    }

    const choice = await requestCloseWindowChoice();
    if (!choice) {
      return null;
    }
    if (choice.remember) {
      await persistCloseWindowBehavior(choice.behavior);
    }
    return choice.behavior;
  }

  function requestCloseWindowChoice() {
    if (closeWindowChoicePromise) {
      return closeWindowChoicePromise;
    }

    rememberCloseWindowChoice = true;
    closeWindowChoiceDialogOpen = true;
    closeWindowChoicePromise = new Promise<CloseWindowChoiceResult>((resolve) => {
      closeWindowChoiceResolver = resolve;
    });
    return closeWindowChoicePromise;
  }

  async function persistCloseWindowBehavior(behavior: CloseWindowBehavior) {
    closeWindowBehavior = behavior;
    await updateAppSetting('closeWindowBehavior', behavior).catch(() => undefined);
  }

  function resolveCloseWindowChoice(behavior: CloseWindowAction) {
    closeWindowChoiceDialogOpen = false;
    const resolver = closeWindowChoiceResolver;
    closeWindowChoiceResolver = null;
    closeWindowChoicePromise = null;
    resolver?.({ behavior, remember: rememberCloseWindowChoice });
  }

  function cancelCloseWindowChoice() {
    closeWindowChoiceDialogOpen = false;
    const resolver = closeWindowChoiceResolver;
    closeWindowChoiceResolver = null;
    closeWindowChoicePromise = null;
    resolver?.(null);
  }

  function openExternalChangeDialog(
    change: ExternalFileChangeState,
    changeToken: string | null = null,
  ) {
    const targetTab = tabs.find((tab) => tab.id === activeTabId);
    if (!targetTab) return;
    externalChangeDialogTargetTabId = targetTab.id;
    externalChangeDialogTargetSessionId = isSegmentedTextTab(targetTab)
      ? targetTab.sessionId
      : null;
    externalChangeDialogToken = changeToken;
    externalChangeDialogState = change;
    externalChangeDialogOpen = true;
  }

  function closeExternalChangeDialog() {
    externalChangeDialogOpen = false;
    externalChangeDialogState = null;
    externalChangeDialogTargetTabId = null;
    externalChangeDialogTargetSessionId = null;
    externalChangeDialogToken = null;
  }

  function isExternalChangeDialogTargetActive(tabId: string, sessionId: string | null) {
    if (activeTabId !== tabId) return false;
    const targetTab = tabs.find((tab) => tab.id === tabId);
    if (!targetTab) return false;
    return (isSegmentedTextTab(targetTab) ? targetTab.sessionId : null) === sessionId;
  }

  function getValidExternalChangeDialogTarget() {
    const change = externalChangeDialogState;
    const tabId = externalChangeDialogTargetTabId;
    const sessionId = externalChangeDialogTargetSessionId;
    const changeToken = externalChangeDialogToken;
    if (!change || !tabId || !isExternalChangeDialogTargetActive(tabId, sessionId)) {
      closeExternalChangeDialog();
      return null;
    }
    return { change, tabId, sessionId, changeToken };
  }

  // 外部文件变更弹框 —— 重新载入外部版本
  async function handleExternalChangeReload() {
    const target = getValidExternalChangeDialogTarget();
    if (!target) return;
    closeExternalChangeDialog();
    try {
      await reloadExternalFile();
    } catch (error) {
      statusMessage = error instanceof Error ? error.message : t.openFileFailed();
      if (isExternalChangeDialogTargetActive(target.tabId, target.sessionId)) {
        openExternalChangeDialog(target.change, target.changeToken);
      }
    }
  }

  // 外部文件变更弹框 —— 覆盖外部版本
  function handleExternalChangeOverwrite() {
    const target = getValidExternalChangeDialogTarget();
    if (!target) return;
    closeExternalChangeDialog();
    overwriteExternalFile().catch(() => undefined);
  }

  // 文件已被删除时，该按钮本身就是最终确认，不再叠加未保存确认框。
  async function handleExternalChangeCloseDiscard() {
    const target = getValidExternalChangeDialogTarget();
    if (!target || target.change.type !== 'deleted') return;
    closeExternalChangeDialog();
    await closeTab(target.tabId, undefined, true);
  }

  // 外部版本冲突时另存为当前冻结 revision；取消文件对话框后恢复冲突选择。
  async function handleExternalChangeSaveAs() {
    const target = getValidExternalChangeDialogTarget();
    if (!target) return;
    closeExternalChangeDialog();
    if (await saveMarkdownFile(true)) {
      return;
    }
    if (isExternalChangeDialogTargetActive(target.tabId, target.sessionId)) {
      openExternalChangeDialog(target.change, target.changeToken);
    }
  }

  // 外部文件变更弹框 —— 忽略（保留当前内容，关闭弹框）
  function handleExternalChangeDismiss() {
    const target = getValidExternalChangeDialogTarget();
    if (!target) return;
    if (target.change.type === 'deleted') {
      keepDeletedExternalFileTemporarily(target);
      return;
    }
    if (target.sessionId && target.changeToken) {
      ignoreSegmentedExternalChange(target.sessionId, target.changeToken);
      return;
    }
    dismissExternalChange(target.change);
  }

  function keepDeletedExternalFileTemporarily(target: {
    tabId: string;
    sessionId: string | null;
    changeToken: string | null;
  }) {
    const targetTab = tabs.find((tab) => tab.id === target.tabId);
    if (!targetTab) {
      closeExternalChangeDialog();
      return;
    }

    if (isSegmentedTextTab(targetTab) && target.sessionId && target.changeToken) {
      ignoreSegmentedExternalChange(target.sessionId, target.changeToken, {
        statusMessage: t.externalDeletedKeptTemporary(),
      });
      return;
    }

    if (!isMarkdownTab(targetTab)) return;
    saveActiveTabState();
    const latestTab = tabs.find((tab) => tab.id === target.tabId);
    if (!isMarkdownTab(latestTab)) return;

    latestTab.nativePath = null;
    latestTab.filePath = '';
    latestTab.savedMarkdown = '';
    latestTab.dirty = true;
    latestTab.lastKnownModifiedAt = 0;
    latestTab.diskReadonly = false;
    latestTab.externalFileChange = createEmptyExternalFileChange();
    nativePath = null;
    filePath = '';
    savedMarkdown = '';
    dirty = true;
    lastKnownModifiedAt = 0;
    diskReadonly = false;
    externalFileChange = createEmptyExternalFileChange();
    editor.setDirty(true);
    tabs = [...tabs];
    closeExternalChangeDialog();
    persistWorkspaceState();
    updateWindowTitle();
    statusMessage = t.externalDeletedKeptTemporary();
  }

  function ignoreSegmentedExternalChange(
    sessionId: string,
    changeToken: string,
    options?: { statusMessage?: string },
  ) {
    ignoredSegmentedExternalChanges.set(sessionId, changeToken);
    // 保留 tab 上的冲突状态以继续暂停自动保存；token 只抑制同一磁盘身份的重复提示。
    closeExternalChangeDialog();
    if (options?.statusMessage) {
      statusMessage = options.statusMessage;
    }
  }

  function dismissExternalChange(
    change: ExternalFileChangeState | null,
    options?: { statusMessage?: string },
  ) {
    // 更新 lastKnownModifiedAt 为磁盘时间，避免下次轮询重复弹框
    if (change && change.modifiedAt > 0) {
      lastKnownModifiedAt = change.modifiedAt;
    }
    setExternalFileChangeState(createEmptyExternalFileChange());
    closeExternalChangeDialog();
    if (options?.statusMessage) {
      statusMessage = options.statusMessage;
    }
  }

  function getSegmentedExternalChangeToken(result: SegmentedExternalChangeResult) {
    if (result.type === 'none') return null;
    // 兼容旧后端；新后端使用完整文件身份生成 token，避免秒级 mtime 碰撞。
    return result.changeToken || `${result.type}:${result.modifiedAt}`;
  }

  function setExternalFileChangeState(value: ExternalFileChangeState) {
    const changed = externalFileChange.type !== value.type;
    externalFileChange = value;
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (activeTab && changed) {
      activeTab.externalFileChange = value;
      tabs = [...tabs];
      persistWorkspaceState();
    }
  }

  function tryHandleExternalFileChangeByPreference(
    change: ExternalFileChangeState,
    segmentedIgnoreTarget?: { sessionId: string; changeToken: string },
  ) {
    // 有本地补丁时任何自动 reload/overwrite 都可能丢数据，必须回到显式冲突选择。
    if (change.type !== 'modified' || change.dirtyAtDetection) {
      return false;
    }

    switch (externalFileChangeBehavior as ExternalFileChangeBehavior) {
      case 'reload-external':
        closeExternalChangeDialog();
        reloadExternalFile().catch((error) => {
          statusMessage = error instanceof Error ? error.message : t.openFileFailed();
        });
        return true;
      case 'ignore':
        if (segmentedIgnoreTarget) {
          ignoreSegmentedExternalChange(
            segmentedIgnoreTarget.sessionId,
            segmentedIgnoreTarget.changeToken,
            { statusMessage: t.externalChangeKeptCurrent() },
          );
        } else {
          dismissExternalChange(change, { statusMessage: t.externalChangeKeptCurrent() });
        }
        return true;
      case 'overwrite-external':
        closeExternalChangeDialog();
        overwriteExternalFile().catch(() => undefined);
        return true;
    }

    return false;
  }

  async function requestExitApp() {
    if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
    const dirtyTabs = getDirtyTabs(tabs);
    let discardDirtySegmented = false;
    if (dirtyTabs.length > 0) {
      const names = dirtyTabs.map((t) => t.fileName).join('、');
      const ok = await confirmAction(t.unsavedChangesExitApp({ names }));
      if (ok === false) return;
      discardDirtySegmented = true;
    }
    // 退出与关闭窗口共享相同的恢复收口边界，确保没有迟到 session 留在后端。
    invalidatePendingPreviewOpen();
    await cancelDeferredWorkspaceRestore();
    await closeAllSegmentedSessions(discardDirtySegmented);
    await flushPersistWorkspaceState();
    await flushCurrentReadingPosition();
    await exitDesktopApp(desktopEnabled);
  }

  async function approveSoftwareUpdateInstall(requestId: string) {
    const approved = await confirmSoftwareUpdateInstall();

    const { emit } = await import('@tauri-apps/api/event');
    await emit('nomo://update-install-decision', { requestId, approved });
  }

  async function confirmSoftwareUpdateInstall() {
    const dirtyTabs = getDirtyTabs(tabs);
    if (dirtyTabs.length === 0) {
      return true;
    }
    const names = dirtyTabs.map((tab) => tab.fileName).join('、');
    return (await confirmAction(t.unsavedChangesBeforeUpdate({ names }))) !== false;
  }

  function handleSoftwareUpdateSnapshot(state: SoftwareUpdateSnapshot) {
    const version = state.version ?? state.candidate?.version;
    if (
      !version ||
      !['available', 'downloaded'].includes(state.status) ||
      state.noticeWindowLabel !== windowLabel
    ) {
      return;
    }
    const signature = `${version}:${state.noticeWindowLabel}`;
    if (signature === softwareUpdateNoticeSignature) {
      return;
    }
    softwareUpdateNoticeSignature = signature;
    if (softwareUpdateDismissedVersion && softwareUpdateDismissedVersion !== version) {
      softwareUpdateDismissedVersion = '';
      void updateAppSetting('softwareUpdateDismissedVersion', '');
    }
    if (softwareUpdateDismissedVersion !== version) {
      softwareUpdateNoticeVisible = true;
    }
  }

  function openSoftwareUpdate() {
    if (!softwareUpdateSnapshot.candidate && !softwareUpdateSnapshot.downloadedUpdate) {
      return;
    }
    softwareUpdateNoticeVisible = false;
    softwareUpdateDialogOpen = true;
  }

  function hideSoftwareUpdateForLater() {
    softwareUpdateNoticeVisible = false;
    softwareUpdateDialogOpen = false;
  }

  function dismissSoftwareUpdateVersion() {
    const version =
      softwareUpdateSnapshot.version ?? softwareUpdateSnapshot.candidate?.version ?? '';
    softwareUpdateNoticeVisible = false;
    if (!version) return;
    softwareUpdateDismissedVersion = version;
    void updateAppSetting('softwareUpdateDismissedVersion', version);
  }

  async function downloadCurrentSoftwareUpdate() {
    const candidate = softwareUpdateSnapshot.candidate;
    if (!candidate || candidate.assetKind !== 'windowsInstaller') {
      return;
    }
    try {
      await startSoftwareUpdateDownload(candidate);
    } catch (error) {
      showVisibleError(error, t.softwareUpdateFailed());
    }
  }

  async function installCurrentSoftwareUpdate() {
    const downloaded = softwareUpdateSnapshot.downloadedUpdate;
    if (!downloaded || !(await confirmSoftwareUpdateInstall())) {
      return;
    }
    try {
      await startSoftwareUpdateInstall(downloaded);
    } catch (error) {
      showVisibleError(error, t.softwareUpdateInstallFailed());
    }
  }

  async function retrySoftwareUpdateCheck() {
    try {
      await runSoftwareUpdateCheck(false);
    } catch (error) {
      showVisibleError(error, t.softwareUpdateFailed());
    }
  }

  function scheduleStartupSoftwareUpdateCheck() {
    if (
      !desktopEnabled ||
      windowLabel !== 'main' ||
      !softwareUpdateAutoCheckEnabled ||
      softwareUpdateSnapshot.installationKind === 'unsupported' ||
      softwareUpdateStartupTimer !== null
    ) {
      return;
    }
    softwareUpdateStartupTimer = window.setTimeout(() => {
      softwareUpdateStartupTimer = null;
      if (
        !softwareUpdateAutoCheckEnabled ||
        softwareUpdateSnapshot.installationKind === 'unsupported'
      ) {
        return;
      }
      void runSoftwareUpdateCheck(true).catch(() => undefined);
    }, 2000);
  }

  function persistEditorModePreference(nextMode: EditorViewMode) {
    preferredEditorMode = nextMode;
    updateAppSetting('editorMode', nextMode).catch(() => undefined);
  }

  function notifyModePaneReady(nextMode: EditorViewMode, generation: number) {
    const editorGrid =
      sourcePane?.closest<HTMLElement>('.editor-grid') ??
      semanticPane?.closest<HTMLElement>('.editor-grid');
    editorGrid?.dispatchEvent(
      new CustomEvent('nomo:mode-pane-ready', {
        detail: { mode: nextMode, generation },
      }),
    );
  }

  async function changeEditorMode(nextMode: EditorViewMode, persistPreference: boolean) {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      return false;
    }
    if (largeDocumentMode && nextMode !== 'source') {
      statusMessage = t.largeDocumentStayReadonlySource();
      return false;
    }
    cancelPendingReadingPositionRestore();
    selectedStats = null;
    const previousViewMode = mode;
    const previousActiveMode = getActiveEditorMode();
    const anchor = getCurrentReadingAnchor(previousActiveMode);
    saveCurrentReadingPositionToMemoryOnly(previousActiveMode, anchor);

    if (previousViewMode === 'split' && previousActiveMode === 'source') {
      refreshSplitSemanticView();
    } else {
      editor.getMarkdown();
    }
    if (nextMode === 'split') {
      splitActivePane = previousActiveMode;
    }
    clearSplitSemanticRefreshTimer();
    splitSemanticRefreshGeneration += 1;
    mode = nextMode;
    try {
      if (nextMode === 'split' && splitActivePane === 'source') {
        editor.refreshSemanticView();
      }
      const targetCoreMode = nextMode === 'split' ? splitActivePane : getCoreModeForView(nextMode);
      const modeSwitchResult = await editorInteraction.setMode(
        targetCoreMode,
        anchor,
        true,
        nextMode,
      );
      if (modeSwitchResult.status === 'superseded' || mode !== nextMode) {
        return false;
      }

      notifyModePaneReady(nextMode, modeSwitchResult.generation);
    } catch (error) {
      await tick();
      if (mode === nextMode) {
        notifyModePaneReady(nextMode, -1);
      }
      throw error;
    }
    if (persistPreference) {
      persistEditorModePreference(nextMode);
    }
    return true;
  }

  function setMode(nextMode: EditorViewMode) {
    if (markdownMiniActive) return;
    void changeEditorMode(nextMode, true).catch(() => undefined);
  }

  function setSidebarHidden(hidden: boolean) {
    focusMode = hidden;
    updateAppSetting('sidebarHidden', hidden).catch(() => undefined);
  }

  function closeToolbarTransientPanels() {
    tablePickerOpen = false;
    linkPickerOpen = false;
  }

  function toggleFocusMode() {
    setSidebarHidden(!focusMode);
  }

  function setToolbarHidden(hidden: boolean) {
    if (hidden) {
      closeToolbarTransientPanels();
    }
    toolbarHidden = hidden;
    updateAppSetting('toolbarHidden', hidden).catch(() => undefined);
  }

  function toggleToolbar() {
    setToolbarHidden(!toolbarHidden);
  }

  function toggleSplitAlignmentGuide() {
    splitAlignmentGuideVisible = !splitAlignmentGuideVisible;
  }

  function setOutlineVisiblePreference(visible: boolean) {
    outlineVisible = visible;
    updateAppSetting('outlineVisible', visible).catch(() => undefined);
  }

  function toggleOutlineVisible() {
    setOutlineVisiblePreference(!outlineVisible);
  }

  function setWritingStatsVisiblePreference(visible: boolean) {
    writingStatsVisible = visible;
    updateAppSetting('writingStatsVisible', visible).catch(() => undefined);
  }

  function setWritingStatsMetric(metric: WritingStatsMetric) {
    writingStatsMetric = metric;
    updateAppSetting('writingStatsMetric', metric).catch(() => undefined);
  }

  const commandHandlers: AppCommandHandlers = {
    createNewFile: () => createNewFile(),
    createNewWindow,
    openFileDialog: () => openFileDialog(),
    openFolderDialog: () => openFolderDialog(),
    openRecentEntry: (path, entryType) => openRecentEntry(path, entryType),
    saveMarkdownFile: (saveAs) => saveMarkdownFile(saveAs),
    closeCurrentFile: () => closeCurrentFile(),
    closeCurrentWindow: () => closeCurrentWindow(),
    runCommand: (command) => runCommand(command),
    openTablePicker: () => openTablePicker(),
    openLinkPicker: () => openLinkPicker(),
    openSearchPanel: (replaceVisible) => openSearchPanel(replaceVisible),
    closeSearchPanel: () => closeSearchPanel(),
    getSearchState: () =>
      isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))
        ? (segmentedWorkspace?.getSearchState() ?? { open: false, replaceVisible: false })
        : { open: searchPanelOpen, replaceVisible: searchReplaceVisible },
    openSettings: () => openSettings(),
    editFrontMatter: () => editFrontMatter(),
    showUnavailableFeature: (featureName) => showUnavailableFeature(featureName),
    setMode: (nextMode) => setMode(nextMode),
    getMode: () => mode,
    toggleTheme: () => toggleTheme(),
    toggleFocusMode: () => toggleFocusMode(),
    toggleToolbar: () => toggleToolbar(),
    toggleMarkdownMini: () => {
      void toggleMarkdownMini();
    },
    toggleOutlineVisible: () => toggleOutlineVisible(),
    getDefaultCodeBlockLanguage: () => defaultCodeBlockLanguage,
    getDefaultDiagramType: () => defaultDiagramType,
    switchToNextTab: () => {
      const idx = tabs.findIndex((t) => t.id === activeTabId);
      const nextIdx = idx >= 0 ? (idx + 1) % tabs.length : 0;
      if (tabs[nextIdx]) switchTab(tabs[nextIdx].id);
    },
    switchToPrevTab: () => {
      const idx = tabs.findIndex((t) => t.id === activeTabId);
      const prevIdx = idx >= 0 ? (idx - 1 + tabs.length) % tabs.length : tabs.length - 1;
      if (tabs[prevIdx]) switchTab(tabs[prevIdx].id);
    },
    exportHtml: () => handleExport('html'),
    exportPdf: () => handleExport('pdf'),
  };

  async function updateWindowTitle() {
    await updateAppWindowTitle(desktopEnabled, fileName, dirty);
  }

  $: {
    if (desktopEnabled && (fileName || dirty !== undefined)) {
      updateWindowTitle();
    }
  }

  // 步骤：recentFiles 变化时异步检测路径是否存在，用于灰显失效条目
  $: if (desktopEnabled && recentFiles.length > 0) {
    void (async () => {
      const paths = recentFiles.map((entry) => entry.path);
      const exists = await checkPathsExist(paths).catch(() => paths.map(() => true));
      const nextMissing = new Set<string>();
      recentFiles.forEach((entry, index) => {
        if (!exists[index]) {
          nextMissing.add(entry.path);
        }
      });
      missingRecentPaths = nextMissing;
    })();
  } else {
    missingRecentPaths = new Set<string>();
  }
  let fileCheckTimer: number | null = null;
  let stopSystemThemeSync: () => void = () => undefined;
  let systemThemeListenerReady = false;
  let appearanceRuntimeActive = false;
  let appearanceApplyRequestId = 0;

  async function ensureExplorerPathExists(path: string, missingMessage: string) {
    if (!desktopEnabled) {
      return true;
    }

    const [exists] = await checkPathsExist([path]).catch(() => [true]);
    if (exists) {
      return true;
    }

    removeMissingExplorerPaths([path], false);
    statusMessage = `${missingMessage}：${path}`;
    await refreshRecentFiles();
    return false;
  }

  function isMissingPathError(error: string) {
    const message = error.toLowerCase();
    return (
      message.includes('文件不存在') ||
      message.includes('not found') ||
      message.includes('os error 2')
    );
  }

  function handleContextMenuOpen(event: ContextMenuOpenEvent) {
    openApplicationContextMenu({
      x: event.x,
      y: event.y,
      items: event.items.length ? event.items : buildEditorContextMenuItems(event.target),
    });
  }

  function openApplicationContextMenu(request: ContextMenuRequest) {
    activeMenu = null;
    tablePickerOpen = false;
    linkPickerOpen = false;
    contextMenuX = request.x;
    contextMenuY = request.y;
    contextMenuItems = request.items;
    contextMenuOpen = request.items.length > 0;
    contextMenuVersion += 1;
  }

  function buildEditorContextMenuItems(target: ContextMenuTarget): ContextMenuItem[] {
    if (target.kind === 'link') return buildLinkContextMenuItems(target);
    if (target.kind === 'heading') return buildHeadingContextMenuItems(target);
    if (target.kind === 'code-block') return buildCodeBlockContextMenuItems(target);
    if (target.kind === 'table') return buildTableContextMenuItems();
    if (target.kind === 'math-block' || target.kind === 'mermaid-block') {
      return buildRenderedBlockContextMenuItems(target);
    }
    return target.kind === 'selection'
      ? buildSelectionContextMenuItems()
      : buildTextContextMenuItems();
  }

  function buildSelectionContextMenuItems(): ContextMenuItem[] {
    const disabled = readonlyDocumentMode;
    return [
      { label: t.cut(), icon: 'cut', disabled, shortcut: 'Ctrl+X', action: cutSelection },
      { label: t.copy(), icon: 'copy', shortcut: 'Ctrl+C', action: copySelection },
      {
        label: t.paste(),
        icon: 'paste',
        disabled,
        shortcut: 'Ctrl+V',
        action: pasteFromContextMenu,
      },
      {
        label: t.pasteAsPlainText(),
        icon: 'paste',
        disabled,
        shortcut: 'Ctrl+Shift+V',
        action: () => pasteFromContextMenu('plain'),
      },
      menuSeparator(),
      {
        label: t.format(),
        icon: 'format',
        disabled,
        children: [
          {
            ...commandMenuItem(t.bold(), { type: 'toggleBold' }, 'format', 'Ctrl+B'),
            active: pendingInlineMarks.strong,
          },
          {
            ...commandMenuItem(t.italic(), { type: 'toggleItalic' }, 'format', 'Ctrl+I'),
            active: pendingInlineMarks.em,
          },
          {
            ...commandMenuItem(t.underline(), { type: 'toggleUnderline' }, 'format', 'Ctrl+U'),
            active: pendingInlineMarks.underline,
          },
          {
            ...commandMenuItem(t.strikethrough(), { type: 'toggleStrikethrough' }, 'format'),
            active: pendingInlineMarks.strikethrough,
          },
          {
            ...commandMenuItem(t.inlineCode(), { type: 'toggleCode' }, 'code', 'Ctrl+`'),
            active: pendingInlineMarks.code,
          },
          {
            ...commandMenuItem(t.highlight(), { type: 'toggleHighlight' }, 'format'),
            active: pendingInlineMarks.highlight,
          },
          menuSeparator(),
          commandMenuItem(t.clearStyle(), { type: 'clearInlineStyles' }, 'format'),
        ],
      },
      {
        label: editor.getActiveLink()?.active ? t.editLink() : t.createLink(),
        icon: 'link',
        disabled,
        shortcut: 'Ctrl+K',
        action: openLinkPicker,
      },
      commandMenuItem(t.insertInlineComment(), { type: 'insertCommentInline' }, 'edit'),
      menuSeparator(),
      {
        label: t.selectAll(),
        icon: 'select-all',
        shortcut: 'Ctrl+A',
        action: () => editor.selectAll(),
      },
      { label: t.find(), icon: 'search', shortcut: 'Ctrl+F', action: () => openSearchPanel(false) },
    ];
  }

  function buildTextContextMenuItems(): ContextMenuItem[] {
    const disabled = readonlyDocumentMode;
    return [
      commandMenuItem(t.undo(), { type: 'undo' }, 'undo', 'Ctrl+Z'),
      commandMenuItem(
        t.redo(),
        { type: 'redo' },
        'redo',
        getPlatformCapabilities().isMac ? 'Ctrl+Shift+Z' : 'Ctrl+Y',
      ),
      {
        label: t.paste(),
        icon: 'paste',
        disabled,
        shortcut: 'Ctrl+V',
        action: pasteFromContextMenu,
      },
      {
        label: t.pasteAsPlainText(),
        icon: 'paste',
        disabled,
        shortcut: 'Ctrl+Shift+V',
        action: () => pasteFromContextMenu('plain'),
      },
      menuSeparator(),
      { label: t.insert(), icon: 'insert', disabled, children: buildInsertContextMenuItems() },
      menuSeparator(),
      {
        label: t.selectAll(),
        icon: 'select-all',
        shortcut: 'Ctrl+A',
        action: () => editor.selectAll(),
      },
      { label: t.find(), icon: 'search', shortcut: 'Ctrl+F', action: () => openSearchPanel(false) },
    ];
  }

  function buildInsertContextMenuItems(): ContextMenuItem[] {
    const headings: ContextMenuItem[] = ([1, 2, 3, 4, 5, 6] as const).map((level) =>
      commandMenuItem(
        t.headingLevel({ level }),
        { type: 'setHeading', level },
        'heading',
        `Ctrl+${level}`,
      ),
    );
    return [
      ...headings,
      menuSeparator(),
      commandMenuItem(t.unorderedList(), { type: 'toggleBulletList' }, 'list'),
      commandMenuItem(t.orderedList(), { type: 'toggleOrderedList' }, 'list'),
      commandMenuItem(t.taskList(), { type: 'toggleTaskList' }, 'list'),
      commandMenuItem(t.quote(), { type: 'toggleBlockquote' }, 'quote'),
      commandMenuItem(t.callout(), { type: 'insertCallout' }, 'quote'),
      menuSeparator(),
      commandMenuItem(t.codeBlock(), { type: 'insertCodeBlock' }, 'code'),
      commandMenuItem(t.table(), { type: 'insertTable', rows: 3, columns: 3 }, 'table'),
      {
        label: t.image(),
        icon: 'image',
        disabled: readonlyDocumentMode,
        action: chooseImageForContextMenu,
      },
      commandMenuItem(t.mathFormula(), { type: 'insertMathBlock', tex: 'E = mc^2' }, 'formula'),
      commandMenuItem(t.mermaidDiagram(), { type: 'insertMermaidBlock' }, 'diagram'),
      commandMenuItem(t.horizontalRule(), { type: 'insertHorizontalRule' }, 'separator'),
    ];
  }

  function buildLinkContextMenuItems(target: ContextMenuTarget): ContextMenuItem[] {
    return [
      {
        label: t.openLink(),
        icon: 'open',
        action: () => target.href && openLinkFromEditor(target.href),
      },
      { label: t.editLink(), icon: 'edit', disabled: readonlyDocumentMode, action: openLinkPicker },
      { label: t.copyLinkAddress(), icon: 'copy', action: () => copyPlainText(target.href ?? '') },
      menuSeparator(),
      commandMenuItem(t.removeLink(), { type: 'removeLink' }, 'unlink'),
    ];
  }

  function buildHeadingContextMenuItems(target: ContextMenuTarget): ContextMenuItem[] {
    const levels = ([1, 2, 3, 4, 5, 6] as const).map((level) => ({
      ...commandMenuItem(t.headingLevel({ level }), { type: 'setHeading', level }, 'heading'),
      active: target.headingLevel === level,
    }));
    return [
      { label: t.heading(), icon: 'heading', children: levels },
      commandMenuItem(t.paragraph(), { type: 'setParagraph' }, 'format'),
      menuSeparator(),
      commandMenuItem(t.liftHeading(), { type: 'increaseHeadingLevel' }, 'heading'),
      commandMenuItem(t.sinkHeading(), { type: 'decreaseHeadingLevel' }, 'heading'),
    ];
  }

  function buildCodeBlockContextMenuItems(target: ContextMenuTarget): ContextMenuItem[] {
    return [
      {
        label: t.editCode(),
        icon: 'edit',
        disabled: readonlyDocumentMode,
        action: () => editor.editContextTarget(target),
      },
      { label: t.copyCode(), icon: 'copy', action: () => copyPlainText(target.text ?? '') },
      {
        label: t.selectLanguage(),
        icon: 'code',
        disabled: readonlyDocumentMode,
        action: () => editor.chooseContextTargetLanguage(target),
      },
      {
        label: t.convertToParagraph(),
        icon: 'format',
        disabled: readonlyDocumentMode,
        action: () => {
          if (editor.selectContextTarget(target)) runCommand({ type: 'insertCodeBlock' });
        },
      },
      menuSeparator(),
      {
        label: t.deleteAction(),
        icon: 'delete',
        danger: true,
        disabled: readonlyDocumentMode,
        action: () => editor.deleteContextTarget(target),
      },
    ];
  }

  function buildTableContextMenuItems(): ContextMenuItem[] {
    return [
      commandMenuItem(t.addRowBefore(), { type: 'addTableRowBefore' }, 'table'),
      commandMenuItem(t.addRowAfter(), { type: 'addTableRowAfter' }, 'table'),
      commandMenuItem(t.addColumnBefore(), { type: 'addTableColumnBefore' }, 'table'),
      commandMenuItem(t.addColumnAfter(), { type: 'addTableColumnAfter' }, 'table'),
      menuSeparator(),
      commandMenuItem(
        t.alignLeft(),
        { type: 'setTableColumnAlignment', align: 'left' },
        'align-left',
      ),
      commandMenuItem(
        t.alignCenter(),
        { type: 'setTableColumnAlignment', align: 'center' },
        'align-center',
      ),
      commandMenuItem(
        t.alignRight(),
        { type: 'setTableColumnAlignment', align: 'right' },
        'align-right',
      ),
      commandMenuItem(t.toggleTableHeader(), { type: 'toggleTableHeader' }, 'table'),
      menuSeparator(),
      commandMenuItem(t.deleteRow(), { type: 'deleteTableRow' }, 'delete'),
      commandMenuItem(t.deleteColumn(), { type: 'deleteTableColumn' }, 'delete'),
      { ...commandMenuItem(t.deleteTable(), { type: 'deleteTable' }, 'delete'), danger: true },
    ];
  }

  function buildRenderedBlockContextMenuItems(target: ContextMenuTarget): ContextMenuItem[] {
    const isMermaid = target.kind === 'mermaid-block';
    return [
      {
        label: isMermaid ? t.editDiagramSource() : t.editFormulaSource(),
        icon: 'edit',
        disabled: readonlyDocumentMode,
        action: () => editor.editContextTarget(target),
      },
      {
        label: isMermaid ? t.copyDiagramSource() : t.copyFormulaSource(),
        icon: 'copy',
        action: () => copyPlainText(target.text ?? ''),
      },
      menuSeparator(),
      {
        label: t.deleteAction(),
        icon: 'delete',
        danger: true,
        disabled: readonlyDocumentMode,
        action: () => editor.deleteContextTarget(target),
      },
    ];
  }

  function commandMenuItem(
    label: string,
    command: EditorCommand,
    icon: ContextMenuItem['icon'],
    shortcut?: string,
  ): ContextMenuItem {
    return {
      label,
      icon,
      shortcut,
      disabled: readonlyDocumentMode || !editor.canExecute(command),
      action: () => runCommand(command),
    };
  }

  function menuSeparator(): ContextMenuItem {
    return { label: '', separator: true };
  }

  async function copySelection() {
    const payload = editor.getClipboardPayload();
    if (!payload) return;
    try {
      await writeEditorClipboard(payload, desktopEnabled);
    } catch {
      statusMessage = t.copyFailed();
    }
  }

  async function cutSelection() {
    const payload = editor.getClipboardPayload();
    if (!payload || readonlyDocumentMode) return;
    try {
      await writeEditorClipboard(payload, desktopEnabled);
      editor.deleteSelection();
    } catch {
      statusMessage = t.cutFailed();
    }
  }

  async function copyPlainText(text: string) {
    try {
      await writeEditorClipboard({ text, html: '' }, desktopEnabled);
    } catch {
      statusMessage = t.copyFailed();
    }
  }

  async function revealContextPath(path: string) {
    try {
      await revealInExplorer(path);
    } catch (error) {
      showVisibleError(error, t.openFolderFailed());
    }
  }

  async function pasteFromContextMenu(mode: EditorPasteMode = 'auto') {
    if (readonlyDocumentMode) return;
    try {
      const content = await readEditorClipboard(desktopEnabled, mode === 'plain' ? 'text' : 'rich');
      if (content.kind === 'image') {
        await imageInsertion.insertImageFiles(content.files);
      } else {
        const input =
          content.kind === 'html'
            ? { html: content.html, text: content.text }
            : { text: content.text };
        const result = editor.pasteClipboard(input, { mode });
        if (result.status === 'rejected' && result.reason === 'no-text') {
          statusMessage = t.clipboardHasNoText();
        }
      }
      editor.focus();
    } catch {
      statusMessage = t.pasteFailed();
    }
  }

  function chooseImageForContextMenu() {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.multiple = true;
    input.addEventListener('change', () => {
      const files = Array.from(input.files ?? []);
      if (files.length) void imageInsertion.insertImageFiles(files);
    });
    input.click();
  }

  function handleWorkspaceContextMenu(event: MouseEvent) {
    if (getActiveEditorMode() !== 'semantic') return;
    event.preventDefault();
    openApplicationContextMenu({
      x: event.clientX,
      y: event.clientY,
      items: [
        {
          label: outlineVisible ? t.hideOutline() : t.showOutline(),
          icon: 'outline',
          active: outlineVisible,
          action: toggleOutlineVisible,
        },
        {
          label: toolbarHidden ? t.showToolbar() : t.hideToolbar(),
          icon: 'toolbar',
          active: !toolbarHidden,
          action: toggleToolbar,
        },
        {
          label: focusMode ? t.exitFocusMode() : t.enterFocusMode(),
          icon: 'focus',
          active: focusMode,
          action: toggleFocusMode,
        },
        menuSeparator(),
        {
          label: mode === 'split' ? t.splitAdaptiveWidth() : t.contentWidth(),
          icon: 'width',
          disabled: mode === 'split',
          children: [45, 60, 75, 90].map((value) => ({
            label: `${value}%`,
            active: contentWidthPercent === value,
            action: () => editorSettings.updateContentWidthValue(value),
          })),
        },
        {
          label: t.resetZoom(),
          icon: 'zoom',
          disabled: zoomPercent === 100,
          action: () => handleZoomChange(100),
        },
      ],
    });
  }

  /**
   * 监听 ImageNodeView 通过自定义 DOM 事件传递的右键菜单。
   * 由于 NodeView.stopEvent 拦截了原生 contextmenu，
   * 菜单数据通过 image-context-menu 自定义事件冒泡到此。
   */
  function handleImageContextMenu(event: Event) {
    const customEvent = event as CustomEvent;
    const detail = customEvent.detail;
    if (!detail?.items) return;
    openApplicationContextMenu({ x: detail.x, y: detail.y, items: detail.items });
  }

  function closeContextMenu() {
    contextMenuOpen = false;
    contextMenuItems = [];
  }

  const editor = createEditorCore({
    markdown,
    mode: getCoreModeForView(mode),
    inlineCodeRenderingEnabled,
    copyMarkdownSyntaxEnabled,
    theme: initialResolvedTheme.editorTheme,
    onChange: syncFromEditor,
    onSelectionChange: handleSemanticSelectionChange,
    onLinkShortcut: () => openLinkPicker(),
    onOpenLink: (href) => openLinkFromEditor(href),
    getImageContext: () => getImageContext(),
    onImagesDeleted: (event) => handleDeletedImageResources(event),
    onContextMenuOpen: handleContextMenuOpen,
  });

  function handleSemanticSelectionChange(event: EditorSelectionEvent) {
    if (getActiveEditorMode() !== 'semantic') return;
    selectedStats = event.selection ? calculateDocumentStats(event.selectedMarkdown) : null;
  }

  function handleSourceSelectionChange(selectedMarkdown: string) {
    if (getActiveEditorMode() !== 'source') return;
    selectedStats = selectedMarkdown ? calculateDocumentStats(selectedMarkdown) : null;
  }

  function openSearchPanel(replaceVisible = false) {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      segmentedWorkspace?.openSearch(replaceVisible);
      return;
    }
    if (!hasOpenDocument()) return;
    searchPanelOpen = true;
    searchReplaceVisible = replaceVisible;
    linkPickerOpen = false;
    tablePickerOpen = false;
    refreshSearchMatches({ preserveActive: true, selectActive: false });
  }

  function closeSearchPanel() {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      segmentedWorkspace?.closeSearch();
      return;
    }
    const activeMatch = searchMatches[searchActiveIndex];
    searchPanelOpen = false;
    clearSearchDebounceTimer();
    if (getActiveEditorMode() === 'source') {
      editor.setSearchHighlights([], 0);
      if (
        sourceEditor &&
        activeMatch &&
        sourceEditor.getSelection().from === activeMatch.from &&
        sourceEditor.getSelection().to === activeMatch.to
      ) {
        sourceEditor.setSelection(activeMatch.to);
      }
      sourceEditor?.focus();
    } else {
      if (editor.clearSearchState) {
        editor.clearSearchState(activeMatch);
      } else {
        editor.setSearchHighlights([], 0);
      }
      editor.focus();
    }
  }

  function updateSearchQuery(event: Event) {
    searchQuery = (event.currentTarget as HTMLInputElement).value;
    searchActiveIndex = 0;
    if (!searchQuery) {
      clearSearchDebounceTimer();
      refreshSearchMatches({ preserveActive: false, selectActive: false });
      return;
    }
    scheduleSearchRefresh({ preserveActive: false, selectActive: false });
  }

  function updateSearchReplacement(event: Event) {
    searchReplacement = (event.currentTarget as HTMLInputElement).value;
  }

  function toggleSearchCaseSensitive() {
    searchCaseSensitive = !searchCaseSensitive;
    searchActiveIndex = 0;
    clearSearchDebounceTimer();
    refreshSearchMatches({ preserveActive: false, selectActive: false });
  }

  function toggleSearchWholeWord() {
    searchWholeWord = !searchWholeWord;
    searchActiveIndex = 0;
    clearSearchDebounceTimer();
    refreshSearchMatches({ preserveActive: false, selectActive: false });
  }

  function toggleSearchBackwards() {
    searchBackwards = !searchBackwards;
  }

  function toggleSearchWrapAround() {
    searchWrapAround = !searchWrapAround;
  }

  function toggleSearchReplaceVisible() {
    searchReplaceVisible = !searchReplaceVisible;
  }

  function findPreviousSearchMatch() {
    moveSearchMatch(-1);
  }

  function findNextSearchMatch() {
    moveSearchMatch(1);
  }

  function moveSearchMatch(direction: -1 | 1) {
    if (searchMatches.length === 0) return;
    const nextIndex = searchActiveIndex + direction;
    if ((nextIndex < 0 || nextIndex >= searchMatches.length) && !searchWrapAround) return;
    searchActiveIndex = (nextIndex + searchMatches.length) % searchMatches.length;
    selectActiveSearchMatch();
  }

  function countSearchMatches() {
    refreshSearchMatches({ preserveActive: true, selectActive: false });
  }

  function replaceCurrentSearchMatch() {
    if (readonlyDocumentMode || searchMatches.length === 0) return;
    const match = searchMatches[searchActiveIndex];
    if (!match) return;

    if (getActiveEditorMode() === 'source') {
      const nextMarkdown = replaceTextRange(markdown, match, searchReplacement);
      sourceEditor?.setMarkdown(nextMarkdown, { addToHistory: true });
    } else {
      editor.replaceSearchMatch(match, searchReplacement);
    }

    tick().then(() => {
      refreshSearchMatches({ preserveActive: true, selectActive: true });
      statusMessage = t.replacedOneMatch();
    });
  }

  function replaceAllSearchMatches() {
    if (readonlyDocumentMode || !searchQuery) return;
    let replaced = 0;

    if (getActiveEditorMode() === 'source') {
      const result = replaceAllTextMatches(markdown, searchQuery, searchReplacement, {
        caseSensitive: searchCaseSensitive,
        wholeWord: searchWholeWord,
      });
      replaced = result.count;
      if (replaced > 0) {
        sourceEditor?.setMarkdown(result.text, { addToHistory: true });
      }
    } else {
      replaced = editor.replaceAllSearchMatches(searchQuery, searchReplacement, {
        caseSensitive: searchCaseSensitive,
        wholeWord: searchWholeWord,
      });
    }

    tick().then(() => {
      searchActiveIndex = 0;
      refreshSearchMatches({ preserveActive: false, selectActive: true });
      statusMessage = t.replacedMatchCount({ count: replaced });
    });
  }

  function refreshSearchMatches(options?: { preserveActive?: boolean; selectActive?: boolean }) {
    clearSearchDebounceTimer();
    if (!searchPanelOpen) {
      searchMatches = [];
      searchMatchCount = 0;
      lastSearchSignature = '';
      return;
    }

    const previousMatch = searchMatches[searchActiveIndex];
    searchMatches =
      getActiveEditorMode() === 'source'
        ? findTextMatches(markdown, searchQuery, {
            caseSensitive: searchCaseSensitive,
            wholeWord: searchWholeWord,
          })
        : editor.findSearchMatches(searchQuery, {
            caseSensitive: searchCaseSensitive,
            wholeWord: searchWholeWord,
          });
    searchMatchCount = searchMatches.length;

    if (searchMatches.length === 0) {
      searchActiveIndex = 0;
      return;
    }

    if (options?.preserveActive && previousMatch) {
      const nextIndex = searchMatches.findIndex(
        (match) => match.from >= previousMatch.from && match.text === previousMatch.text,
      );
      searchActiveIndex =
        nextIndex >= 0 ? nextIndex : Math.min(searchActiveIndex, searchMatches.length - 1);
    } else {
      searchActiveIndex = Math.min(searchActiveIndex, searchMatches.length - 1);
    }

    // 更新编辑器搜索高亮 decorations（不依赖 focus 即可显示）
    if (getActiveEditorMode() !== 'source') {
      editor.setSearchHighlights(searchMatches, searchActiveIndex);
    } else {
      editor.setSearchHighlights([], 0);
    }

    if (options?.selectActive) {
      selectActiveSearchMatch();
    }
  }

  function scheduleSearchRefresh(options?: { preserveActive?: boolean; selectActive?: boolean }) {
    clearSearchDebounceTimer();
    searchDebounceTimer = window.setTimeout(() => {
      searchDebounceTimer = null;
      refreshSearchMatches(options);
    }, SEARCH_DEBOUNCE_MS);
  }

  function clearSearchDebounceTimer() {
    if (searchDebounceTimer !== null) {
      window.clearTimeout(searchDebounceTimer);
      searchDebounceTimer = null;
    }
  }

  async function selectActiveSearchMatch() {
    const match = searchMatches[searchActiveIndex];
    if (!match) return;
    cancelPendingReadingPositionRestore();

    const isSearchFocused = document.activeElement?.closest('.search-replace-panel') !== null;
    const activeSearchInput =
      document.activeElement instanceof HTMLInputElement &&
      document.activeElement.closest('.search-replace-panel')
        ? document.activeElement
        : null;
    const searchInput =
      activeSearchInput ?? document.querySelector<HTMLInputElement>('.search-replace-panel input');
    const searchCursorStart = activeSearchInput?.selectionStart ?? null;
    const searchCursorEnd = activeSearchInput?.selectionEnd ?? null;

    // 总是 focus 编辑器，让 scrollIntoView 和 selection 高亮生效
    if (getActiveEditorMode() === 'source') {
      await selectSourceSearchMatch(match, true);
    } else {
      editor.selectSearchMatch(match, true);
      await tick();
      await waitForAnimationFrame();
    }

    // 如果搜索面板之前有焦点，focus 回去并保持光标位置；preventScroll 避免覆盖编辑区跳转。
    if (isSearchFocused && searchInput) {
      searchInput.focus({ preventScroll: true });
      if (searchCursorStart !== null && searchCursorEnd !== null) {
        searchInput.setSelectionRange(searchCursorStart, searchCursorEnd);
      }
    }
  }

  async function selectSourceSearchMatch(match: EditorSearchMatch, focusEditor = true) {
    await tick();
    if (!sourceEditor) return;
    if (focusEditor) {
      sourceEditor.focus();
    }
    sourceEditor.revealRange(match.from, match.to);
    await waitForAnimationFrame();
  }

  function waitForAnimationFrame() {
    return new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
  }

  function hasOpenDocument() {
    return tabs.length > 0 && Boolean(activeTabId);
  }

  function detachMountedEditorHostEvents() {
    if (!mountedEditorHost) return;
    mountedEditorHost.removeEventListener('image-context-menu', handleImageContextMenu);
    mountedEditorHost = null;
  }

  function mountEditorHostIfReady() {
    if (!hasOpenDocument() || !editorHost || mountedEditorHost === editorHost) {
      return;
    }

    detachMountedEditorHostEvents();
    editor.mount(editorHost);
    editorHost.addEventListener('image-context-menu', handleImageContextMenu);
    mountedEditorHost = editorHost;
  }

  $: if (tabs.length > 0 && activeTabId && editorHost) mountEditorHostIfReady();
  $: if ((tabs.length === 0 || !activeTabId) && mountedEditorHost) detachMountedEditorHostEvents();

  const editorSettings = createEditorSettingsController({
    getDesktopEnabled: () => desktopEnabled,
    getFontSize: () => fontSize,
    setFontSize: (value) => {
      fontSize = value;
    },
    getLineHeight: () => lineHeight,
    setLineHeight: (value) => {
      lineHeight = value;
    },
    getContentWidthPercent: () => contentWidthPercent,
    setContentWidthPercent: (value) => {
      contentWidthPercent = value;
    },
    refreshEditorViewportLayout: () => refreshEditorViewportLayout(),
  });
  function openSettings() {
    openSettingsWindow(desktopEnabled);
  }

  function openPreviewFile(path: string) {
    return enqueueOpenTargetOperation(() => routePreviewFile(path));
  }

  async function routePreviewFile(path: string) {
    if (!desktopEnabled) return false;
    await syncCurrentWindowOpenTargetsNow();
    const decision = await prepareOpenTargetWindow(
      desktopEnabled,
      { kind: 'documents', paths: [path] },
      false,
      { reuseDirectoryWindow: false }, // 文件树单击保留当前窗口预览，仅已打开文件跨窗口定位。
    ).catch((error) => {
      showVisibleError(error, t.previewOpenFailed());
      return null;
    });
    if (!decision || decision.action === 'handled') return false;
    if (decision.action === 'activate-current') {
      if (decision.target.kind !== 'documents' || decision.target.paths.length === 0) return false;
      await openPreviewFileInCurrentWindow(decision.target.paths[0]);
      return true;
    }
    if (decision.target.kind !== 'documents' || decision.target.paths.length === 0) return false;
    await openPreviewFileInCurrentWindow(decision.target.paths[0]);
    return true;
  }

  // 打开预览标签页（文件树单击）
  async function openPreviewFileInCurrentWindow(path: string) {
    if (!desktopEnabled) return;
    const requestGeneration = invalidatePendingPreviewOpen();
    if (!(await ensureExplorerPathExists(path, t.fileMissing()))) {
      return;
    }
    if (requestGeneration !== previewOpenGeneration) return;

    // 已有固定标签页打开此文件 → 切换到它
    const existingFixedTab = tabs.find(
      (t) => t.nativePath && sameNativePath(t.nativePath, path) && t.id !== previewTabId,
    );
    if (existingFixedTab) {
      await switchTab(existingFixedTab.id);
      return;
    }

    // 打开新预览会卸载当前工作区，必须在任何磁盘读取或 session 替换前完成增量刷新。
    try {
      await flushSegmentedDocumentBeforeTransition(
        tabs.find((tab) => tab.id === activeTabId),
        segmentedWorkspace,
        segmentedDocumentPort,
      );
      if (requestGeneration !== previewOpenGeneration) return;
    } catch (error) {
      showVisibleError(error, t.saveFileFailed());
      return;
    }

    const documentKind = getDocumentKindFromPath(path);
    if (documentKind === 'text' || documentKind === 'json') {
      try {
        const opened = await segmentedDocumentPort.open(path);
        if (requestGeneration !== previewOpenGeneration) {
          await segmentedDocumentPort.closeSession(opened.sessionId, false).catch(() => undefined);
          return;
        }
        await applyOpenedSegmentedDocument(path, opened, {
          preview: true,
          previewGeneration: requestGeneration,
        });
        if (requestGeneration !== previewOpenGeneration) return;
        if (opened.recoveryConflictPath) {
          statusMessage = t.segmentedRecoveryConflict({ path: opened.recoveryConflictPath });
        }
      } catch (error) {
        showVisibleError(error, t.previewOpenFailed());
      }
      return;
    }

    const segmentedPreview = previewTabId
      ? tabs.find((tab) => tab.id === previewTabId && isSegmentedTextTab(tab))
      : undefined;
    if (isSegmentedTextTab(segmentedPreview)) {
      await closeSegmentedTab(segmentedPreview);
      if (requestGeneration !== previewOpenGeneration) return;
      if (tabs.some((tab) => tab.id === segmentedPreview.id)) return;
    }

    const { document, error } = await readMarkdownFromPath(path, t.previewOpenFailed());
    if (requestGeneration !== previewOpenGeneration) return;
    if (error) {
      statusMessage = error;
      if (isMissingPathError(error)) {
        removeMissingExplorerPaths([path], false);
        statusMessage = t.removedFromExplorer({ message: error });
      }
      return;
    }
    if (!document) return;

    // 保存当前固定标签页状态（如果当前不是预览）
    if (activeTabId !== previewTabId) {
      saveActiveTabState();
    }

    // 复用现有预览标签页或按设置直接创建固定标签页
    let targetTab: MarkdownTabState;
    const existingPreview =
      filePreviewEnabled && previewTabId
        ? tabs.find((t): t is MarkdownTabState => t.id === previewTabId && isMarkdownTab(t))
        : undefined;

    if (existingPreview) {
      targetTab = existingPreview;
    } else {
      targetTab = createBlankTab('', '');
      tabs = [...tabs, targetTab];
      previewTabId = filePreviewEnabled ? targetTab.id : null;
    }

    const isLargeDocument =
      document.markdown.length > largeDocumentLimit || document.sizeBytes > largeDocumentLimit;

    targetTab.fileName = document.fileName;
    targetTab.filePath = document.path;
    targetTab.nativePath = document.path;
    targetTab.draftId = null;
    targetTab.markdown = document.markdown;
    targetTab.savedMarkdown = document.markdown;
    targetTab.encoding = normalizeMarkdownEncoding(document.encoding);
    targetTab.dirty = false;
    targetTab.lastKnownModifiedAt = document.modifiedAt;
    targetTab.largeDocumentMode = isLargeDocument;
    targetTab.readonlyDocumentMode = isLargeDocument;
    targetTab.diskReadonly = document.readonly;
    targetTab.externalFileChange = createEmptyExternalFileChange();
    targetTab.version = 0;

    tabs = [...tabs];
    activeTabId = targetTab.id;
    loadTabState(targetTab);

    const parentDir = getDirectoryLabel(document.path);
    if (parentDir && parentDir !== t.currentFolder()) {
      if (!currentFolderPath) {
        loadFolder(parentDir).catch(() => undefined);
      } else {
        expandAncestors(document.path, currentFolderPath);
      }
    }
  }

  async function applyOpenedSegmentedDocument(
    path: string,
    opened: OpenSegmentedDocumentResult,
    options: { preview: boolean; previewGeneration?: number },
  ) {
    const fileName = path.replace(/\\/g, '/').split('/').pop() || path;
    const status = await statMarkdownFile(path).catch(() => null);
    if (
      options.previewGeneration !== undefined &&
      options.previewGeneration !== previewOpenGeneration
    ) {
      await segmentedDocumentPort.closeSession(opened.sessionId, false).catch(() => undefined);
      return;
    }
    segmentedSessionRegistry.register(opened);

    if (activeTabId !== previewTabId) {
      saveActiveTabState();
    }

    const reusableBlank = tabs.find((tab) => tab.id === activeTabId && isReusableUntitledTab(tab));
    const existingPreview =
      options.preview && filePreviewEnabled && previewTabId
        ? tabs.find((tab) => tab.id === previewTabId)
        : !options.preview && previewTabId === activeTabId
          ? tabs.find((tab) => tab.id === previewTabId && !tab.dirty)
          : undefined;
    const replacedTab = existingPreview ?? reusableBlank;

    if (isSegmentedTextTab(existingPreview)) {
      // 预览标签被另一文件复用前先关闭旧 session，避免后台索引和日志继续占用资源。
      void segmentedDocumentPort.closeSession(existingPreview.sessionId).catch(() => undefined);
      segmentedSessionRegistry.delete(existingPreview.sessionId);
    }

    const targetTab = createTabForDocument({
      id: replacedTab?.id,
      fileName,
      filePath: path,
      nativePath: path,
      segmentedSession: {
        sessionId: opened.sessionId,
        revision: opened.revision,
        persistedRevision: opened.persistedRevision,
        indexProgress: opened.firstWindow.indexProgress,
        readonly: opened.readonly,
        recoveryConflictPath: opened.recoveryConflictPath ?? null,
      },
      lastKnownModifiedAt: status?.modifiedAt ?? 0,
      diskReadonly: opened.filesystemReadonly ?? Boolean(status?.readonly),
    });

    tabs = replacedTab
      ? tabs.map((tab) => (tab.id === replacedTab.id ? targetTab : tab))
      : [...tabs, targetTab];
    activeTabId = targetTab.id;
    previewTabId =
      options.preview && filePreviewEnabled
        ? targetTab.id
        : replacedTab?.id === previewTabId
          ? null
          : previewTabId;
    loadTabState(targetTab);
    persistWorkspaceState();
    updateWindowTitle();

    const parentDir = getDirectoryLabel(path);
    if (parentDir && parentDir !== t.currentFolder()) {
      if (!currentFolderPath) {
        loadFolder(parentDir).catch(() => undefined);
      } else {
        expandAncestors(path, currentFolderPath);
      }
    }
  }

  // 手动固定当前预览标签页（双击标签页标题）
  function pinPreviewTab() {
    invalidatePendingPreviewOpen();
    if (previewTabId && previewTabId === activeTabId) {
      previewTabId = null;
    }
  }

  // 步骤：关闭除指定标签外的所有标签页（保留标签自动固定）
  async function handleCloseOtherTabs(event: CustomEvent<{ tabId: string }>) {
    invalidatePendingPreviewOpen();
    closeExternalChangeDialog();
    const keepTabId = event.detail.tabId;
    const keepTab = tabs.find((t) => t.id === keepTabId);
    if (!keepTab) return;

    const dirtyTabs = getDirtyTabs(tabs.filter((t) => t.id !== keepTabId));
    if (dirtyTabs.length > 0) {
      const names = dirtyTabs.map((t) => t.fileName).join('、');
      const ok = await confirmAction(t.unsavedChangesCloseTabs({ names }));
      if (ok === false) return;
    }

    invalidatePendingPreviewOpen();
    const tabsToClose = tabs.filter((tab) => tab.id !== keepTabId);
    await closeSegmentedSessions(tabsToClose, true);
    tabs = [keepTab];
    activeTabId = keepTabId;
    // 无论保留的是否是预览标签，都固定它（只剩一个标签不需要预览机制）
    previewTabId = null;
    loadTabState(keepTab);
    updateWindowTitle();
    persistWorkspaceState();
  }

  // 步骤：关闭指定标签页右侧的所有标签页
  async function handleCloseTabsToRight(event: CustomEvent<{ tabId: string }>) {
    invalidatePendingPreviewOpen();
    closeExternalChangeDialog();
    const tabId = event.detail.tabId;
    const tabIndex = tabs.findIndex((t) => t.id === tabId);
    if (tabIndex < 0) return;

    const rightTabs = tabs.slice(tabIndex + 1);
    const dirtyRightTabs = getDirtyTabs(rightTabs);
    if (dirtyRightTabs.length > 0) {
      const names = dirtyRightTabs.map((t) => t.fileName).join('、');
      const ok = await confirmAction(t.unsavedChangesCloseTabs({ names }));
      if (ok === false) return;
    }

    invalidatePendingPreviewOpen();
    await closeSegmentedSessions(rightTabs, true);
    const remaining = tabs.slice(0, tabIndex + 1);
    tabs = remaining;
    if (previewTabId && !remaining.find((t) => t.id === previewTabId)) {
      previewTabId = null;
    }
    if (!remaining.find((t) => t.id === activeTabId)) {
      activeTabId = tabId;
      const tab = tabs.find((t) => t.id === tabId);
      if (tab) loadTabState(tab);
    }
    updateWindowTitle();
    persistWorkspaceState();
  }

  // 步骤：关闭全部标签页，清空状态不保留空白标签
  function handleCloseAllTabs() {
    closeAllTabsWithConfirmation().catch(() => undefined);
  }

  const documentActions = createDocumentActionsController({
    getLargeDocumentLimit: () => largeDocumentLimit,
    getAutoSaveDelayMs: () => autoSaveDelayMs,
    getCreateSnapshotBeforeSave: () => createSnapshotBeforeSave,
    recoveryKey: RECOVERY_KEY,
    getDesktopEnabled: () => desktopEnabled,
    getDirty: () => dirty,
    getAutoSaveEnabled: () => autoSaveEnabled,
    setMarkdown: (value) => {
      markdown = value;
    },
    setSavedMarkdown: (value) => {
      savedMarkdown = value;
    },
    setDirty: (value) => {
      dirty = value;
    },
    setLargeDocumentMode: (value) => {
      largeDocumentMode = value;
    },
    setReadonlyDocumentMode: (value) => {
      readonlyDocumentMode = value;
    },
    setDiskReadonly: (value) => {
      diskReadonly = value;
    },
    getNativePath: () => nativePath,
    setNativePath: (value) => {
      nativePath = value;
    },
    getFileName: () => fileName,
    setFileName: (value) => {
      fileName = value;
    },
    getFilePath: () => filePath,
    setFilePath: (value) => {
      filePath = value;
    },
    getLastKnownModifiedAt: () => lastKnownModifiedAt,
    setLastKnownModifiedAt: (value) => {
      lastKnownModifiedAt = value;
    },
    getExternalFileChange: () => externalFileChange,
    setExternalFileChange: (value) => {
      setExternalFileChangeState(value);
      if (markdownMiniActive) {
        // 小窗观看期间只标记冲突并暂停自动保存，返回主窗口后再让用户处理。
        return;
      }
      // 检测到外部变更时，优先按偏好设置中的默认行为处理。
      if (value.type !== 'none' && !externalChangeDialogOpen) {
        if (!tryHandleExternalFileChangeByPreference(value)) {
          openExternalChangeDialog(value);
        }
      }
    },
    getCurrentFolderPath: () => currentFolderPath,
    getFileInput: () => fileInput,
    getEditor: () => editor,
    beforeMarkdownCommit: flushActiveEditorView,
    getTabs: () => tabs,
    setTabs: (value) => {
      tabs = value;
      persistWorkspaceState();
    },
    getActiveTabId: () => activeTabId,
    setActiveTabId: (value) => {
      activeTabId = value;
      persistWorkspaceState();
    },
    getPreviewTabId: () => previewTabId,
    setPreviewTabId: (value) => {
      previewTabId = value;
      persistWorkspaceState();
    },
    setStatusMessage: (value) => {
      statusMessage = value;
    },
    setRecentFiles: (value) => {
      recentFiles = value;
    },
    saveActiveTabState,
    loadTabState,
    switchTab,
    writeRecoveryDraft,
    updateWindowTitle,
    loadFolder,
    expandAncestors,
  });
  const outlineInteraction = createOutlineInteractionController({
    getMode: () => getActiveEditorMode(),
    getMarkdown: () => markdown,
    getOutline: () => outline,
    getCollapsedOutlineIds: () => collapsedOutlineIds,
    setCollapsedOutlineIds: (value) => {
      collapsedOutlineIds = value;
    },
    getOutlineVisible: () => outlineVisible,
    setOutlineVisible: (value) => {
      outlineVisible = value;
    },
    setActiveOutlineId: (value) => {
      activeOutlineId = value;
    },
    getSuppressOutlineScrollUntil: () => suppressOutlineScrollUntil,
    setSuppressOutlineScrollUntil: (value) => {
      suppressOutlineScrollUntil = value;
    },
    getSemanticPane: () => semanticPane,
    getSourcePane: () => sourcePane,
    getSourceEditor: () => sourceEditor,
    getEditor: () => editor,
    getReadonly: () => readonlyDocumentMode,
    setStatusMessage: (value) => {
      statusMessage = value;
    },
    onExplicitJumpIntent: () => {
      cancelPendingReadingPositionRestore();
      if (mode === 'split') {
        semanticPane?.closest('.editor-grid')?.dispatchEvent(new CustomEvent('nomo:scroll-sync-navigation', {
          detail: { pane: getActiveEditorMode() },
        }));
      }
    },
  });
  const editorInteraction = createEditorInteractionController({
    getEditor: () => editor,
    getLargeDocumentMode: () => largeDocumentMode,
    getMode: () => getActiveEditorMode(),
    getSplitView: () => mode === 'split',
    getOutline: () => outline,
    getSemanticPane: () => semanticPane,
    getSourcePane: () => sourcePane,
    getSourceEditor: () => sourceEditor,
    getPendingSourceScrollTop: () => pendingSourceScrollTop,
    setPendingSourceScrollTop: (value) => {
      pendingSourceScrollTop = value;
    },
    suppressSourceLayoutScroll: () => suppressProgrammaticReadingScroll('source'),
    setSuppressOutlineScrollUntil: (value) => {
      suppressOutlineScrollUntil = value;
    },
    setStatusMessage: (value) => {
      statusMessage = value;
    },
    getSourceLineHeight,
  });
  const imageInsertion = createImageInsertionHandlers({
    getEditor: () => editor,
    getMode: () => getActiveEditorMode(),
    getFileName: () => fileName,
    getNativePath: () => nativePath,
    getSourceEditor: () => sourceEditor,
    getImageContext: () => getImageContext(),
    saveMarkdownFile: (saveAs) => saveMarkdownFile(saveAs),
    setMarkdown: (value) => editor.setMarkdown(value),
    setStatusMessage: (message) => {
      statusMessage = message;
    },
    syncSourceTextareaHeight: () => syncSourceTextareaHeight(),
  });
  const handleEditorDrop = imageInsertion.handleEditorDrop;
  const handleEditorPaste = imageInsertion.handleEditorPaste;
  function updateMarkdown(nextMarkdown: string) {
    editorInteraction.updateMarkdown(nextMarkdown);
    scheduleSplitSemanticRefresh();
  }
  const runMarkdownCommand = editorInteraction.runCommand;
  function runCommand(command: EditorCommand) {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (isSegmentedTextTab(activeTab)) {
      if (command.type === 'undo') segmentedWorkspace?.undo();
      if (command.type === 'redo') segmentedWorkspace?.redo();
      return;
    }
    runMarkdownCommand(command);
  }
  refreshEditorViewportLayout = editorInteraction.refreshEditorViewportLayout;
  function getCurrentAppearancePreferences() {
    return { themeMode, colorThemeId, documentStyleId };
  }

  /**
   * 把当前外观偏好写进本窗 CSS / 编辑器。
   *
   * 不得在提交前等待桌面系统主题 IPC：设置窗广播和系统深浅色事件都会带上
   * `systemScheme` 或使用本窗已有的 `theme`。跟随系统时若没有提示值，先用浏览器
   * 媒体查询上色，避免主窗再卡 1～2 秒。
   *
   * @param options.transition 是否启用短颜色过渡。
   * @param options.systemScheme 已确认的系统深浅色；跟随系统且缺省时用当前 `theme` 或浏览器方案。
   * @param options.writeBootSnapshot 是否把本次结果写入启动快照。
   * @returns 实际写入的已解析主题。
   */
  async function applyCurrentAppearance(options?: {
    transition?: boolean;
    systemScheme?: 'light' | 'dark';
    writeBootSnapshot?: boolean;
  }) {
    const requestId = ++appearanceApplyRequestId;
    const requestedPreferences = getCurrentAppearancePreferences();
    const systemScheme =
      options?.systemScheme ??
      (requestedPreferences.themeMode === 'system' ? theme || getBrowserSystemScheme() : undefined);

    if (!appearanceRuntimeActive || requestId !== appearanceApplyRequestId) {
      return resolveTheme(requestedPreferences, systemScheme);
    }

    const resolved = applyThemeRuntime(requestedPreferences, {
      transition: options?.transition,
      systemScheme,
      desktopEnabled,
      editor,
    });
    theme = resolved.effectiveScheme;
    currentEditorTheme = resolved.editorTheme;
    themeMode = resolved.preferences.themeMode;
    colorThemeId = resolved.preferences.colorThemeId;
    documentStyleId = resolved.preferences.documentStyleId;
    if (options?.writeBootSnapshot) {
      writeThemeBootSnapshot(resolved);
    }
    return resolved;
  }

  /**
   * 在「跟随系统」时把本窗外观对齐到最新系统深浅色。
   *
   * 原生事件若已带上明确 scheme，立刻上色且不再打桌面 IPC。否则直接读操作系统
   * 外观校正，不再先用 WKWebView `matchMedia` 上色——窗口未跟上系统时它会报浅色。
   *
   * @param options.transition 是否启用短颜色过渡。
   * @param options.systemScheme 原生事件带来的深浅色；缺省时走桌面查询。
   * @param options.writeBootSnapshot 校正后是否写启动快照。
   */
  async function syncSystemThemeFromDesktop(options?: {
    transition?: boolean;
    systemScheme?: 'light' | 'dark';
    writeBootSnapshot?: boolean;
  }) {
    if (!appearanceRuntimeActive || themeMode !== 'system') {
      return;
    }

    if (options?.systemScheme) {
      if (options.systemScheme !== theme) {
        await applyCurrentAppearance({
          systemScheme: options.systemScheme,
          transition: options.transition,
          writeBootSnapshot: options.writeBootSnapshot,
        });
      } else if (options.writeBootSnapshot) {
        writeThemeBootSnapshot(
          resolveTheme(getCurrentAppearancePreferences(), options.systemScheme),
        );
      }
      return;
    }

    const systemScheme = await readEffectiveSystemScheme(desktopEnabled);
    if (!appearanceRuntimeActive || themeMode !== 'system') {
      return;
    }
    if (systemScheme === theme) {
      if (options?.writeBootSnapshot) {
        writeThemeBootSnapshot(resolveTheme(getCurrentAppearancePreferences(), systemScheme));
      }
      return;
    }
    await applyCurrentAppearance({
      systemScheme,
      transition: options?.transition,
      writeBootSnapshot: options?.writeBootSnapshot,
    });
  }

  async function toggleTheme() {
    const previousMode = themeMode;
    const nextMode: ThemeMode = theme === 'light' ? 'dark' : 'light';
    themeMode = nextMode;
    const resolved = await applyCurrentAppearance({ transition: true });
    try {
      await updateAppSetting('themeMode', nextMode);
      writeThemeBootSnapshot(resolved);
    } catch {
      themeMode = previousMode;
      await applyCurrentAppearance({ transition: true });
      statusMessage = t.settingsSaveFailed();
    }
  }
  const updateContentWidth = editorSettings.updateContentWidth;
  const isOutlineItemExpandable = outlineInteraction.isOutlineItemExpandable;
  const toggleOutlineItemExpanded = outlineInteraction.toggleOutlineItemExpanded;
  const expandAllOutline = outlineInteraction.expandAllOutline;
  const collapseAllOutline = outlineInteraction.collapseAllOutline;
  const pruneCollapsedOutlineIds = outlineInteraction.pruneCollapsedOutlineIds;
  const syncSourceTextareaHeight = editorInteraction.syncSourceTextareaHeight;
  const openMarkdownFile = documentActions.openMarkdownFile;
  const saveMarkdownDocument = documentActions.saveMarkdownFile;
  const createMarkdownFile = documentActions.createNewFile;
  const _documentCloseTab = documentActions.closeTab;
  const refreshRecentFiles = documentActions.refreshRecentFiles;
  const reloadMarkdownExternalFile = documentActions.reloadExternalFile;
  const overwriteMarkdownExternalFile = documentActions.overwriteExternalFile;
  const checkMarkdownExternalFileChange = documentActions.checkExternalFileChange;

  async function createNewFile() {
    try {
      if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
      await flushSegmentedDocumentBeforeTransition(
        tabs.find((tab) => tab.id === activeTabId),
        segmentedWorkspace,
        segmentedDocumentPort,
      );
      createMarkdownFile();
    } catch (error) {
      statusMessage = error instanceof Error ? error.message : t.saveFileFailed();
    }
  }

  async function openDocumentPath(
    path: string,
    options: { message: string; fallbackMessage: string },
  ): Promise<boolean> {
    if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return false;
    invalidatePendingPreviewOpen();
    const existingTab = tabs.find(
      (tab) => tab.nativePath != null && sameNativePath(tab.nativePath, path),
    );
    if (existingTab) {
      await switchTab(existingTab.id);
      if (existingTab.id === previewTabId) previewTabId = null;
      statusMessage = t.switchedToOpenedTab();
      return activeTabId === existingTab.id;
    }

    // 新文档挂载会销毁当前分段 Core；刷新必须发生在路由读取及活动标签替换之前。
    await flushSegmentedDocumentBeforeTransition(
      tabs.find((tab) => tab.id === activeTabId),
      segmentedWorkspace,
      segmentedDocumentPort,
    );

    const routed = await openDocumentByPath(path, {
      openMarkdown: (markdownPath) => readMarkdownFromPath(markdownPath, options.fallbackMessage),
      openSegmented: (segmentedPath) => segmentedDocumentPort.open(segmentedPath),
    });

    if (routed.documentKind === 'markdown') {
      if (routed.value.error) {
        statusMessage = routed.value.error;
        return false;
      }
      if (routed.value.document) {
        await documentActions.applyNativeDocument(routed.value.document, options.message);
        return true;
      }
      return false;
    }

    if (routed.value.documentKind !== routed.documentKind) {
      await segmentedDocumentPort.closeSession(routed.value.sessionId).catch(() => undefined);
      throw new Error(`Segmented document kind mismatch: ${routed.value.documentKind}`);
    }
    await applyOpenedSegmentedDocument(path, routed.value, { preview: false });
    statusMessage = routed.value.recoveryConflictPath
      ? t.segmentedRecoveryConflict({ path: routed.value.recoveryConflictPath })
      : options.message;
    const openedFileName = path.replace(/\\/g, '/').split('/').pop() || path;
    await rememberRecentEntry(path, 'file', openedFileName, 0).catch(() => undefined);
    await refreshRecentFiles();
    return true;
  }

  async function openDroppedMarkdown(paths: string[]) {
    const target = findDroppedDocumentPath(paths);
    if (!target) {
      statusMessage = t.dragDropNoMarkdown();
      return;
    }
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (isMarkdownTab(activeTab) && activeTab.dirty) {
      writeRecoveryDraft('drag-open-blocked');
      statusMessage = t.dragOpenBlockedUnsaved();
      return;
    }
    await openDocumentPath(target, {
      message: t.openedByDragDrop(),
      fallbackMessage: t.dragOpenFailed(),
    }).catch((error) => {
      showVisibleError(error, t.dragOpenFailed());
    });
  }

  async function openFileDialog() {
    if (!desktopEnabled) {
      fileInput.click();
      return;
    }
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (isMarkdownTab(activeTab) && activeTab.dirty) {
      writeRecoveryDraft('open-dialog');
    }
    const path = await pickDocumentPathWithDialog();
    if (!path) return;
    await openTargetWithBehavior({ kind: 'documents', paths: [path] }).catch((error) => {
      showVisibleError(error, t.openFileFailed());
    });
  }

  async function openFilePathInCurrentWindow(path: string) {
    if (!desktopEnabled) return;
    await openDocumentPath(path, {
      message: t.recentFileOpened(),
      fallbackMessage: t.openRecentFailed(),
    }).catch((error) => {
      showVisibleError(error, t.openRecentFailed());
    });
  }

  async function saveMarkdownFile(saveAs = false): Promise<boolean> {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isSegmentedTextTab(activeTab)) {
      return saveMarkdownDocument(saveAs);
    }

    const savingTabId = activeTab.id;
    const savingSessionId = activeTab.sessionId;
    const preparedSave = await segmentedWorkspace?.prepareSave();
    const frozenRevision = preparedSave?.revision ?? activeTab.revision;

    const requiresSaveAs = saveAs || activeTab.diskReadonly;
    let targetPath: string | undefined;
    if (requiresSaveAs) {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const selected = await save({
        defaultPath: activeTab.fileName,
        filters: [
          {
            name: activeTab.documentKind === 'json' ? 'JSON' : 'Text',
            extensions: [activeTab.documentKind === 'json' ? 'json' : 'txt'],
          },
        ],
      });
      if (!selected) return false;
      targetPath = selected;
    }

    try {
      const result = await segmentedDocumentPort.saveRevision({
        sessionId: savingSessionId,
        revision: frozenRevision,
        targetPath,
      });
      if (result.sessionId !== savingSessionId) {
        throw new Error(`Segmented save returned another session: ${result.sessionId}`);
      }
      const targetTab = tabs.find((tab) => tab.id === savingTabId);
      if (!isSegmentedTextTab(targetTab) || targetTab.sessionId !== savingSessionId) return true;
      const observedState =
        activeTabId === savingTabId
          ? segmentedWorkspace?.applySaveResult(savingSessionId, result)
          : null;
      const nextState = reconcileSegmentedSaveState(result, observedState);
      targetTab.persistedRevision = nextState.persistedRevision;
      targetTab.revision = nextState.revision;
      targetTab.dirty = nextState.dirty;
      targetTab.lastKnownModifiedAt = result.modifiedAt;
      targetTab.diskReadonly = nextState.filesystemReadonly ?? false;
      if (targetPath) {
        targetTab.nativePath = targetPath;
        targetTab.filePath = targetPath;
        targetTab.fileName = targetPath.replace(/\\/g, '/').split('/').pop() || targetPath;
        targetTab.externalFileChange = createEmptyExternalFileChange();
      }
      segmentedSessionRegistry.update(savingSessionId, {
        revision: nextState.revision,
        persistedRevision: nextState.persistedRevision,
        readonly: nextState.readonly,
      });
      ignoredSegmentedExternalChanges.delete(savingSessionId);
      if (activeTabId === savingTabId) {
        dirty = targetTab.dirty;
        nativePath = targetTab.nativePath;
        filePath = targetTab.filePath;
        fileName = targetTab.fileName;
        lastKnownModifiedAt = targetTab.lastKnownModifiedAt;
        diskReadonly = targetTab.diskReadonly;
        externalFileChange = targetTab.externalFileChange;
      }
      tabs = [...tabs];
      persistWorkspaceState();
      if (activeTabId === savingTabId) statusMessage = t.saved();
      if (targetTab.nativePath) {
        await rememberRecentEntry(targetTab.nativePath, 'file', targetTab.fileName, 0).catch(
          () => undefined,
        );
        await refreshRecentFiles();
      }
      return true;
    } catch (error) {
      if (activeTabId === savingTabId) {
        showVisibleError(error, t.saveFileFailed());
      }
      return false;
    }
  }

  async function checkExternalFileChange() {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isSegmentedTextTab(activeTab)) {
      await checkMarkdownExternalFileChange();
      return;
    }
    const checkingTabId = activeTab.id;
    const checkingSessionId = activeTab.sessionId;
    try {
      const result = await segmentedDocumentPort.checkExternalChange(checkingSessionId);
      if (result.sessionId !== checkingSessionId) return;
      // 原子保存窗口内的临时身份变化由保存结果收口，轮询不得把自身写入误报为外部冲突。
      if (result.saveInProgress) return;
      const targetTab = tabs.find(
        (tab) =>
          tab.id === checkingTabId &&
          isSegmentedTextTab(tab) &&
          tab.sessionId === checkingSessionId,
      );
      if (!isSegmentedTextTab(targetTab)) return;
      const reconciledCheck = reconcileSegmentedExternalChangeCheck(result, {
        sessionId: targetTab.sessionId,
        revision: targetTab.revision,
        dirty: targetTab.dirty,
        hasPendingEdits:
          activeTabId === checkingTabId && Boolean(segmentedWorkspace?.hasPendingEdits()),
      });
      if (!reconciledCheck) return;
      const { dirtyAtDetection } = reconciledCheck;
      const changeToken = getSegmentedExternalChangeToken(result);
      if (result.type === 'none') {
        ignoredSegmentedExternalChanges.delete(checkingSessionId);
      } else if (
        changeToken &&
        ignoredSegmentedExternalChanges.get(checkingSessionId) === changeToken
      ) {
        // 保留冲突状态以暂停自动保存；同一磁盘身份只是不再弹框，新身份仍走正常流程。
        if (activeTabId === checkingTabId) {
          closeExternalChangeDialog();
        }
        return;
      } else {
        ignoredSegmentedExternalChanges.delete(checkingSessionId);
      }
      const change: ExternalFileChangeState =
        result.type === 'none'
          ? createEmptyExternalFileChange()
          : {
              type: result.type,
              path: targetTab.nativePath,
              modifiedAt: result.modifiedAt,
              dirtyAtDetection,
              message:
                result.type === 'deleted'
                  ? t.externalFileDeleted()
                  : dirtyAtDetection
                    ? t.externalFileModifiedDirty()
                    : t.externalFileModifiedClean(),
            };
      if (activeTabId !== checkingTabId) {
        // IPC 返回后标签可能已切换；只更新发起检查的标签，绝不驱动当前标签的 reload/overwrite。
        targetTab.externalFileChange = change;
        tabs = [...tabs];
        persistWorkspaceState();
        return;
      }
      setExternalFileChangeState(change);
      const segmentedIgnoreTarget = changeToken
        ? { sessionId: checkingSessionId, changeToken }
        : undefined;
      if (
        change.type !== 'none' &&
        !tryHandleExternalFileChangeByPreference(change, segmentedIgnoreTarget)
      ) {
        openExternalChangeDialog(change, changeToken);
      }
    } catch (error) {
      const stillCurrent = tabs.some(
        (tab) =>
          tab.id === checkingTabId &&
          isSegmentedTextTab(tab) &&
          tab.sessionId === checkingSessionId &&
          tab.id === activeTabId,
      );
      if (stillCurrent) {
        statusMessage = error instanceof Error ? error.message : t.fileStatusCheckFailed();
      }
    }
  }

  async function reloadExternalFile() {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isSegmentedTextTab(activeTab)) {
      await reloadMarkdownExternalFile();
      return;
    }
    if (!activeTab.nativePath) return;

    const reloadingTabId = activeTab.id;
    const oldSessionId = activeTab.sessionId;
    await segmentedWorkspace?.flushPendingEdits();
    // Rust 先完整构造候选会话再原子替换；失败时旧 session 与 recovery 保持可用。
    const opened = await segmentedDocumentPort.reloadSession(oldSessionId);
    const targetTab = tabs.find((tab) => tab.id === reloadingTabId);
    if (!isSegmentedTextTab(targetTab) || targetTab.sessionId !== oldSessionId) {
      await segmentedDocumentPort.closeSession(opened.sessionId, true).catch(() => undefined);
      return;
    }
    segmentedSessionRegistry.delete(oldSessionId);
    ignoredSegmentedExternalChanges.delete(oldSessionId);
    segmentedSessionRegistry.register(opened);
    targetTab.sessionId = opened.sessionId;
    targetTab.revision = opened.revision;
    targetTab.persistedRevision = opened.persistedRevision;
    targetTab.recoveryConflictPath = opened.recoveryConflictPath ?? null;
    targetTab.indexProgress = opened.firstWindow.indexProgress;
    targetTab.dirty = false;
    targetTab.selection = null;
    targetTab.diskReadonly = opened.filesystemReadonly ?? false;
    targetTab.externalFileChange = createEmptyExternalFileChange();
    tabs = [...tabs];
    if (activeTabId === reloadingTabId) {
      loadTabState(targetTab);
      statusMessage = t.reloadedExternalVersion();
    }
    persistWorkspaceState();
  }

  async function overwriteExternalFile() {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isSegmentedTextTab(activeTab)) {
      await overwriteMarkdownExternalFile();
      return;
    }
    const savingTabId = activeTab.id;
    const savingSessionId = activeTab.sessionId;
    const preparedSave = await segmentedWorkspace?.prepareSave();
    const frozenRevision = preparedSave?.revision ?? activeTab.revision;
    try {
      const result = await segmentedDocumentPort.saveRevision({
        sessionId: savingSessionId,
        revision: frozenRevision,
        overwriteExternal: true,
      });
      if (result.sessionId !== savingSessionId) {
        throw new Error(`Segmented save returned another session: ${result.sessionId}`);
      }
      const targetTab = tabs.find((tab) => tab.id === savingTabId);
      if (!isSegmentedTextTab(targetTab) || targetTab.sessionId !== savingSessionId) return;
      const observedState =
        activeTabId === savingTabId
          ? segmentedWorkspace?.applySaveResult(savingSessionId, result)
          : null;
      const nextState = reconcileSegmentedSaveState(result, observedState);
      targetTab.persistedRevision = nextState.persistedRevision;
      targetTab.revision = nextState.revision;
      targetTab.dirty = nextState.dirty;
      targetTab.lastKnownModifiedAt = result.modifiedAt;
      targetTab.diskReadonly = nextState.filesystemReadonly ?? false;
      targetTab.externalFileChange = createEmptyExternalFileChange();
      ignoredSegmentedExternalChanges.delete(savingSessionId);
      if (activeTabId === savingTabId) {
        setExternalFileChangeState(targetTab.externalFileChange);
      }
      tabs = [...tabs];
      persistWorkspaceState();
      if (activeTabId === savingTabId) statusMessage = t.overwrittenExternalVersion();
    } catch (error) {
      if (activeTabId === savingTabId) {
        showVisibleError(error, t.saveFileFailed());
      }
    }
  }

  async function closeSegmentedTab(
    tabToClose: Extract<Tab, { documentKind: 'text' | 'json' }>,
    discardWithoutConfirmation = false,
  ) {
    const wasActive = tabToClose.id === activeTabId;
    try {
      if (wasActive) {
        await segmentedWorkspace?.flushPendingEdits();
      }
      await segmentedDocumentPort.flushJournal(tabToClose.sessionId, tabToClose.revision);
    } catch (error) {
      // 恢复日志未确认落盘时保持标签打开，并把具体失败原因暴露给用户。
      showVisibleError(error, t.saveFileFailed());
      return;
    }

    let discardChanges = discardWithoutConfirmation;
    if (tabToClose.dirty && !discardWithoutConfirmation) {
      const choice = await confirmAction(t.confirmCloseModifiedFile(), {
        title: tabToClose.fileName,
        okLabel: t.discardChanges(),
        cancelLabel: t.cancel(),
        saveLabel: tabToClose.nativePath ? t.save() : undefined,
      });
      if (choice === false) return;
      if (choice === 'save') {
        const saved = wasActive
          ? await saveMarkdownFile(false)
          : await segmentedDocumentPort
              .saveRevision({
                sessionId: tabToClose.sessionId,
                revision: tabToClose.revision,
              })
              .then((result) => {
                if (result.sessionId !== tabToClose.sessionId) {
                  throw new Error(`Segmented save returned another session: ${result.sessionId}`);
                }
                tabToClose.persistedRevision = result.persistedRevision;
                tabToClose.revision = result.currentRevision;
                tabToClose.dirty = result.dirty;
                tabToClose.lastKnownModifiedAt = result.modifiedAt;
                tabToClose.diskReadonly = result.filesystemReadonly ?? false;
                return true;
              })
              .catch((error) => {
                showVisibleError(error, t.saveFileFailed());
                return false;
              });
        if (!saved) return;
      } else {
        discardChanges = true;
      }
    }

    try {
      await segmentedDocumentPort.closeSession(tabToClose.sessionId, discardChanges);
    } catch (error) {
      showVisibleError(error, t.saveFileFailed());
      return;
    }
    segmentedSessionRegistry.delete(tabToClose.sessionId);
    ignoredSegmentedExternalChanges.delete(tabToClose.sessionId);
    const index = tabs.findIndex((tab) => tab.id === tabToClose.id);
    tabs = tabs.filter((tab) => tab.id !== tabToClose.id);
    if (previewTabId === tabToClose.id) previewTabId = null;

    if (wasActive) {
      if (tabs.length > 0) {
        const nextTab = tabs[Math.min(index, tabs.length - 1)];
        activeTabId = nextTab.id;
        loadTabState(nextTab);
      } else {
        activeTabId = '';
        dirty = false;
        fileName = '';
        filePath = '';
        nativePath = null;
        externalFileChange = createEmptyExternalFileChange();
      }
      updateWindowTitle();
    }
    persistWorkspaceState();
  }

  // 包装 closeTab：预览标签页直接关闭无需确认
  async function closeTab(tabId: string, event?: Event, discardWithoutConfirmation = false) {
    event?.stopPropagation();
    if (markdownMiniActive && activeTabId === tabId) {
      if (!(await requestMarkdownMiniReturn({ showExternalChange: false }))) return;
    }
    invalidatePendingPreviewOpen();
    if (externalChangeDialogTargetTabId === tabId) {
      closeExternalChangeDialog();
    }

    // 关闭前保存阅读位置
    if (activeTabId === tabId) {
      saveActiveTabState();
      void flushReadingPositions();
    }

    const tabToClose = tabs.find((t) => t.id === tabId);
    if (!tabToClose) {
      logCloseDiagnostics('closeTab: 未找到目标标签', {
        tabId,
        activeTabId,
        tabCount: tabs.length,
      });
      return;
    }

    if (isSegmentedTextTab(tabToClose)) {
      await closeSegmentedTab(tabToClose, discardWithoutConfirmation);
      return;
    }

    logCloseDiagnostics('closeTab: 收到关闭请求', {
      tabId,
      activeTabId,
      previewTabId,
      targetDirty: tabToClose.dirty,
      appDirty: dirty,
      isActiveTarget: tabId === activeTabId,
      targetRevision: tabToClose.version,
      appVersion: version,
      targetMarkdownLength: isMarkdownTab(tabToClose) ? tabToClose.markdown.length : null,
      appMarkdownLength: markdown.length,
      targetSavedMarkdownLength: isMarkdownTab(tabToClose) ? tabToClose.savedMarkdown.length : null,
      appSavedMarkdownLength: savedMarkdown.length,
    });

    const dirtyTabToClose = getDirtyTabs([tabToClose]).find((tab) => tab.id === tabId);
    logCloseDiagnostics('closeTab: 实时 dirty 判定完成', {
      tabId,
      hasDirtyTabToClose: Boolean(dirtyTabToClose),
      targetDirtyAfterCheck: tabToClose.dirty,
      appDirty: dirty,
      dirtyTabRevision: isMarkdownTab(dirtyTabToClose)
        ? dirtyTabToClose.version
        : dirtyTabToClose?.revision,
      dirtyMarkdownLength: isMarkdownTab(dirtyTabToClose) ? dirtyTabToClose.markdown.length : null,
      dirtySavedMarkdownLength: isMarkdownTab(dirtyTabToClose)
        ? dirtyTabToClose.savedMarkdown.length
        : null,
    });

    if (dirtyTabToClose && !tabToClose.dirty) {
      Object.assign(tabToClose, dirtyTabToClose);
      tabs = [...tabs];
      logCloseDiagnostics('closeTab: 已把活动标签 dirty 状态写回 tabs', {
        tabId,
        targetDirty: tabToClose.dirty,
        targetRevision: tabToClose.version,
      });
    }

    if (tabId === previewTabId && !dirtyTabToClose) {
      logCloseDiagnostics('closeTab: 干净预览标签直接关闭', { tabId });
      const wasActive = activeTabId === tabId;
      const index = tabs.findIndex((t) => t.id === tabId);
      tabs = tabs.filter((t) => t.id !== tabId);
      previewTabId = null;

      if (wasActive) {
        if (tabs.length > 0) {
          const newActiveIndex = Math.min(index, tabs.length - 1);
          activeTabId = tabs[newActiveIndex].id;
          loadTabState(tabs[newActiveIndex]);
        } else {
          activeTabId = '';
          markdown = '';
          savedMarkdown = '';
          fileName = '';
          filePath = '';
          nativePath = null;
          dirty = false;
          lastKnownModifiedAt = 0;
          largeDocumentMode = false;
          readonlyDocumentMode = false;
          diskReadonly = false;
          externalFileChange = createEmptyExternalFileChange();
          outline = [];
          isSwitchingTab = true;
          try {
            if (editor) {
              editor.setMarkdown('', { reason: 'switch-tab', dirty: false, savedMarkdown: '' });
            }
          } finally {
            isSwitchingTab = false;
          }
        }
        updateWindowTitle();
      }
      persistWorkspaceState();
      return;
    }

    logCloseDiagnostics('closeTab: 交给 documentActions.closeTab 处理确认', {
      tabId,
      targetDirty: tabToClose.dirty,
    });
    await _documentCloseTab(tabId, event, discardWithoutConfirmation);

    // 关闭最后一个普通标签后清空编辑器状态
    if (tabs.length === 0) {
      markdown = '';
      savedMarkdown = '';
      fileName = '';
      filePath = '';
      nativePath = null;
      dirty = false;
      lastKnownModifiedAt = 0;
      largeDocumentMode = false;
      readonlyDocumentMode = false;
      diskReadonly = false;
      externalFileChange = createEmptyExternalFileChange();
      outline = [];
      isSwitchingTab = true;
      try {
        if (editor) {
          editor.setMarkdown('', { reason: 'switch-tab', dirty: false, savedMarkdown: '' });
        }
      } finally {
        isSwitchingTab = false;
      }
      updateWindowTitle();
    }
  }

  function getDirtyTabs(candidateTabs: Tab[]) {
    const dirtyTabs = candidateTabs.filter((tab) => tab.dirty);
    const activeTab = candidateTabs.find((tab) => tab.id === activeTabId);
    if (dirty && isMarkdownTab(activeTab) && !dirtyTabs.some((tab) => tab.id === activeTab.id)) {
      return [
        ...dirtyTabs,
        {
          ...activeTab,
          markdown,
          savedMarkdown,
          dirty: true,
          version,
        },
      ];
    }
    return dirtyTabs;
  }
  const jumpToOutlineItem = outlineInteraction.jumpToOutlineItem;
  const moveOutlineSection = outlineInteraction.moveOutlineSection;
  const updateActiveOutlineFromSourceScroll =
    outlineInteraction.updateActiveOutlineFromSourceScroll;
  const updateActiveOutlineFromSemanticScroll =
    outlineInteraction.updateActiveOutlineFromSemanticScroll;

  async function handleRefreshFolder() {
    if (currentFolderPath) {
      await loadFolder(currentFolderPath);
    }
  }

  function handleCollapseAll() {
    expandedFolders = new Set();
    // 保留根目录展开，只折叠子文件夹
  }

  // 步骤：打开删除确认对话框
  function handleDeleteNode(event: CustomEvent<{ path: string; isDir: boolean }>) {
    const { path, isDir } = event.detail;
    deleteConfirmPath = path;
    deleteConfirmIsDir = isDir;
    deleteConfirmName = path.includes('\\')
      ? path.slice(path.lastIndexOf('\\') + 1)
      : path.includes('/')
        ? path.slice(path.lastIndexOf('/') + 1)
        : path;
    deleteConfirmOpen = true;
  }

  // 步骤：执行删除操作
  async function executeDelete() {
    const path = deleteConfirmPath;
    const isDir = deleteConfirmIsDir;
    const typeLabel = isDir ? t.folder() : t.file();
    deleteConfirmOpen = false;

    try {
      await deleteFile(path);
      // 关闭受影响的标签页（精确匹配或以文件夹路径开头）
      const affectedTabs = tabs.filter((t) =>
        isDir
          ? t.nativePath && pathEqualsOrDescendsFrom(t.nativePath, path)
          : t.nativePath != null && sameNativePath(t.nativePath, path),
      );
      for (const tab of affectedTabs) {
        if (tab.id === previewTabId) {
          // 预览标签直接移除
          if (isSegmentedTextTab(tab)) {
            await segmentedDocumentPort.closeSession(tab.sessionId, true).catch(() => undefined);
            segmentedSessionRegistry.delete(tab.sessionId);
          }
          tabs = tabs.filter((t) => t.id !== tab.id);
          previewTabId = null;
        } else {
          await closeTab(tab.id);
        }
      }
      // 如果删光了所有标签，清空状态不自动创建标签
      if (tabs.length === 0) {
        activeTabId = '';
        markdown = '';
        savedMarkdown = '';
        fileName = '';
        filePath = '';
        nativePath = null;
        dirty = false;
        lastKnownModifiedAt = 0;
        largeDocumentMode = false;
        readonlyDocumentMode = false;
        diskReadonly = false;
        externalFileChange = createEmptyExternalFileChange();
        outline = [];
        isSwitchingTab = true;
        try {
          if (editor) {
            editor.setMarkdown('', { reason: 'switch-tab', dirty: false, savedMarkdown: '' });
          }
        } finally {
          isSwitchingTab = false;
        }
      }
      // 刷新文件夹
      if (currentFolderPath) {
        await loadFolder(currentFolderPath);
      }
      statusMessage = t.deletedType({ type: typeLabel });
    } catch (error) {
      statusMessage = t.deleteFailed({ error });
    }
  }

  function closeDeleteConfirm() {
    deleteConfirmOpen = false;
  }

  async function handleCreateNode(
    event: CustomEvent<{ parentPath: string; type: 'folder' | 'file'; name: string }>,
  ) {
    const { parentPath, type, name } = event.detail;
    let finalName = name || (type === 'folder' ? t.newFolder() : t.untitledMarkdown());
    finalName = finalName.replace(/[<>:"/\\|?*]/g, '');
    if (!finalName) finalName = type === 'folder' ? t.newFolder() : t.untitledMarkdown();
    if (type === 'file' && !finalName.toLowerCase().endsWith('.md')) {
      finalName += '.md';
    }

    const { join } = await import('@tauri-apps/api/path');
    let targetPath = await join(parentPath, finalName);

    const { statMarkdownFile } = await import('../lib/desktop/tauriStorage');
    let suffix = 1;
    let currentName = finalName;
    while (true) {
      const stat = await statMarkdownFile(targetPath).catch(() => null);
      if (!stat || !stat.exists) break;
      if (type === 'file') {
        const base = finalName.replace(/\.md$/i, '');
        currentName = `${base} (${suffix}).md`;
      } else {
        currentName = `${finalName} (${suffix})`;
      }
      targetPath = await join(parentPath, currentName);
      suffix++;
    }

    if (type === 'folder') {
      const { createFolder } = await import('../lib/desktop/tauriStorage');
      await createFolder(targetPath).catch((err) => {
        statusMessage = t.createFolderFailed({ error: err });
      });
      await loadFolder(currentFolderPath);
      expandAncestors(targetPath, currentFolderPath);
    } else {
      const { saveMarkdownNative } = await import('../lib/desktop/tauriStorage');
      const defaultContent = `# ${currentName.replace(/\.md$/i, '')}\n\n`;
      const result = await saveMarkdownNative(targetPath, defaultContent, currentName);
      if (result) {
        await loadFolder(currentFolderPath);
        expandAncestors(targetPath, currentFolderPath);
        openFilePathInCurrentWindow(targetPath);
      }
    }
  }

  async function handleRenameNode(event: CustomEvent<{ path: string; newName: string }>) {
    const { path, newName } = event.detail;
    let finalName = newName.replace(/[<>:"/\\|?*]/g, '');
    if (!finalName) return;

    const { dirname, join } = await import('@tauri-apps/api/path');
    const parentDir = await dirname(path);
    const targetPath = await join(parentDir, finalName);

    if (path === targetPath) return;

    const renameBlock = getOpenDocumentRenameBlock(tabs, path, targetPath);
    if (renameBlock) {
      statusMessage = t.renameOpenDocumentBlocked();
      return;
    }

    const { renameFile } = await import('../lib/desktop/tauriStorage');
    try {
      await renameFile(path, targetPath);
    } catch (err) {
      statusMessage = t.renameFailed({ error: err });
      return;
    }

    await loadFolder(currentFolderPath);

    tabs.forEach((t) => {
      if (t.nativePath && pathEqualsOrDescendsFrom(t.nativePath, path)) {
        const newNativePath = t.nativePath.replace(path, targetPath);
        t.nativePath = newNativePath;
        t.filePath = newNativePath;
        if (sameNativePath(t.nativePath, targetPath)) {
          t.fileName = finalName;
        }
        if (activeTabId === t.id) {
          fileName = t.fileName;
          filePath = t.filePath;
          nativePath = t.nativePath;
        }
      }
    });
    tabs = [...tabs];
    persistWorkspaceState();
  }

  const unsubscribe = editor.subscribe(syncFromEditor);

  async function applyAppPreferences(
    preferences: AppPreferences,
    options: {
      applyEditorMode?: boolean;
      refreshInterfaceChrome?: boolean;
      writeBootSnapshot?: boolean;
    } = {},
  ) {
    themeMode = preferences.themeMode;
    colorThemeId = preferences.colorThemeId;
    documentStyleId = preferences.documentStyleId;
    await applyCurrentAppearance({
      transition: true,
      writeBootSnapshot: options.writeBootSnapshot,
    });
    interfaceLanguage = preferences.interfaceLanguage;
    interfaceLocale = applyInterfaceLanguagePreference(interfaceLanguage);
    if (options.refreshInterfaceChrome) {
      void refreshInterfaceLanguageChrome(desktopEnabled);
    }
    fontSize = preferences.fontSize;
    lineHeight = preferences.lineHeight;
    contentWidthPercent = preferences.contentWidthPercent;
    preferredEditorMode = preferences.editorMode;
    splitViewLayout = preferences.splitViewLayout;
    splitLeftPercent = preferences.splitLeftPercent;
    imageSettings = preferences.imageHandlingSettings;
    openDefaultBehavior = preferences.openDefaultBehavior;
    filePreviewEnabled = preferences.filePreviewEnabled;
    closeWindowBehavior = preferences.closeWindowBehavior;
    externalFileChangeBehavior = preferences.externalFileChangeBehavior;
    focusMode = preferences.sidebarHidden;
    toolbarHidden = preferences.toolbarHidden;
    if (focusMode || toolbarHidden) {
      closeToolbarTransientPanels();
    }
    outlineVisible = preferences.outlineVisible;
    writingStatsVisible = preferences.writingStatsVisible;
    writingStatsMetric = preferences.writingStatsMetric;
    readingTimeVisible = preferences.readingTimeVisible;
    markdownLintEnabled = preferences.markdownLintEnabled;
    markdownLintRuleSet = preferences.markdownLintRuleSet;
    largeDocumentLimit = preferences.largeDocumentLimit;
    autoSaveDelayMs = preferences.autoSaveDelayMs;
    createSnapshotBeforeSave = preferences.createSnapshotBeforeSave;
    defaultCodeBlockLanguage = preferences.defaultCodeBlockLanguage;
    defaultDiagramType = preferences.defaultDiagramType;
    zoomPercent = preferences.zoomPercent;
    ctrlWheelZoomEnabled = preferences.ctrlWheelZoomEnabled;
    outlineDefaultExpandLevel = preferences.outlineDefaultExpandLevel;
    codeBlockLineNumbersVisible = preferences.codeBlockLineNumbersVisible;
    codeBlockIndent = preferences.codeBlockIndent;
    inlineCodeRenderingEnabled = preferences.inlineCodeRenderingEnabled;
    copyMarkdownSyntaxEnabled = preferences.copyMarkdownSyntaxEnabled;
    shortcutPreferences = preferences.shortcutPreferences;
    developerMode = preferences.developerMode;
    softwareUpdateAutoCheckEnabled = preferences.softwareUpdateAutoCheckEnabled;

    if (!filePreviewEnabled) {
      invalidatePendingPreviewOpen();
      previewTabId = null;
    }
    if (autoSaveEnabled && !preferences.autoSaveEnabled) {
      documentActions.cancelPendingAutoSaves();
    }
    autoSaveEnabled = preferences.autoSaveEnabled;

    applyTypographySettings(fontSize, lineHeight);
    applyEditorLayoutSettings(contentWidthPercent);
    applyZoomSetting(zoomPercent, { onFrame: refreshEditorViewportLayout });
    applyCodeBlockLineNumberSetting(codeBlockLineNumbersVisible);
    document.documentElement.dataset.codeBlockIndent = codeBlockIndent;
    editor.updateOptions({ inlineCodeRenderingEnabled, copyMarkdownSyntaxEnabled });
    applyOutlineDefaultExpansion();

    const shouldBeLargeDocument = markdown.length > largeDocumentLimit;
    if (!shouldBeLargeDocument && largeDocumentMode) {
      largeDocumentMode = false;
      readonlyDocumentMode = false;
      mode = preferredEditorMode;
      editor.updateOptions({ mode: getCoreModeForView(mode) });
    } else if (shouldBeLargeDocument && !largeDocumentMode) {
      largeDocumentMode = true;
      readonlyDocumentMode = true;
      mode = 'source';
      editor.updateOptions({ mode: 'source' });
      statusMessage = t.largeDocumentReadonly();
    }

    if (options.applyEditorMode && !largeDocumentMode) {
      mode = preferences.editorMode;
      editor.updateOptions({ mode: getCoreModeForView(preferences.editorMode) });
    }

    if (preferences.developerMode) {
      enableLogger();
    } else {
      disableLogger();
    }

    persistWorkspaceState();
  }

  function getCurrentAppPreferences(): AppPreferences {
    return normalizeAppPreferences({
      themeMode,
      colorThemeId,
      documentStyleId,
      interfaceLanguage,
      editorMode: preferredEditorMode,
      splitViewLayout,
      splitLeftPercent,
      autoSaveEnabled,
      autoSaveDelayMs,
      createSnapshotBeforeSave,
      fontSize,
      lineHeight,
      contentWidthPercent,
      largeDocumentLimit,
      openDefaultBehavior,
      filePreviewEnabled,
      closeWindowBehavior,
      externalFileChangeBehavior,
      sidebarHidden: focusMode,
      toolbarHidden,
      outlineVisible,
      writingStatsVisible,
      writingStatsMetric,
      readingTimeVisible,
      markdownLintEnabled,
      markdownLintRuleSet,
      defaultCodeBlockLanguage,
      defaultDiagramType,
      zoomPercent,
      ctrlWheelZoomEnabled,
      outlineDefaultExpandLevel,
      codeBlockLineNumbersVisible,
      codeBlockIndent,
      inlineCodeRenderingEnabled,
      copyMarkdownSyntaxEnabled,
      shortcutPreferences,
      imageHandlingSettings: imageSettings,
      developerMode,
      softwareUpdateAutoCheckEnabled,
    });
  }

  async function applyAppPreferencesPatch(
    patch: AppPreferencesPatch,
    options: { systemScheme?: 'light' | 'dark' } = {},
  ) {
    const preferences = normalizeAppPreferences({
      ...getCurrentAppPreferences(),
      ...patch,
      imageHandlingSettings: patch.imageHandlingSettings ?? imageSettings,
      shortcutPreferences: patch.shortcutPreferences ?? shortcutPreferences,
    });
    const patchKeys = Object.keys(patch);
    const appearanceOnly =
      patchKeys.length > 0 &&
      patchKeys.every(
        (key) => key === 'themeMode' || key === 'colorThemeId' || key === 'documentStyleId',
      );

    if (appearanceOnly) {
      themeMode = preferences.themeMode;
      colorThemeId = preferences.colorThemeId;
      documentStyleId = preferences.documentStyleId;
      await applyCurrentAppearance({
        transition: true,
        systemScheme:
          options.systemScheme ??
          (themeMode === 'system' && !('themeMode' in patch) ? theme : undefined),
      });
      return;
    }

    await applyAppPreferences(preferences, {
      applyEditorMode: 'editorMode' in patch,
      refreshInterfaceChrome: false,
    });
  }

  function isSettingsUpdatedPayload(payload: unknown): payload is SettingsUpdatedPayload {
    return Boolean(
      payload &&
      typeof payload === 'object' &&
      (payload as SettingsUpdatedPayload).source === 'settings-window',
    );
  }

  function applyOutlineDefaultExpansion() {
    if (outlineDefaultExpandLevel >= 6) {
      collapsedOutlineIds = new Set();
      return;
    }

    const nextCollapsedIds = new Set<string>();
    outline.forEach((item, index) => {
      const next = outline[index + 1];
      if (item.level >= outlineDefaultExpandLevel && next && next.level > item.level) {
        nextCollapsedIds.add(item.id);
      }
    });
    collapsedOutlineIds = nextCollapsedIds;
  }

  async function reloadAppPreferencesFromSettingsWindow() {
    const preferences = await loadAppPreferences(desktopEnabled);
    await applyAppPreferences(preferences, { applyEditorMode: true, refreshInterfaceChrome: true });
  }

  async function handleSettingsUpdated(payload: unknown) {
    if (!isSettingsUpdatedPayload(payload) || !payload.patch || typeof payload.patch !== 'object') {
      await reloadAppPreferencesFromSettingsWindow();
      return;
    }

    await applyAppPreferencesPatch(payload.patch, {
      systemScheme: isColorScheme(payload.effectiveScheme) ? payload.effectiveScheme : undefined,
    });
  }

  async function maybeOpenFirstRunSample(state: FirstRunSampleState) {
    if (!desktopEnabled) {
      return;
    }

    if (shouldOpenFirstRunSample(state)) {
      try {
        const document = await installSampleDocument();
        await documentActions.applyNativeDocument(document, t.sampleOpened());
        await updateAppSetting(FIRST_RUN_SAMPLE_DOCUMENT_OPEN_ERROR_KEY, '').catch(() => undefined);
        await updateAppSetting(FIRST_RUN_SAMPLE_DOCUMENT_OPENED_KEY, true).catch(() => undefined);
      } catch (error) {
        const message = t.sampleOpenFailed({
          error: error instanceof Error ? error.message : String(error),
        });
        await updateAppSetting(FIRST_RUN_SAMPLE_DOCUMENT_OPEN_ERROR_KEY, message).catch(
          () => undefined,
        );
        statusMessage = message;
        showToast(message, 3500);
        return;
      }
      return;
    }

    if (shouldMarkFirstRunSampleHandled(state)) {
      await updateAppSetting(FIRST_RUN_SAMPLE_DOCUMENT_OPENED_KEY, true).catch(() => undefined);
    }
  }

  function scheduleStartupFolderLoad() {
    if (
      !desktopEnabled ||
      !startupFolderPath ||
      startupFolderLoadScheduled ||
      startupFolderLoadInProgress
    ) {
      return;
    }

    const folderPath = startupFolderPath;
    startupFolderPath = '';
    startupFolderLoadScheduled = true;

    const runStartupFolderLoad = () => {
      void (async () => {
        startupFolderLoadScheduled = false;
        startupFolderLoadInProgress = true;
        try {
          await loadFolder(folderPath);
          if (
            nativePath &&
            currentFolderPath &&
            sameFileSystemPath(currentFolderPath, folderPath)
          ) {
            await expandAncestors(nativePath, currentFolderPath);
          }
          await rememberNativeFolder(folderPath);
          await refreshRecentFiles();
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          statusMessage = `${t.loadFolderTreeFailed()}：${message}`;
        } finally {
          startupFolderLoadInProgress = false;
        }
      })();
    };

    if (typeof queueMicrotask === 'function') {
      queueMicrotask(runStartupFolderLoad);
      return;
    }

    window.setTimeout(runStartupFolderLoad, 0);
  }

  async function restoreWindowWorkspaceState(
    settings: Awaited<ReturnType<typeof listAppSettings>>,
  ) {
    if (!desktopEnabled || !windowLabel) return;
    const workspaceTabsKey = windowLabel ? `workspaceTabs:${windowLabel}` : 'workspaceTabs';
    const workspaceTabsSetting =
      settings.find((s) => s.key === workspaceTabsKey) ??
      settings.find((s) => s.key === 'workspaceTabs');
    if (!workspaceTabsSetting) return;
    const result = await migrateWorkspaceSetting(workspaceTabsSetting).catch(() => null);
    if (!result) return;
    if (result.migrated) {
      await updateAppSetting(workspaceTabsKey, result.state).catch(() => undefined);
    }
    await restorePersistedWorkspaceState(result.state);
  }

  onMount(async () => {
    appearanceRuntimeActive = true;
    appBootState = 'restoring-workspace';
    try {
      desktopEnabled = isTauriRuntime();
      window.addEventListener('wheel', handleGlobalWheel, { capture: true, passive: false });
      let persistedEditorMode: EditorViewMode | null = null;
      let settings: Awaited<ReturnType<typeof listAppSettings>> = [];
      let restoredWorkspaceTabs = false;
      let hasPendingFolder = false;

      if (desktopEnabled) {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        windowLabel = getCurrentWindow().label;
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('refresh_window_menu').catch(() => undefined);
        await setupCriticalDesktopEvents();
        void invoke<{ shouldPrompt: boolean }>('get_legacy_installer_notice')
          .then((notice) => {
            legacyInstallerPromptOpen = notice.shouldPrompt;
          })
          .catch(() => undefined);

        settings = await listAppSettings().catch(() => []);
        const dismissedUpdateSetting = settings.find(
          (setting) => setting.key === 'softwareUpdateDismissedVersion',
        );
        if (dismissedUpdateSetting) {
          try {
            const value = JSON.parse(dismissedUpdateSetting.valueJson);
            softwareUpdateDismissedVersion = typeof value === 'string' ? value : '';
          } catch {
            softwareUpdateDismissedVersion = '';
          }
        }
        unsubscribeSoftwareUpdate = softwareUpdateState.subscribe((state) => {
          softwareUpdateSnapshot = state;
          handleSoftwareUpdateSnapshot(state);
        });
        await initializeSoftwareUpdateCoordinator().catch(() => undefined);
        await loadReadingPositions();

        // 先读取待处理的外部打开路径和文件夹，再决定如何恢复工作区
        const pendingExternalOpenSetting = settings.find(
          (s) => s.key === `pendingExternalOpen:${windowLabel}`,
        );
        if (pendingExternalOpenSetting) {
          try {
            const paths = JSON.parse(pendingExternalOpenSetting.valueJson);
            if (Array.isArray(paths)) {
              const validPaths = paths.filter((path): path is string => typeof path === 'string');
              logInfo('ExternalOpen', '读取冷启动待打开文件', { paths: validPaths });
              queuePendingExternalOpenPaths(validPaths);
            }
            await updateAppSetting(`pendingExternalOpen:${windowLabel}`, '').catch(() => undefined);
          } catch {
            // ignore
          }
        }

        let pendingFolderPath = '';
        const pendingFolderSetting = settings.find((s) => s.key === `pendingFolder:${windowLabel}`);
        if (pendingFolderSetting) {
          try {
            const folderPath = JSON.parse(pendingFolderSetting.valueJson);
            if (folderPath && typeof folderPath === 'string' && folderPath.length > 0) {
              pendingFolderPath = folderPath;
              hasPendingFolder = true;
              await updateAppSetting(`pendingFolder:${windowLabel}`, '').catch(() => undefined);
            }
          } catch {
            // ignore
          }
        }

        if (pendingExternalOpenPaths.length > 0) {
          // 双击 md 文件启动：不恢复上次工作区，稍后单独加载文件所在目录并打开该文件
          restoredWorkspaceTabs = false;
        } else {
          await restoreWindowWorkspaceState(settings);
          restoredWorkspaceTabs = tabs.length > 0;

          // 若待打开文件夹与恢复的工作区不同，先清除旧标签页
          if (
            pendingFolderPath &&
            currentFolderPath &&
            !sameFileSystemPath(currentFolderPath, pendingFolderPath)
          ) {
            try {
              // 恢复可能仍在后台打开非活动分段标签；先收口并关闭全部 Rust session，
              // 再清空前端标签，避免 by_path 留下不可达的会话所有权。
              await cancelDeferredWorkspaceRestore();
              await closeAllSegmentedSessions(false);
              clearAllTabsWithoutCreatingBlank();
              restoredWorkspaceTabs = false;
            } catch (error) {
              showVisibleError(error, t.saveFileFailed());
              pendingFolderPath = '';
              hasPendingFolder = false;
            }
          }
          if (pendingFolderPath) {
            currentFolderPath = pendingFolderPath;
            startupFolderPath = pendingFolderPath;
          }
        }

        const appPreferences = await loadAppPreferences(desktopEnabled, settings);
        await applyAppPreferences(appPreferences, {
          applyEditorMode: false,
          writeBootSnapshot: true,
        });
        persistedEditorMode = appPreferences.editorMode;
      }

      setupSystemThemeListener();
      if (themeMode === 'system') {
        await syncSystemThemeFromDesktop({ writeBootSnapshot: true });
      }

      if (persistedEditorMode && !largeDocumentMode) {
        preferredEditorMode = persistedEditorMode;
        mode = persistedEditorMode;
        editor.updateOptions({ mode: getCoreModeForView(persistedEditorMode) });
      }
      await setupDesktopEvents();
      await refreshRecentFiles();
      const startupExternalOpenPaths = pendingExternalOpenPaths;
      pendingExternalOpenPaths = [];
      logInfo('ExternalOpen', '处理冷启动文件队列', { paths: startupExternalOpenPaths });
      if (startupExternalOpenPaths.length === 0) {
        await maybeOpenFirstRunSample({
          settings,
          recentFilesCount: recentFiles.length,
          restoredWorkspaceTabs,
          hasPendingFolder,
        });
      }
      if (startupExternalOpenPaths.length > 0) {
        appBootState = 'opening-file';
        await openStartupExternalMarkdownPaths(startupExternalOpenPaths);
      } else {
        scheduleStartupFolderLoad();
      }
      window.addEventListener('keydown', handleGlobalShortcut);
      fileCheckTimer = window.setInterval(() => {
        void checkExternalFileChange();
        void syncLoadedExplorerFolders();
      }, 5000);
      await tick();
      syncSourceTextareaHeight();
      await updateWindowTitle().catch(() => undefined);

      while (pendingExternalOpenPaths.length > 0) {
        const deferredExternalOpenPaths = pendingExternalOpenPaths;
        pendingExternalOpenPaths = [];
        await openExternalMarkdownPaths(deferredExternalOpenPaths);
      }
      appBootState = 'ready';
    } finally {
      if (appearanceRuntimeActive && !systemThemeListenerReady) {
        setupSystemThemeListener();
        if (themeMode === 'system') {
          void syncSystemThemeFromDesktop({ writeBootSnapshot: true });
        }
      }
      appBootState = 'ready';
      scheduleStartupSoftwareUpdateCheck();
    }
  });

  onDestroy(() => {
    resolveOpenTargetChoice(null);
    appearanceRuntimeActive = false;
    cancelPendingReadingPositionRestore();
    clearReadingPositionSaveTimer();
    appearanceApplyRequestId += 1;
    systemThemeListenerReady = false;
    // 组件销毁前立即持久化工作区状态和阅读位置
    void flushAllSegmentedSessions().catch(() => undefined);
    void flushPersistWorkspaceState();
    saveCurrentReadingPositionToMemoryOnly();
    void flushReadingPositions();
    _unsubConfirmStore();
    unsubscribeSoftwareUpdate?.();
    void disposeSoftwareUpdateCoordinator();
    for (const unlisten of desktopUnlisteners) unlisten();
    if (fileCheckTimer !== null) window.clearInterval(fileCheckTimer);
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    if (linkOpeningTimer !== null) window.clearTimeout(linkOpeningTimer);
    if (softwareUpdateStartupTimer !== null) window.clearTimeout(softwareUpdateStartupTimer);
    clearSplitSemanticRefreshTimer();
    clearContentAnalysisTimer();
    clearSearchDebounceTimer();
    window.removeEventListener('keydown', handleGlobalShortcut);
    window.removeEventListener('wheel', handleGlobalWheel, { capture: true });
    detachMountedEditorHostEvents();
    stopSystemThemeSync();
    sidebarResize.destroy();
    unsubscribe();
    editor.destroy();
    markdownLintController.destroy();
  });

  function syncFromEditor(event: EditorChangeEvent) {
    if (isSwitchingTab) return;

    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isMarkdownTab(activeTab)) {
      // 隐藏的 Markdown EditorCore 仍可能在主题或布局更新时发事件；分段标签必须彻底忽略。
      return;
    }
    const markdownChanged = event.markdown !== markdown;
    if (markdownChanged) {
      selectedStats = null;
    }

    // 预览标签页开始编辑 → 自动固定
    if (markdownChanged && previewTabId && previewTabId === activeTabId && event.dirty) {
      previewTabId = null;
    }

    const contentStable = event.reason !== 'content-pending';
    dirty = event.dirty;
    if (!dirty) {
      documentActions.cancelPendingAutoSave(activeTab.id);
    }
    if (!dirty && contentStable) {
      savedMarkdown = event.markdown;
    }
    version = event.version;
    pendingInlineMarks = event.pendingInlineMarks;

    activeTab.dirty = dirty;
    activeTab.version = version;
    if (!dirty && contentStable) {
      activeTab.savedMarkdown = event.markdown;
    }

    if (!markdownChanged) {
      tabs = [...tabs];
      persistWorkspaceState();
      return;
    }

    markdown = event.markdown;
    if (event.reason === 'source-input') {
      scheduleMarkdownAnalysis(event.markdown);
    } else {
      applyMarkdownAnalysis(event.markdown);
    }

    activeTab.markdown = markdown;
    if (!dirty && contentStable) {
      activeTab.savedMarkdown = markdown;
    }
    tabs = [...tabs];
    schedulePersistWorkspaceDrafts();
    persistWorkspaceState();

    if (autoSaveEnabled && desktopEnabled && dirty && nativePath) {
      documentActions.debouncedAutoSave(activeTab.id);
    }

    if (event.reason === 'source-input') {
      return;
    }

    if (event.markdown.length > largeDocumentLimit) {
      syncSourceTextareaHeight();
      return;
    }
    syncSourceTextareaHeight();
  }

  function handleSegmentedStateChange(event: CustomEvent<SegmentedEditorMetadata>) {
    const next = event.detail;
    const targetTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isSegmentedTextTab(targetTab) || targetTab.sessionId !== next.sessionId) return;

    targetTab.revision = next.revision;
    targetTab.persistedRevision = next.persistedRevision;
    targetTab.dirty = next.dirty;
    targetTab.indexProgress = next.indexProgress;
    targetTab.diskReadonly = next.filesystemReadonly;
    targetTab.selection = next.selection;
    targetTab.scrollAnchor = next.scrollAnchor;
    if (targetTab.id === previewTabId && next.dirty) {
      previewTabId = null;
    }
    if (targetTab.id === activeTabId) {
      dirty = next.dirty;
      diskReadonly = next.filesystemReadonly;
    }
    tabs = [...tabs];
    persistWorkspaceState();
    void updateWindowTitle();
  }

  function handleSegmentedStatus(event: CustomEvent<{ message: string }>) {
    statusMessage = event.detail.message;
  }

  function applyMarkdownAnalysis(markdownToAnalyze: string) {
    clearContentAnalysisTimer();
    const analysis = analyzeMarkdown(markdownToAnalyze);
    outline = analysis.outline;
    if (!outline.some((item) => item.id === activeOutlineId))
      activeOutlineId = outline[0]?.id ?? '';
    pruneCollapsedOutlineIds();
    stats = analysis.stats;
  }

  function scheduleMarkdownAnalysis(markdownToAnalyze: string) {
    clearContentAnalysisTimer();
    contentAnalysisTimer = window.setTimeout(() => {
      contentAnalysisTimer = null;
      if (markdownToAnalyze === markdown) {
        applyMarkdownAnalysis(markdownToAnalyze);
      }
    }, CONTENT_ANALYSIS_DEBOUNCE_MS);
  }

  function clearContentAnalysisTimer() {
    if (contentAnalysisTimer !== null) {
      window.clearTimeout(contentAnalysisTimer);
      contentAnalysisTimer = null;
    }
  }

  function openTablePicker() {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      return;
    }
    tablePickerOpen = true;
  }

  function closeTablePicker() {
    tablePickerOpen = false;
  }

  function openLinkPicker() {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      // 分段文档不读取隐藏的 Markdown EditorCore 选区或链接状态。
      return;
    }
    if (readonlyDocumentMode) {
      statusMessage = t.readonlyCannotEditLink();
      return;
    }
    if (getActiveEditorMode() !== 'semantic') {
      statusMessage = t.switchSemanticBeforeEditLink();
      return;
    }

    const activeLink = editor.getActiveLink();
    linkText = activeLink?.text ?? '';
    linkHref = activeLink?.href ?? '';
    linkCanRemove = Boolean(activeLink?.active);
    linkDraftTitle = activeLink?.title ?? null;
    linkPickerPositionStyle = getLinkPickerPositionStyle(editor.getSelectionAnchorRect());
    linkError = '';
    tablePickerOpen = false;
    linkPickerOpen = true;
  }

  function openLinkFromEditor(href: string) {
    const token = ++linkOpeningToken;
    linkOpening = true;
    statusMessage = t.openingLink();
    if (linkOpeningTimer !== null) window.clearTimeout(linkOpeningTimer);

    const minimumVisibleTime = new Promise<void>((resolve) => {
      linkOpeningTimer = window.setTimeout(resolve, 700);
    });

    Promise.all([
      navigateEditorLink(href).catch((error) => {
        statusMessage = getEditorLinkErrorMessage(error);
      }),
      minimumVisibleTime,
    ]).finally(() => {
      if (token === linkOpeningToken) {
        linkOpening = false;
        linkOpeningTimer = null;
      }
    });
  }

  async function navigateEditorLink(href: string) {
    const trimmedHref = href.trim();
    const isExternalOrAnchor =
      /^(?:https?|mailto):/i.test(trimmedHref) || trimmedHref.startsWith('#');
    if (!desktopEnabled && !isExternalOrAnchor) {
      throw new Error(t.localLinkDesktopRequired());
    }

    const target = await resolveEditorLink(trimmedHref, nativePath);
    if (target.kind === 'external') {
      await openExternalLink(target.href);
      return;
    }
    if (target.kind === 'anchor') {
      jumpToLinkFragment(target.fragment);
      return;
    }
    if (target.kind === 'attachment') {
      await openLocalAttachment(target.path);
      statusMessage = target.fragment
        ? t.localAttachmentFragmentIgnored()
        : t.localAttachmentOpened();
      return;
    }

    const opened = await openDocumentPath(target.path, {
      message: t.localLinkedDocumentOpened(),
      fallbackMessage: t.localLinkedDocumentOpenFailed({ path: target.path }),
    });
    if (!opened || !target.fragment) return;

    if (target.documentKind !== 'markdown') {
      statusMessage = t.localDocumentFragmentIgnored();
      return;
    }

    await tick();
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (
      !isMarkdownTab(activeTab) ||
      !activeTab.nativePath ||
      !sameNativePath(activeTab.nativePath, target.path)
    ) {
      return;
    }
    jumpToLinkFragment(target.fragment);
  }

  function jumpToLinkFragment(fragment: string) {
    const fragmentId = fragment.trim().toLowerCase();
    const item = outline.find((candidate) => candidate.id.toLowerCase() === fragmentId);
    if (!item) {
      statusMessage = t.localLinkAnchorMissing({ anchor: fragment });
      return;
    }

    outlineInteraction.jumpToOutlineItem(item);
    statusMessage = t.localLinkAnchorOpened({ anchor: fragment });
  }

  function getEditorLinkErrorMessage(error: unknown) {
    if (!(error instanceof EditorLinkResolutionError)) {
      return t.openLinkFailed({ error });
    }

    switch (error.code) {
      case 'invalid-encoding':
        return t.localLinkInvalidEncoding();
      case 'unsupported-protocol':
        return t.localLinkUnsupportedProtocol();
      case 'file-uri-unsupported':
        return t.localLinkFileUriUnsupported();
      case 'query-unsupported':
        return t.localLinkQueryUnsupported();
      case 'unc-unsupported':
        return t.localLinkUncUnsupported();
      case 'save-document-first':
        return t.localLinkSaveFirst();
      case 'unsupported-local-type':
        return t.localLinkUnsupportedType();
      case 'empty-fragment':
        return t.localLinkEmptyAnchor();
    }
  }

  function closeLinkPicker() {
    linkPickerOpen = false;
    linkText = '';
    linkError = '';
    linkCanRemove = false;
  }

  function updateLinkText(event: Event) {
    linkText = (event.currentTarget as HTMLInputElement).value;
  }

  function updateLinkHref(event: Event) {
    linkHref = (event.currentTarget as HTMLInputElement).value;
    if (linkError) linkError = '';
  }

  function applyLink() {
    if (!linkHref.trim()) {
      linkError = t.linkHrefRequired();
      return;
    }

    const applied = editor.execute({
      type: 'insertLink',
      href: linkHref,
      title: linkDraftTitle ?? undefined,
      text: linkText,
    });
    if (!applied) {
      linkError = t.linkHrefInvalid();
      return;
    }

    closeLinkPicker();
    editor.focus();
  }

  function getLinkPickerPositionStyle(anchorRect: EditorAnchorRect | null) {
    if (typeof window === 'undefined') return '';

    const popoverWidth = 392;
    const popoverHeight = 118;
    const viewportGap = 12;
    const fallbackLeft = window.innerWidth / 2 - popoverWidth / 2;
    const fallbackTop = 116;
    const baseLeft = anchorRect?.left ?? fallbackLeft;
    const baseTop = anchorRect ? anchorRect.bottom + 10 : fallbackTop;
    const maxLeft = Math.max(viewportGap, window.innerWidth - popoverWidth - viewportGap);
    const left = Math.min(Math.max(baseLeft, viewportGap), maxLeft);
    const top =
      baseTop + popoverHeight > window.innerHeight
        ? Math.max(viewportGap, (anchorRect?.top ?? fallbackTop) - popoverHeight - 10)
        : baseTop;

    return `left: ${Math.round(left)}px; top: ${Math.round(top)}px;`;
  }

  function removeLink() {
    const removed = editor.execute({ type: 'removeLink' });
    if (!removed) {
      linkError = t.linkNothingToRemove();
      return;
    }

    closeLinkPicker();
    editor.focus();
  }

  function insertTableWithSize(rows: number, columns: number) {
    runCommand({ type: 'insertTable', rows, columns });
    closeTablePicker();
  }

  function editFrontMatter() {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      // TXT/JSON 不能通过标题栏或原生菜单进入 Markdown Front Matter 链路。
      return;
    }
    if (readonlyDocumentMode) {
      statusMessage = t.readonlyCannotEditMetadata();
      return;
    }
    const hasFrontMatter = Boolean(extractFrontMatterBlock(editor.getMarkdown()));
    if (!hasFrontMatter) {
      editor.execute({ type: 'insertFrontMatter' });
      frontMatterFocusTarget = 'title-value';
      frontMatterFocusRequest += 1;
    } else {
      frontMatterFocusTarget = 'default';
    }
    frontMatterEditing = true;
  }

  function enterFrontMatterEdit() {
    if (readonlyDocumentMode) {
      statusMessage = t.readonlyCannotEditMetadata();
      return;
    }
    frontMatterFocusTarget = 'default';
    frontMatterEditing = true;
  }

  function leaveFrontMatterEdit() {
    frontMatterEditing = false;
  }

  function updateFrontMatterContent(content: string) {
    if (readonlyDocumentMode) {
      return;
    }
    editor.setMarkdown(replaceFrontMatterContent(editor.getMarkdown(), content));
  }

  function deleteFrontMatter() {
    if (readonlyDocumentMode) {
      statusMessage = t.readonlyCannotDeleteMetadata();
      return;
    }
    frontMatterEditing = false;
    editor.setMarkdown(removeFrontMatter(editor.getMarkdown()));
  }

  function showToast(message: string, durationMs = 1500) {
    if (toastTimer !== null) {
      window.clearTimeout(toastTimer);
    }
    toastMessage = message;
    toastTimer = window.setTimeout(() => {
      toastMessage = '';
      toastTimer = null;
    }, durationMs);
  }

  function showUnavailableFeature(featureName: string) {
    showToast(t.featureComingSoon({ featureName }));
  }

  async function handleExport(format: 'html' | 'pdf') {
    if (isSegmentedTextTab(tabs.find((tab) => tab.id === activeTabId))) {
      // TXT/JSON 不进入 Markdown HTML/PDF 导出链路。
      return;
    }
    if (!nativePath && !markdown.trim()) {
      showToast(t.noOpenDocumentForExport(), 2000);
      return;
    }

    const { exportHtml, exportPdf } = await import('./services/exportService');
    const renderedHtml = editorHost?.innerHTML ?? '';
    const suggestedFileName = fileName.replace(/\.(md|markdown|txt)$/i, '') || 'Untitled';

    const result =
      format === 'html'
        ? await exportHtml({
            markdown,
            renderedHtml,
            documentPath: nativePath,
            suggestedFileName,
            title: fileName || 'Untitled',
          })
        : await exportPdf({
            markdown,
            renderedHtml,
            documentPath: nativePath,
            suggestedFileName,
            title: fileName || 'Untitled',
          });

    if (result.cancelled) {
      return;
    }

    if (result.success) {
      const message =
        format === 'html'
          ? t.exportHtmlSuccess({ path: result.filePath! })
          : t.exportPdfSuccess({ path: result.filePath! });
      const warningText = result.warnings?.filter(Boolean).join('；');
      showToast(warningText ? `${message}；${warningText}` : message, warningText ? 5000 : 2500);
    } else {
      showToast(result.error ?? t.exportFailed(), 3500);
    }
  }

  $: visibleOutlineIds = new Set(
    outline
      .filter((_item, index) => getOutlineItemVisible(outline, collapsedOutlineIds, index))
      .map((item) => item.id),
  );

  $: frontMatter = extractFrontMatterBlock(markdown);
  $: if (!frontMatter) {
    frontMatterEditing = false;
    frontMatterFocusTarget = 'default';
  }
  $: {
    const activeDocument = tabs.find((tab) => tab.id === activeTabId);
    if (isMarkdownTab(activeDocument)) {
      const signature = `${searchPanelOpen}|${mode}|${searchCaseSensitive}|${searchWholeWord}|${markdown}`;
      if (signature !== lastSearchSignature) {
        lastSearchSignature = signature;
        refreshSearchMatches({ preserveActive: true, selectActive: false });
      }
    }
  }

  async function setupCriticalDesktopEvents() {
    if (!desktopEnabled || criticalDesktopEventsReady) {
      return;
    }

    const { listen } = await import('@tauri-apps/api/event');
    const [
      exitRequestUnlisten,
      closeRequestUnlisten,
      markdownMiniReturnUnlisten,
      openDocumentUnlisten,
    ] = await Promise.all([
      listen('nomo://request-exit-app', () => {
        requestExitApp().catch(() => undefined);
      }).catch(() => null),
      listen<{ windowLabel?: string; window_label?: string }>(
        'nomo://request-close-window',
        (event) => {
          // 多窗口场景下过滤只响应当前窗口的关闭请求，避免所有窗口同时弹出确认
          const requestedWindowLabel = event.payload?.windowLabel ?? event.payload?.window_label;
          if (requestedWindowLabel !== windowLabel) return;
          closeCurrentWindow().catch(() => undefined);
        },
      ).catch(() => null),
      listen('nomo://markdown-mini-request-return', () => {
        void requestMarkdownMiniReturn();
      }).catch(() => null),
      listenDesktopOpenDocuments((paths, targetWindowLabel) => {
        logInfo('ExternalOpen', '收到原生文件打开事件', {
          paths,
          targetWindowLabel,
          appBootState,
        });
        if (targetWindowLabel && targetWindowLabel !== windowLabel) {
          return;
        }
        if (windowLabel) {
          updateAppSetting(`pendingExternalOpen:${windowLabel}`, '').catch(() => undefined);
        }
        if (appBootState !== 'ready') {
          queuePendingExternalOpenPaths(paths);
          return;
        }
        openExternalMarkdownPaths(paths).catch(() => undefined);
      }).catch(() => null),
    ]);

    criticalDesktopEventsReady = true;
    desktopUnlisteners = [
      ...desktopUnlisteners,
      exitRequestUnlisten,
      closeRequestUnlisten,
      markdownMiniReturnUnlisten,
      openDocumentUnlisten,
    ].filter((value): value is () => void => Boolean(value));
  }

  async function setupDesktopEvents() {
    if (!desktopEnabled) {
      return;
    }

    const { listen } = await import('@tauri-apps/api/event');
    const [
      menuUnlisten,
      dropUnlisten,
      settingsUnlisten,
      updateInstallRequestUnlisten,
      openFolderUnlisten,
    ] = await Promise.all([
      listenDesktopMenuCommands((command) => {
        executeDesktopCommand(command);
      }).catch(() => null),
      listenDesktopFileDrops((paths) => {
        openDroppedMarkdown(paths);
      }).catch(() => null),
      listen<SettingsUpdatedPayload>(SETTINGS_UPDATED_EVENT, (event) => {
        handleSettingsUpdated(event.payload).catch(() => undefined);
      }).catch(() => null),
      listen<{ requestId?: string }>('nomo://request-update-install', (event) => {
        const requestId = event.payload?.requestId;
        if (typeof requestId === 'string' && requestId.length > 0) {
          approveSoftwareUpdateInstall(requestId).catch(() => undefined);
        }
      }).catch(() => null),
      listenDesktopOpenFolder((folderPath, targetWindowLabel) => {
        if (targetWindowLabel && targetWindowLabel !== windowLabel) {
          return;
        }
        if (windowLabel) {
          updateAppSetting(`pendingFolder:${windowLabel}`, '').catch(() => undefined);
        }
        openFolderWithBehavior(folderPath).catch(() => undefined);
      }).catch(() => null),
    ]);

    desktopUnlisteners = [
      ...desktopUnlisteners,
      menuUnlisten,
      dropUnlisten,
      settingsUnlisten,
      updateInstallRequestUnlisten,
      openFolderUnlisten,
    ].filter((value): value is () => void => Boolean(value));
  }

  function queuePendingExternalOpenPaths(paths: string[]) {
    pendingExternalOpenPaths = [...new Set([...pendingExternalOpenPaths, ...paths])];
  }

  async function openExternalMarkdownPaths(paths: string[]) {
    if (!desktopEnabled || paths.length === 0) {
      return;
    }

    const supportedPaths = paths.filter((path) => /\.(md|markdown|txt|json)$/i.test(path));
    if (supportedPaths.length > 0) {
      await openTargetWithBehavior({ kind: 'documents', paths: supportedPaths });
    }
  }

  // 双击 md 文件启动：不恢复上次工作区，只打开文件所在目录并打开该文件
  async function openStartupExternalMarkdownPaths(paths: string[]) {
    logInfo('ExternalOpen', '开始处理冷启动文件', { paths });
    if (!desktopEnabled || paths.length === 0) {
      return;
    }

    const supportedPaths = paths.filter((path) => /\.(md|markdown|txt|json)$/i.test(path));
    if (supportedPaths.length === 0) {
      return;
    }

    // 丢弃恢复的标签，避免上次工作区干扰
    clearAllTabsWithoutCreatingBlank({ skipPersist: true });

    const firstPath = supportedPaths[0];
    const parentDir = getDirectoryLabel(firstPath);

    if (parentDir && parentDir !== t.currentFolder()) {
      currentFolderPath = parentDir;
      await loadFolder(parentDir).catch(() => undefined);
      await rememberNativeFolder(parentDir).catch(() => undefined);
    }

    for (const path of supportedPaths) {
      await openFilePathInCurrentWindow(path);
    }
  }

  function executeDesktopCommand(command: string) {
    executeDesktopAppCommand(command, commandHandlers);
  }

  function handleGlobalShortcut(event: KeyboardEvent) {
    handleGlobalAppShortcut(event, commandHandlers, shortcutPreferences);
  }

  // 使用 capture 阶段在事件到达可滚动元素之前拦截，
  // 避免浏览器对可滚动元素的 wheel 事件强制 passive 导致 preventDefault 失效。
  function handleGlobalWheel(event: WheelEvent) {
    if (!ctrlWheelZoomEnabled || !event.ctrlKey) {
      return;
    }

    const target = event.target as HTMLElement | null;
    if (!target?.closest('.editor-grid')) {
      return;
    }

    event.preventDefault();
    const direction = event.deltaY < 0 ? 1 : -1;
    const nextZoom = Math.min(160, Math.max(80, zoomPercent + direction * 5));
    if (nextZoom === zoomPercent) {
      return;
    }
    const anchor = saveScrollAnchor(event.clientX, event.clientY);
    zoomPercent = nextZoom;
    applyZoomSetting(zoomPercent, {
      transition: true,
      onFrame: () => {
        syncZoomFrameViewportLayout(anchor?.pane);
        refreshEditorViewportLayout();
        restoreScrollAnchor(anchor);
      },
    });
    updateAppSetting('zoomPercent', zoomPercent).catch(() => undefined);
    statusMessage = t.zoomStatus({ percent: zoomPercent });
  }

  // 步骤1：校验目标缩放值并更新状态
  // 步骤2：保存滚动锚点，缩放后恢复阅读位置
  // 步骤3：应用缩放动画并持久化到设置
  function handleZoomChange(nextZoom: number) {
    const clamped = Math.min(160, Math.max(80, nextZoom));
    if (clamped === zoomPercent) {
      return;
    }
    const anchor = saveScrollAnchor();
    zoomPercent = clamped;
    applyZoomSetting(zoomPercent, {
      transition: true,
      onFrame: () => {
        syncZoomFrameViewportLayout(anchor?.pane);
        refreshEditorViewportLayout();
        restoreScrollAnchor(anchor);
      },
    });
    updateAppSetting('zoomPercent', zoomPercent).catch(() => undefined);
    statusMessage = t.zoomStatus({ percent: zoomPercent });
  }

  function syncZoomFrameViewportLayout(pane?: HTMLElement) {
    const visiblePane = pane ?? (getActiveEditorMode() === 'source' ? sourcePane : semanticPane);
    visiblePane?.dispatchEvent(new Event('nomo:editor-viewport-layout-refresh'));
  }

  // Ctrl+滚轮使用鼠标位置，状态栏缩放使用视口中心。
  // 记录指向元素内的相对位置，每帧用元素的新几何位置校正滚动，
  // 避免按整页 scrollHeight 比例缩放时视角逐步向下漂移。
  function saveScrollAnchor(clientX?: number, clientY?: number): ZoomScrollAnchor | null {
    // 双栏只重测并更新跟随栏，缩放动画不能恢复旧主栏位置、覆盖新的滚动意图。
    if (mode === 'split') return null;
    const pane = getActiveEditorMode() === 'source' ? sourcePane : semanticPane;
    if (!pane) return null;

    const paneRect = pane.getBoundingClientRect();
    const anchorX = Math.min(
      paneRect.right - 1,
      Math.max(paneRect.left, clientX ?? paneRect.left + paneRect.width / 2),
    );
    const anchorY = Math.min(
      paneRect.bottom - 1,
      Math.max(paneRect.top, clientY ?? paneRect.top + paneRect.height / 2),
    );
    const pointedElement = document.elementFromPoint(anchorX, anchorY);
    const editorSurface = pane.querySelector<HTMLElement>('.ProseMirror, .source-editor');
    let anchorElement =
      pointedElement instanceof HTMLElement && pane.contains(pointedElement)
        ? pointedElement
        : editorSurface;

    if (!anchorElement) return null;
    if (editorSurface && anchorElement !== editorSurface && editorSurface.contains(anchorElement)) {
      while (anchorElement.parentElement && anchorElement.parentElement !== editorSurface) {
        anchorElement = anchorElement.parentElement;
      }
    }

    const elementRect = anchorElement.getBoundingClientRect();
    const elementRatio =
      elementRect.height > 0
        ? Math.min(1, Math.max(0, (anchorY - elementRect.top) / elementRect.height))
        : 0;
    return { pane, element: anchorElement, elementRatio, clientY: anchorY };
  }

  function restoreScrollAnchor(anchor: ZoomScrollAnchor | null) {
    if (!anchor || !anchor.element.isConnected || !anchor.pane.contains(anchor.element)) return;
    const elementRect = anchor.element.getBoundingClientRect();
    const currentClientY = elementRect.top + elementRect.height * anchor.elementRatio;
    setScrollTop(anchor.pane, anchor.pane.scrollTop + currentClientY - anchor.clientY);
  }

  function setupSystemThemeListener() {
    if (!appearanceRuntimeActive) {
      return;
    }
    stopSystemThemeSync();
    stopSystemThemeSync = listenForSystemThemeChanges((systemScheme) => {
      if (!appearanceRuntimeActive || themeMode !== 'system') {
        return;
      }
      return syncSystemThemeFromDesktop({ transition: true, systemScheme });
    });
    systemThemeListenerReady = true;
  }

  async function loadFolder(folderPath: string) {
    await folderExplorer.loadFolder(folderPath);
  }

  function getSourceLineHeight() {
    return outlineInteraction.getSourceLineHeight();
  }

  function handleDeletedImageResources(event: EditorImageDeletionEvent) {
    const loader = getImageLoader();
    if (!imageSettings.autoDeleteUnusedLocalImages || !loader?.remove || event.srcs.length === 0) {
      return;
    }

    const context = getImageContext();
    Promise.allSettled(event.srcs.map((src) => loader.remove!(src, context))).then((results) => {
      const removed = results.filter(
        (result) => result.status === 'fulfilled' && result.value.removed,
      ).length;
      const failed = results.filter(
        (result) =>
          result.status === 'rejected' ||
          (result.status === 'fulfilled' && Boolean(result.value.error)),
      ).length;

      if (removed > 0 && failed > 0) {
        statusMessage = t.imageCleanupRemovedFailed({ removed, failed });
      } else if (removed > 0) {
        statusMessage = t.imageCleanupRemoved({ removed });
      } else if (failed > 0) {
        statusMessage = t.imageCleanupFailed({ failed });
      }
    });
  }

  function getImageContext(): ImageContext {
    const documentPath = nativePath ?? filePath;
    const documentDir = getParentPath(documentPath);
    return {
      documentPath,
      documentFileName: fileName,
      documentDir,
      assetsDirectory: documentDir ? joinPath(documentDir, 'assets') : undefined,
      settings: imageSettings,
    };
  }

  function getParentPath(path: string | null | undefined): string | undefined {
    if (!path) {
      return undefined;
    }
    const normalized = path.replace(/\\/g, '/');
    const index = normalized.lastIndexOf('/');
    if (index <= 0) {
      return undefined;
    }
    return path.slice(0, index);
  }

  function joinPath(parent: string, child: string): string {
    const separator = parent.includes('\\') ? '\\' : '/';
    return `${parent.replace(/[\\/]+$/, '')}${separator}${child}`;
  }

  function writeRecoveryDraft(reason: string) {
    const activeTab = tabs.find((tab) => tab.id === activeTabId);
    if (!isMarkdownTab(activeTab)) {
      return;
    }
    writeRecoveryDraftToStorage(RECOVERY_KEY, {
      reason,
      fileName,
      filePath,
      nativePath,
      markdown: editor.getMarkdown(),
    });
  }

  function createEmptyPendingInlineMarks(): InlinePendingMarks {
    return {
      strong: false,
      em: false,
      code: false,
      strikethrough: false,
      underline: false,
      highlight: false,
    };
  }
</script>

<svelte:head>
  <title>{t.appName()}</title>
</svelte:head>

<AppShell
  {interfaceLocale}
  {appBootState}
  bind:segmentedWorkspace
  bind:fileInput
  bind:sourcePane
  bind:semanticPane
  bind:sourceEditor
  bind:editorHost
  editorCore={editor}
  {focusMode}
  {toolbarHidden}
  toolbarShortcut={shortcutPreferences['toggle-toolbar']}
  markdownMiniShortcut={shortcutPreferences['toggle-markdown-mini']}
  {markdownMiniActive}
  {markdownMiniPinned}
  markdownMiniExternalChanged={externalFileChange.type !== 'none'}
  {isResizing}
  {contentWidthPercent}
  {theme}
  editorTheme={currentEditorTheme}
  {desktopEnabled}
  {activeMenu}
  {recentFiles}
  {missingRecentPaths}
  {mode}
  {splitViewLayout}
  {splitLeftPercent}
  {splitActivePane}
  {splitAlignmentGuideVisible}
  {outlineVisible}
  {currentFolderPath}
  {rootFolderExpanded}
  {folderTree}
  {expandedFolders}
  {nativePath}
  {dirty}
  {fileName}
  {filePath}
  {sidebarWidth}
  {tabs}
  {activeTabId}
  {previewTabId}
  {markdown}
  sourceDocumentId={activeTabId}
  {largeDocumentMode}
  {frontMatter}
  {frontMatterEditing}
  {frontMatterFocusRequest}
  {frontMatterFocusTarget}
  {readonlyDocumentMode}
  {outline}
  {activeOutlineId}
  {collapsedOutlineIds}
  {visibleOutlineIds}
  stats={effectiveStats}
  {writingStatsVisible}
  {writingStatsMetric}
  {readingTimeVisible}
  {markdownLintEnabled}
  {markdownLintRuleSet}
  {markdownLintState}
  {zoomPercent}
  {tablePickerOpen}
  {linkPickerOpen}
  {linkText}
  {linkHref}
  {linkError}
  {linkCanRemove}
  {linkPickerPositionStyle}
  {searchPanelOpen}
  {searchReplaceVisible}
  {searchQuery}
  {searchReplacement}
  {searchCaseSensitive}
  {searchWholeWord}
  {searchBackwards}
  {searchWrapAround}
  {searchActiveIndex}
  {searchMatchCount}
  {autoSaveEnabled}
  {autoSaveDelayMs}
  softwareUpdateState={softwareUpdateSnapshot}
  {openSoftwareUpdate}
  {getCompactPath}
  {getFolderName}
  {getDirectoryLabel}
  {toggleMenu}
  {closeMenu}
  {toggleTheme}
  {exitApp}
  {createNewWindow}
  {createNewFile}
  {openFileDialog}
  {openFolderDialog}
  {openRecentEntry}
  {openPreviewFile}
  pinPreviewFile={pinPreviewTab}
  {clearRecentEntriesList}
  {removeRecentEntry}
  {closeCurrentFile}
  {closeCurrentWindow}
  {saveMarkdownFile}
  {runCommand}
  {pendingInlineMarks}
  {openTablePicker}
  {openLinkPicker}
  {openSearchPanel}
  {closeSearchPanel}
  {updateSearchQuery}
  {updateSearchReplacement}
  {toggleSearchCaseSensitive}
  {toggleSearchWholeWord}
  {toggleSearchBackwards}
  {toggleSearchWrapAround}
  {toggleSearchReplaceVisible}
  {findPreviousSearchMatch}
  {findNextSearchMatch}
  {countSearchMatches}
  {replaceCurrentSearchMatch}
  {replaceAllSearchMatches}
  {editFrontMatter}
  {showUnavailableFeature}
  {closeTablePicker}
  {closeLinkPicker}
  {updateLinkText}
  {updateLinkHref}
  {applyLink}
  {removeLink}
  {insertTableWithSize}
  {setMode}
  {setSplitActivePane}
  {updateSplitLeftPercent}
  {toggleSplitAlignmentGuide}
  {toggleOutlineVisible}
  {toggleFocusMode}
  {toggleToolbar}
  {toggleMarkdownMini}
  {toggleMarkdownMiniPinned}
  {toggleRootFolder}
  {toggleFolderCollapse}
  {startResize}
  {switchTab}
  {closeTab}
  {pinPreviewTab}
  {updateContentWidth}
  {updateMarkdown}
  {enterFrontMatterEdit}
  {leaveFrontMatterEdit}
  {updateFrontMatterContent}
  {deleteFrontMatter}
  {updateActiveOutlineFromSourceScroll}
  {updateActiveOutlineFromSemanticScroll}
  onSourceScroll={handleSourceScroll}
  onSemanticScroll={handleSemanticScroll}
  {handleEditorPaste}
  {handleEditorDrop}
  {handleWorkspaceContextMenu}
  openContextMenu={openApplicationContextMenu}
  copyContextText={copyPlainText}
  {revealContextPath}
  {isOutlineItemExpandable}
  {toggleOutlineItemExpanded}
  {expandAllOutline}
  {collapseAllOutline}
  collapseOutlineToDefaultLevel={applyOutlineDefaultExpansion}
  {jumpToOutlineItem}
  {moveOutlineSection}
  {openMarkdownFile}
  {openSettings}
  {setWritingStatsMetric}
  onSourceSelectionChange={handleSourceSelectionChange}
  {retryMarkdownLint}
  onMarkdownLintIssueSelect={revealMarkdownLintIssue}
  onZoomChange={handleZoomChange}
  exportHtml={() => handleExport('html')}
  exportPdf={() => handleExport('pdf')}
  on:createNode={handleCreateNode}
  on:renameNode={handleRenameNode}
  on:refreshFolder={handleRefreshFolder}
  on:collapseAll={handleCollapseAll}
  on:closeOtherTabs={handleCloseOtherTabs}
  on:closeTabsToRight={handleCloseTabsToRight}
  on:closeAllTabs={handleCloseAllTabs}
  on:deleteNode={handleDeleteNode}
  on:revealError={(event) => showVisibleError(event.detail, t.openFolderFailed())}
  on:stateChange={handleSegmentedStateChange}
  on:status={handleSegmentedStatus}
/>

{#if softwareUpdateNoticeVisible}
  <SoftwareUpdateNotice
    version={softwareUpdateSnapshot.version ?? softwareUpdateSnapshot.candidate?.version ?? ''}
    summary={createSoftwareUpdateSummary(
      softwareUpdateSnapshot.body ?? softwareUpdateSnapshot.candidate?.body,
      t.softwareUpdateNoticeSummary(),
    )}
    onView={openSoftwareUpdate}
    onLater={hideSoftwareUpdateForLater}
    onDismiss={dismissSoftwareUpdateVersion}
    onAutoHide={() => {
      softwareUpdateNoticeVisible = false;
    }}
  />
{/if}

{#if softwareUpdateDialogOpen && (softwareUpdateSnapshot.candidate || softwareUpdateSnapshot.downloadedUpdate)}
  <SoftwareUpdateDialog
    state={softwareUpdateSnapshot}
    onClose={() => {
      softwareUpdateDialogOpen = false;
    }}
    onLater={hideSoftwareUpdateForLater}
    onDownload={downloadCurrentSoftwareUpdate}
    onInstall={installCurrentSoftwareUpdate}
    onRetry={retrySoftwareUpdateCheck}
  />
{/if}

<div class="app-toast" class:visible={toastMessage} role="status">{toastMessage}</div>

{#if linkOpening}
  <div class="link-opening-indicator" role="status" aria-live="polite">
    <span class="link-opening-spinner" aria-hidden="true"></span>
    <span>{t.openingLinkShort()}</span>
  </div>
{/if}

<FolderOpenDialog
  {interfaceLocale}
  open={pendingOpenChoice !== null}
  targetPath={getOpenTargetDialogPath(pendingOpenChoice)}
  targetName={getOpenTargetDialogName(pendingOpenChoice)}
  on:choose={handleOpenTargetChoice}
  on:cancel={() => resolveOpenTargetChoice(null)}
/>

{#if contextMenuOpen}
  {#key contextMenuVersion}
    <ContextMenu
      x={contextMenuX}
      y={contextMenuY}
      items={contextMenuItems}
      onClose={closeContextMenu}
    />
  {/key}
{/if}

<ConfirmDialog
  open={legacyInstallerPromptOpen}
  title={t.legacyInstallerTitle()}
  message={t.legacyInstallerMessage()}
  confirmLabel={t.legacyInstallerOpenApps()}
  danger={false}
  onConfirm={() => void openWindowsInstalledAppsForLegacyNomo()}
  onCancel={() => (legacyInstallerPromptOpen = false)}
/>

<ConfirmDialog
  open={deleteConfirmOpen}
  title={t.confirmDelete()}
  message={t.confirmDeleteMessage({
    type: deleteConfirmIsDir ? t.folder() : t.file(),
    name: deleteConfirmName,
  })}
  detail={deleteConfirmPath}
  confirmLabel={t.delete()}
  danger={true}
  onConfirm={executeDelete}
  onCancel={closeDeleteConfirm}
/>

<UnsavedConfirmDialog
  open={confirmDialogState.open}
  title={confirmDialogState.title}
  message={confirmDialogState.message}
  confirmLabel={confirmDialogState.confirmLabel}
  cancelLabel={confirmDialogState.cancelLabel}
  saveLabel={confirmDialogState.saveLabel}
  onConfirm={() => resolveConfirmDialog(true)}
  onCancel={() => dismissConfirmDialog()}
  onSave={() => resolveConfirmDialog('save')}
/>

<CloseWindowBehaviorDialog
  open={closeWindowChoiceDialogOpen}
  title={t.closeWindowChoiceTitle()}
  message={t.closeWindowChoiceMessage()}
  closeWindowLabel={t.closeWindowBehaviorCloseWindow()}
  closeToTrayLabel={t.closeWindowBehaviorCloseToTray()}
  rememberLabel={t.rememberCloseWindowChoice()}
  remember={rememberCloseWindowChoice}
  onRememberChange={(value) => (rememberCloseWindowChoice = value)}
  onChoose={resolveCloseWindowChoice}
  onCancel={cancelCloseWindowChoice}
/>

<UnsavedConfirmDialog
  open={startupDraftConflict !== null}
  title={t.startupDraftConflictTitle()}
  message={startupDraftConflict
    ? t.startupDraftConflictMessage({ fileName: startupDraftConflict.fileName })
    : ''}
  confirmLabel={t.startupDraftConflictRestoreDraft()}
  cancelLabel={t.startupDraftConflictUseDisk()}
  saveLabel={t.startupDraftConflictSaveDraftAs()}
  onConfirm={applyStartupConflictDraft}
  onCancel={applyStartupConflictDiskVersion}
  onSave={saveStartupConflictDraftAs}
/>

<ExternalChangeDialog
  open={externalChangeDialogOpen}
  change={externalChangeDialogState}
  onReload={handleExternalChangeReload}
  onOverwrite={handleExternalChangeOverwrite}
  onSaveAs={handleExternalChangeSaveAs}
  onDismiss={handleExternalChangeDismiss}
  onCloseDiscard={handleExternalChangeCloseDiscard}
/>
