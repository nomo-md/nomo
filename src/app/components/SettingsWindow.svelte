<script lang="ts">
  import {
    BarChart3,
    BookOpenText,
    Code2,
    FileImage,
    FolderOpen,
    Info,
    Palette,
    Settings2,
    SlidersHorizontal,
    X,
  } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { DIAGRAM_TEMPLATES } from '../../lib/editor-core/diagramTemplates';
  import { isTauriRuntime, openExternalLink } from '../../lib/desktop/tauriStorage';
  import {
    type DownloadedSoftwareUpdate,
    type SoftwareUpdateCandidate,
    type SoftwareUpdateSnapshot,
    type SoftwareUpdateUiState,
  } from '../../lib/desktop/tauriUpdater';
  import packageInfo from '../../../package.json';
  import {
    DEFAULT_APP_PREFERENCES,
    SETTINGS_UPDATED_EVENT,
    applyEditorLayoutSettings,
    applyTypographySettings,
    loadAppPreferences,
    normalizeAppPreferences,
    saveAppPreferences,
    type AppPreferences,
    type AppPreferenceKey,
    type AppPreferencesPatch,
    type CloseWindowBehavior,
    type CodeBlockIndentPreference,
    type EditorModePreference,
    type ExternalFileChangeBehavior,
    type OpenDefaultBehavior,
    type InterfaceLanguagePreference,
    type MarkdownLintRuleSet,
    type RenderModePreference,
    type SplitViewLayoutPreference,
    type ShortcutCommandId,
    type WritingStatsMetric,
  } from '../services/settings';
  import { suppressUnhandledContextMenu } from '../services/contextMenuPolicy';
  import {
    applyThemeRuntime,
    listenForSystemThemeChanges,
    readEffectiveSystemScheme,
    resolveTheme,
    writeThemeBootSnapshot,
  } from '../services/themeManager';
  import {
    CLASSIC_DOCUMENT_STYLE_ID,
    DEFAULT_DOCUMENT_STYLE_ID,
    getThemeDisplayName,
    themeRegistry,
  } from '../services/themeRegistry';
  import type { ColorScheme, ThemeMode } from '../../lib/theme/types';
  import { createPerfTimer, logToTerminal } from '../../lib/services/logger';
  import {
    INTERFACE_LANGUAGE_OPTIONS,
    applyInterfaceLanguagePreference,
    getDiagramTypeLabel,
    t,
    type EffectiveInterfaceLocale,
  } from '../i18n';
  import type {
    ImageDefaultAlign,
    ImageInsertStrategy,
    ImageUploadProvider,
  } from '../../lib/services/render';
  import { getPlatformCapabilities, HOMEBREW_SETUP_COMMAND } from '../services/platform';
  import {
    disposeSoftwareUpdateCoordinator,
    initializeSoftwareUpdateCoordinator,
    runSoftwareUpdateCheck,
    softwareUpdateState,
    startSoftwareUpdateDownload,
    startSoftwareUpdateInstall,
  } from '../services/softwareUpdate';
  import SoftwareUpdateDialog from './SoftwareUpdateDialog.svelte';
  import WindowsCaptionControls from './WindowsCaptionControls.svelte';
  import nomoLogoDark from '../../../src-tauri/icons/nomo/source/nomo-app-dark-128.png?url';
  import nomoLogoLight from '../../../src-tauri/icons/nomo/source/nomo-app-light-128.png?url';

  const GITHUB_REPOSITORY_URL = 'https://github.com/nomo-md/nomo';
  const GITHUB_ISSUE_URL = 'https://github.com/nomo-md/nomo/issues/new/choose';

  type CategoryId =
    | 'general'
    | 'editor'
    | 'appearance'
    | 'files'
    | 'images'
    | 'stats'
    | 'advanced'
    | 'about';

  type MarkdownAssociationStatus = {
    supported: boolean;
    registered: boolean;
    is_default: boolean;
    default_prog_id: string | null;
    managedByPackage: boolean;
    message: string;
  };

  type WindowsContextMenuStatus = {
    supported: boolean;
    registered: boolean;
    enabled: boolean;
    managedByPackage: boolean;
    message: string;
  };

  const categories = [
    { id: 'general' as const, labelKey: 'settingsCategoryGeneral', icon: Settings2 },
    { id: 'editor' as const, labelKey: 'settingsCategoryEditor', icon: BookOpenText },
    { id: 'appearance' as const, labelKey: 'settingsCategoryAppearance', icon: Palette },
    { id: 'files' as const, labelKey: 'settingsCategoryFiles', icon: FolderOpen },
    { id: 'images' as const, labelKey: 'settingsCategoryImages', icon: FileImage },
    { id: 'stats' as const, labelKey: 'settingsCategoryStats', icon: BarChart3 },
    { id: 'advanced' as const, labelKey: 'settingsCategoryAdvanced', icon: SlidersHorizontal },
    { id: 'about' as const, labelKey: 'settingsCategoryAbout', icon: Info },
  ];

  let activeCategory: CategoryId = 'general';
  let draftSettings: AppPreferences = { ...DEFAULT_APP_PREFERENCES };
  let lastPersistedSettings: AppPreferences = { ...DEFAULT_APP_PREFERENCES };
  let effectiveSystemScheme: ColorScheme =
    document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light';
  const availableThemes = themeRegistry.listThemes();
  let interfaceLocale: EffectiveInterfaceLocale = applyInterfaceLanguagePreference(
    draftSettings.interfaceLanguage,
  );
  $: categoryTitles = createCategoryTitles(interfaceLocale);
  let loaded = false;
  let statusMessage = '';
  let statusTimer: number | null = null;
  let autoSaveTimer: number | null = null;
  let saveInFlight = false;
  let saveQueued = false;
  let activeSavePromise: Promise<void> | null = null;
  let dirtyPreferenceKeys = new Set<AppPreferenceKey>();
  let desktopEnabled = false;
  let platformCapabilities = getPlatformCapabilities();
  let picgoTesting = false;
  let bindingMdAssociation = false;
  let unbindingMdAssociation = false;
  let checkingMdAssociation = false;
  let mdAssociationStatus: MarkdownAssociationStatus | null = null;
  let mdAssociationError = '';
  let filesIntegrationStatusRequested = false;
  let registeringContextMenu = false;
  let unregisteringContextMenu = false;
  let checkingContextMenu = false;
  let contextMenuStatus: WindowsContextMenuStatus | null = null;
  let contextMenuError = '';
  let focusDebounceTimer: number | null = null;
  let downloadedSoftwareUpdate: DownloadedSoftwareUpdate | null = null;
  let softwareUpdateSnapshot: SoftwareUpdateSnapshot = {
    status: 'idle',
    currentVersion: packageInfo.version,
    installationKind: 'unsupported',
  };
  let softwareUpdateDialogOpen = false;
  let updateState: SoftwareUpdateUiState = {
    status: 'idle',
    message: '',
  };
  let updateDecisionUnlisten: (() => void) | null = null;
  let settingsCloseUnlisten: (() => void) | null = null;
  let settingsCloseListenerCancelled = false;
  let systemThemeSyncActive = false;
  let closeInProgress = false;
  let emitDesktopEvent:
    | ((event: string, payload: unknown) => Promise<void>)
    | null = null;

  // 响应式派生：强制 Svelte 追踪 updateState 的变化
  $: updateStatus = updateState.status;
  $: updateBusy =
    updateStatus === 'checking' || updateStatus === 'downloading' || updateStatus === 'installing';

  function createCategoryTitles(_locale: EffectiveInterfaceLocale): Record<CategoryId, string> {
    return {
      general: t.settingsCategoryGeneral(),
      editor: t.settingsCategoryEditor(),
      appearance: t.settingsCategoryAppearance(),
      files: t.settingsCategoryFiles(),
      images: t.settingsCategoryImages(),
      stats: t.settingsCategoryStats(),
      advanced: t.settingsCategoryAdvanced(),
      about: t.settingsCategoryAboutTitle(),
    };
  }

  const shortcutItems: Array<{ id: ShortcutCommandId; labelKey: string }> = [
    { id: 'new-file', labelKey: 'newMarkdown' },
    { id: 'open-file', labelKey: 'openFile' },
    { id: 'save-file', labelKey: 'save' },
    { id: 'toggle-source', labelKey: 'toggleSourceMode' },
    { id: 'toggle-theme', labelKey: 'toggleThemeLightDark' },
    { id: 'toggle-focus', labelKey: 'showHideExplorer' },
    { id: 'toggle-toolbar', labelKey: 'showHideToolbar' },
    { id: 'toggle-markdown-mini', labelKey: 'markdownMiniShortcut' },
    { id: 'insert-code-block', labelKey: 'insertCodeBlock' },
    { id: 'insert-table', labelKey: 'insertTable' },
    { id: 'insert-math-block', labelKey: 'insertMathBlock' },
    { id: 'menu-link', labelKey: 'editLink' },
    { id: 'menu-clear-format', labelKey: 'clearStyle' },
  ];

  onMount(() => {
    logToTerminal('info', 'SettingsWindow', '设置窗口打开');
    desktopEnabled = isTauriRuntime();
    platformCapabilities = getPlatformCapabilities();
    settingsCloseListenerCancelled = false;
    systemThemeSyncActive = true;
    if (desktopEnabled) {
      void preloadDesktopEventEmit();
      void initializeDesktopSettingsWindow();
    } else {
      void loadPreferences();
    }
    const unsubscribeSoftwareUpdate = softwareUpdateState.subscribe(applySharedSoftwareUpdateState);
    void initializeSoftwareUpdateCoordinator();
    window.addEventListener('focus', handleWindowFocus);
    const stopSystemThemeSync = listenForSystemThemeChanges(syncSystemTheme);

    return () => {
      logToTerminal('info', 'SettingsWindow', '设置窗口关闭（组件卸载）');
      if (statusTimer !== null) {
        window.clearTimeout(statusTimer);
      }
      if (autoSaveTimer !== null) {
        window.clearTimeout(autoSaveTimer);
      }
      if (focusDebounceTimer !== null) {
        window.clearTimeout(focusDebounceTimer);
      }
      if (updateDecisionUnlisten) {
        updateDecisionUnlisten();
      }
      settingsCloseListenerCancelled = true;
      systemThemeSyncActive = false;
      settingsCloseUnlisten?.();
      settingsCloseUnlisten = null;
      unsubscribeSoftwareUpdate();
      stopSystemThemeSync();
      void disposeSoftwareUpdateCoordinator();
      window.removeEventListener('focus', handleWindowFocus);
    };
  });

  async function initializeDesktopSettingsWindow() {
    await installSettingsCloseListener();
    if (!settingsCloseListenerCancelled) {
      await loadPreferences();
    }
  }

  async function preloadDesktopEventEmit() {
    if (!desktopEnabled || emitDesktopEvent) {
      return;
    }
    try {
      const { emit } = await import('@tauri-apps/api/event');
      emitDesktopEvent = emit;
    } catch {
      emitDesktopEvent = null;
    }
  }

  async function installSettingsCloseListener() {
    try {
      const { listen } = await import('@tauri-apps/api/event');
      const unlisten = await listen<number>('nomo://settings-request-close', (event) => {
        void handleClose(event.payload);
      });
      if (settingsCloseListenerCancelled) {
        unlisten();
        return;
      }
      settingsCloseUnlisten = unlisten;
      const { invoke } = await import('@tauri-apps/api/core');
      if (settingsCloseListenerCancelled) {
        settingsCloseUnlisten?.();
        settingsCloseUnlisten = null;
        return;
      }
      await invoke('mark_settings_close_handler_ready');
    } catch (error) {
      settingsCloseUnlisten?.();
      settingsCloseUnlisten = null;
      logToTerminal('error', 'SettingsWindow', '监听设置窗口关闭请求失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  function handleWindowFocus() {
    logToTerminal('debug', 'SettingsWindow', '设置窗口获得焦点', { activeCategory });
    if (activeCategory !== 'files') {
      return;
    }
    // 防抖：避免频繁切换焦点导致反复查询
    if (focusDebounceTimer !== null) {
      window.clearTimeout(focusDebounceTimer);
    }
    focusDebounceTimer = window.setTimeout(() => {
      focusDebounceTimer = null;
      void refreshMarkdownAssociationStatus({ silent: true });
      void refreshWindowsContextMenuStatus({ silent: true });
    }, 300);
  }

  function selectCategory(categoryId: CategoryId) {
    logToTerminal('info', 'SettingsWindow', `切换到分类: ${categoryId}`);
    activeCategory = categoryId;
    if (categoryId === 'files') {
      void ensureFilesIntegrationStatus();
    }
  }

  async function ensureFilesIntegrationStatus() {
    if (filesIntegrationStatusRequested) {
      logToTerminal('debug', 'SettingsWindow', '文件集成状态已请求过，跳过首次刷新');
      return;
    }

    const timer = createPerfTimer('SettingsWindow', 'ensureFilesIntegrationStatus');
    try {
      await Promise.all([refreshMarkdownAssociationStatus(), refreshWindowsContextMenuStatus()]);
      filesIntegrationStatusRequested = true;
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '文件集成状态首次刷新失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    } finally {
      timer.end();
    }
  }

  async function loadPreferences() {
    logToTerminal('info', 'SettingsWindow', '开始加载偏好设置');
    const timer = createPerfTimer('SettingsWindow', 'loadPreferences');
    draftSettings = await loadAppPreferences(desktopEnabled);
    if (!systemThemeSyncActive) return;
    lastPersistedSettings = draftSettings;
    const initialSystemScheme = await readEffectiveSystemScheme(desktopEnabled);
    if (!systemThemeSyncActive) return;
    const resolved = await applySettingsToThisWindow(draftSettings, initialSystemScheme);
    if (!systemThemeSyncActive) return;
    effectiveSystemScheme = initialSystemScheme;
    writeThemeBootSnapshot(resolved);
    loaded = true;
    timer.end({ desktopEnabled });
    logToTerminal('info', 'SettingsWindow', '偏好设置加载完成');
  }

  async function saveLatestSettings() {
    if (!loaded) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '开始保存偏好设置', {
      keys: Array.from(dirtyPreferenceKeys),
    });

    if (saveInFlight) {
      saveQueued = true;
      return activeSavePromise ?? Promise.resolve();
    }

    saveInFlight = true;
    activeSavePromise = (async () => {
      try {
        do {
          saveQueued = false;
          const settingsToSave = draftSettings;
          const keysToSave = Array.from(dirtyPreferenceKeys);
          dirtyPreferenceKeys = new Set();
          if (keysToSave.length === 0) {
            showStatus(t.settingsSaved());
            continue;
          }
          try {
            const saved = await saveAppPreferences(desktopEnabled, settingsToSave, keysToSave);
            if (
              desktopEnabled &&
              (keysToSave.includes('interfaceLanguage') ||
                keysToSave.includes('shortcutPreferences'))
            ) {
              const { invoke } = await import('@tauri-apps/api/core');
              await invoke('refresh_interface_language_chrome').catch(() => undefined);
            }
            lastPersistedSettings = normalizeAppPreferences({
              ...lastPersistedSettings,
              ...Object.fromEntries(keysToSave.map((key) => [key, saved[key]])),
            });
            if (draftSettings === settingsToSave) {
              draftSettings = saved;
              await applySettingsToThisWindow(saved);
            }
            if (keysToSave.some(isAppearancePreferenceKey)) {
              writeThemeBootSnapshot(resolveTheme(saved, effectiveSystemScheme));
            }
            logToTerminal('info', 'SettingsWindow', '偏好设置保存成功', {
              keys: keysToSave,
            });
            showStatus(t.settingsSaved());
          } catch (error) {
            const appearanceKeys = keysToSave.filter(
              (key) => isAppearancePreferenceKey(key) && draftSettings[key] === settingsToSave[key],
            );
            const otherKeys = keysToSave.filter((key) => !isAppearancePreferenceKey(key));
            otherKeys.forEach((key) => dirtyPreferenceKeys.add(key));
            if (appearanceKeys.length > 0) {
              const rollbackPatch = Object.fromEntries(
                appearanceKeys.map((key) => [key, lastPersistedSettings[key]]),
              ) as AppPreferencesPatch;
              draftSettings = normalizeAppPreferences({
                ...draftSettings,
                ...rollbackPatch,
              });
              await applySettingsToThisWindow(draftSettings);
              await emitSettingsUpdated(rollbackPatch);
            }
            logToTerminal('error', 'SettingsWindow', '偏好设置保存失败', {
              error: error instanceof Error ? error.message : String(error),
            });
            showStatus(error instanceof Error ? error.message : t.settingsSaveFailed());
            throw error;
          }
        } while (saveQueued);
      } finally {
        saveInFlight = false;
        activeSavePromise = null;
      }
    })();

    return activeSavePromise;
  }

  function scheduleAutoSave() {
    if (!loaded) {
      return;
    }

    if (autoSaveTimer !== null) {
      window.clearTimeout(autoSaveTimer);
    }
    showStatus(t.settingsSaving());
    autoSaveTimer = window.setTimeout(() => {
      autoSaveTimer = null;
      void saveLatestSettings().catch(() => undefined);
    }, 350);
  }

  async function flushPendingSettingsSave() {
    if (autoSaveTimer !== null) {
      window.clearTimeout(autoSaveTimer);
      autoSaveTimer = null;
    }

    await saveLatestSettings();
  }

  async function handleClose(requestId?: number) {
    if (closeInProgress) {
      return;
    }
    closeInProgress = true;
    logToTerminal('info', 'SettingsWindow', '收到设置窗口关闭请求');
    try {
      if (desktopEnabled && requestId !== undefined) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('acknowledge_settings_close_request', { requestId });
      }
      await flushPendingSettingsSave();
      await closeCurrentWindow();
    } catch (error) {
      closeInProgress = false;
      if (desktopEnabled && requestId !== undefined) {
        await import('@tauri-apps/api/core')
          .then(({ invoke }) => invoke('cancel_settings_close_request', { requestId }))
          .catch(() => undefined);
      }
      logToTerminal('error', 'SettingsWindow', '关闭设置窗口失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  /**
   * 把设置补丁广播给所有文档窗。
   *
   * 外观变更必须在 Dock 图标 IPC 之前发出，并带上已经解析好的 `effectiveScheme`，
   * 让主窗立刻上色，不再等系统主题查询。
   *
   * @param patch 本次变更的偏好字段。
   * @param effectiveScheme 设置窗已经落地的有效深浅色；非外观变更可缺省。
   */
  async function emitSettingsUpdated(
    patch: AppPreferencesPatch,
    effectiveScheme?: ColorScheme,
  ) {
    if (!desktopEnabled) {
      return;
    }
    const payload = { source: 'settings-window' as const, patch, effectiveScheme };
    if (emitDesktopEvent) {
      await emitDesktopEvent(SETTINGS_UPDATED_EVENT, payload).catch(() => undefined);
      return;
    }
    const { emit } = await import('@tauri-apps/api/event');
    emitDesktopEvent = emit;
    await emit(SETTINGS_UPDATED_EVENT, payload).catch(() => undefined);
  }

  async function closeCurrentWindow() {
    if (!desktopEnabled) {
      window.close();
      return;
    }
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('close_window');
  }

  /**
   * 把偏好应用到设置窗自身，不负责跨窗广播。
   *
   * @param settings 要落地的完整偏好。
   * @param systemScheme 解析「跟随系统」时使用的深浅色。
   * @param options.syncDesktopIcons 是否同步 Dock / 窗口图标；点击外观时应先广播再同步。
   * @returns 本次写入设置窗的已解析主题。
   */
  async function applySettingsToThisWindow(
    settings: AppPreferences,
    systemScheme: ColorScheme = effectiveSystemScheme,
    options: { syncDesktopIcons?: boolean } = {},
  ) {
    interfaceLocale = applyInterfaceLanguagePreference(settings.interfaceLanguage);
    const resolved = applyThemeRuntime(settings, {
      systemScheme,
      desktopEnabled,
      syncDesktopIcons: options.syncDesktopIcons,
    });
    applyTypographySettings(settings.fontSize, settings.lineHeight);
    applyEditorLayoutSettings(settings.contentWidthPercent);
    return resolved;
  }

  /**
   * 跟随系统时同步设置窗外观。
   *
   * @param systemScheme 原生事件带来的深浅色；缺省时才去读桌面主题。
   */
  async function syncSystemTheme(systemScheme?: ColorScheme) {
    if (!systemThemeSyncActive || !loaded) {
      return;
    }

    const nextSystemScheme = systemScheme ?? (await readEffectiveSystemScheme(desktopEnabled));
    if (!systemThemeSyncActive || !loaded || nextSystemScheme === effectiveSystemScheme) {
      return;
    }
    if (draftSettings.themeMode !== 'system') {
      effectiveSystemScheme = nextSystemScheme;
      return;
    }
    await applySettingsToThisWindow(draftSettings, nextSystemScheme);
    if (!systemThemeSyncActive || !loaded || draftSettings.themeMode !== 'system') {
      return;
    }
    effectiveSystemScheme = nextSystemScheme;
  }

  function isAppearancePreferenceKey(key: AppPreferenceKey) {
    return key === 'themeMode' || key === 'colorThemeId' || key === 'documentStyleId';
  }

  function showStatus(message: string) {
    statusMessage = message;
    if (statusTimer !== null) {
      window.clearTimeout(statusTimer);
    }
    statusTimer = window.setTimeout(() => {
      statusMessage = '';
      statusTimer = null;
    }, 1800);
  }

  async function openProjectLink(href: string) {
    logToTerminal('info', 'SettingsWindow', '打开项目外部链接', { href });
    try {
      await openExternalLink(href);
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '打开项目外部链接失败', {
        href,
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(t.externalLinkOpenFailed());
    }
  }

  let pendingUpdateCandidate: SoftwareUpdateCandidate | null = null;

  async function checkForSoftwareUpdate() {
    if (['checking', 'downloading', 'installing'].includes(updateState.status)) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '开始检查软件更新');

    try {
      await runSoftwareUpdateCheck(false);
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '软件更新检查失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function downloadAvailableUpdate() {
    if (
      !pendingUpdateCandidate ||
      pendingUpdateCandidate.assetKind !== 'windowsInstaller' ||
      updateState.status !== 'available'
    ) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '用户确认，开始下载更新');

    try {
      downloadedSoftwareUpdate = await startSoftwareUpdateDownload(pendingUpdateCandidate);
      logToTerminal('info', 'SettingsWindow', '软件更新下载完成');
    } catch (error) {
      downloadedSoftwareUpdate = null;
      logToTerminal('error', 'SettingsWindow', '软件更新下载失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  function applySharedSoftwareUpdateState(snapshot: SoftwareUpdateSnapshot) {
    softwareUpdateSnapshot = snapshot;
    pendingUpdateCandidate = snapshot.candidate ?? null;
    downloadedSoftwareUpdate = snapshot.downloadedUpdate ?? null;
    updateState = {
      status: snapshot.status,
      message: getSoftwareUpdateMessage(snapshot),
      version: snapshot.version,
      progress: snapshot.progress,
      error: snapshot.error,
    };
  }

  async function installDownloadedSoftwareUpdate() {
    if (!downloadedSoftwareUpdate || updateState.status !== 'downloaded') {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '开始安装软件更新');

    try {
      const approved = await requestUpdateInstallApproval();
      if (!approved) {
        logToTerminal('info', 'SettingsWindow', '用户取消安装更新');
        updateState = {
          ...updateState,
          status: 'downloaded',
          message: t.softwareUpdateWaitingInstall(),
        };
        return;
      }

      await startSoftwareUpdateInstall(downloadedSoftwareUpdate);
      logToTerminal('info', 'SettingsWindow', '软件更新安装完成');
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '软件更新安装失败', {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function requestUpdateInstallApproval(): Promise<boolean> {
    if (!desktopEnabled) {
      return false;
    }

    const requestId = `update-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const { emit, listen } = await import('@tauri-apps/api/event');

    return new Promise<boolean>((resolve) => {
      let settled = false;
      let timeoutId: number | null = null;

      const settle = (approved: boolean) => {
        if (settled) {
          return;
        }
        settled = true;
        if (timeoutId !== null) {
          window.clearTimeout(timeoutId);
        }
        if (updateDecisionUnlisten) {
          updateDecisionUnlisten();
          updateDecisionUnlisten = null;
        }
        resolve(approved);
      };

      listen<{ requestId?: string; approved?: boolean }>(
        'nomo://update-install-decision',
        (event) => {
          if (event.payload?.requestId !== requestId) {
            return;
          }
          settle(Boolean(event.payload.approved));
        },
      )
        .then((unlisten) => {
          updateDecisionUnlisten = unlisten;
          return emit('nomo://request-update-install', { requestId });
        })
        .catch(() => settle(false));

      timeoutId = window.setTimeout(() => settle(false), 30_000);
    });
  }

  function getSoftwareUpdateMessage(snapshot: SoftwareUpdateSnapshot) {
    switch (snapshot.status) {
      case 'checking':
        return t.softwareUpdateChecking();
      case 'upToDate':
        return t.softwareUpdateUpToDate();
      case 'available':
        return t.softwareUpdateAvailable({ version: snapshot.version ?? '' });
      case 'downloading':
        return typeof snapshot.progress?.percent === 'number'
          ? t.softwareUpdateDownloadingPercent({ percent: snapshot.progress.percent })
          : t.softwareUpdateDownloading();
      case 'downloaded':
        return t.softwareUpdateDownloaded();
      case 'installing':
        return t.softwareUpdateInstalling();
      case 'managed':
        return t.softwareUpdateStoreManaged();
      case 'unsupported':
        return platformCapabilities.isMac
          ? t.softwareUpdateUnsupportedMacos()
          : t.softwareUpdateUnsupported();
      case 'error':
        return t.softwareUpdateFailed();
      default:
        return '';
    }
  }

  // 以下软件更新相关的 UI 派生值必须用 $: 声明，不能使用普通函数。
  // 原因：Svelte 5 兼容模式下，编译器无法穿透函数体追踪 updateState/updateStatus 的依赖，
  // 导致状态变为 idle 后按钮仍然显示"检测中"且转圈不消失。
  $: softwareUpdateButtonLabel =
    updateStatus === 'checking'
      ? t.softwareUpdateCheckingShort()
      : updateStatus === 'downloading'
        ? t.softwareUpdateDownloadingShort()
        : updateStatus === 'available'
          ? t.softwareUpdateViewDetails()
          : updateStatus === 'managed'
            ? t.softwareUpdateViewDetails()
            : updateStatus === 'downloaded'
              ? t.softwareUpdateRestartAndInstall()
              : updateStatus === 'installing'
                ? t.softwareUpdateInstallingShort()
                : updateStatus === 'unsupported' && platformCapabilities.isMac
                  ? t.softwareUpdateCopyHomebrewCommand()
                  : t.softwareUpdateCheckNow();

  $: softwareUpdateDescription = updateState.error
    ? updateState.error
    : updateState.message || t.updateCheckDescription();

  $: softwareUpdatePillLabel =
    updateStatus === 'checking'
      ? t.checking()
      : updateStatus === 'upToDate'
        ? t.softwareUpdateLatest()
        : updateStatus === 'managed'
          ? t.softwareUpdateStorePill()
          : updateStatus === 'downloaded'
            ? t.softwareUpdateReady()
            : updateStatus === 'error'
              ? t.checkFailed()
              : updateStatus === 'unsupported'
                ? t.unsupported()
                : updateState.version
                  ? `v${updateState.version}`
                  : t.softwareUpdateManual();

  $: softwareUpdatePillClass =
    updateStatus === 'checking'
      ? 'pending'
      : updateStatus === 'upToDate' || updateStatus === 'downloaded' || updateStatus === 'managed'
        ? 'bound'
        : updateStatus === 'error'
          ? 'error'
          : updateStatus === 'available' || updateStatus === 'downloading'
            ? 'pending'
            : 'idle';

  $: softwareUpdateButtonDisabled =
    updateStatus === 'checking' ||
    updateStatus === 'downloading' ||
    updateStatus === 'installing' ||
    (updateStatus === 'unsupported' && !platformCapabilities.isMac);

  function handleSoftwareUpdateButton() {
    if (updateState.status === 'downloaded') {
      void installDownloadedSoftwareUpdate();
      return;
    }
    if (updateState.status === 'available') {
      softwareUpdateDialogOpen = true;
      return;
    }
    if (updateState.status === 'managed') {
      softwareUpdateDialogOpen = true;
      return;
    }
    if (updateState.status === 'unsupported' && platformCapabilities.isMac) {
      void copyHomebrewSetupCommand();
      return;
    }
    void checkForSoftwareUpdate();
  }

  async function copyHomebrewSetupCommand() {
    try {
      await navigator.clipboard.writeText(HOMEBREW_SETUP_COMMAND);
      showStatus(t.softwareUpdateHomebrewCopied());
    } catch {
      showStatus(t.copyFailed());
    }
  }

  async function openMicrosoftStoreProduct() {
    if (!softwareUpdateSnapshot.storeProductId) {
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_microsoft_store_product');
    } catch (error) {
      showStatus(error instanceof Error ? error.message : String(error));
    }
  }

  function updateDraft(patch: Partial<AppPreferences>) {
    const changedKeys = Object.keys(patch) as AppPreferenceKey[];
    logToTerminal('debug', 'SettingsWindow', '设置项变更', { keys: changedKeys });
    const nextSettings = normalizeAppPreferences({
      ...draftSettings,
      ...patch,
      imageHandlingSettings: patch.imageHandlingSettings ?? draftSettings.imageHandlingSettings,
    });
    const normalizedPatch = createNormalizedPatch(patch, nextSettings);

    draftSettings = nextSettings;
    const appearanceChanged = changedKeys.some(isAppearancePreferenceKey);
    // 先广播解析结果，再写本窗 CSS，避免本窗样式更新占用主窗消息的发送时机。
    const resolved = resolveTheme(nextSettings, effectiveSystemScheme);
    void emitSettingsUpdated(
      normalizedPatch,
      appearanceChanged ? resolved.effectiveScheme : undefined,
    );
    applyThemeRuntime(nextSettings, {
      systemScheme: effectiveSystemScheme,
      desktopEnabled,
      syncDesktopIcons: appearanceChanged,
    });
    interfaceLocale = applyInterfaceLanguagePreference(nextSettings.interfaceLanguage);
    applyTypographySettings(nextSettings.fontSize, nextSettings.lineHeight);
    applyEditorLayoutSettings(nextSettings.contentWidthPercent);
    markDirtyPreferences(normalizedPatch);
    scheduleAutoSave();
  }

  function createNormalizedPatch(
    patch: Partial<AppPreferences>,
    nextSettings: AppPreferences,
  ): AppPreferencesPatch {
    const normalizedPatch: AppPreferencesPatch = {};
    for (const key of Object.keys(patch) as AppPreferenceKey[]) {
      normalizedPatch[key] = nextSettings[key] as never;
    }
    return normalizedPatch;
  }

  function markDirtyPreferences(patch: AppPreferencesPatch) {
    for (const key of Object.keys(patch) as AppPreferenceKey[]) {
      dirtyPreferenceKeys.add(key);
    }
  }

  function updateImageSettings(patch: Partial<AppPreferences['imageHandlingSettings']>) {
    updateDraft({
      imageHandlingSettings: {
        ...draftSettings.imageHandlingSettings,
        ...patch,
      },
    });
  }

  async function handleWindowDrag(event: MouseEvent) {
    if (!desktopEnabled || event.buttons !== 1) {
      return;
    }

    const target = event.target as HTMLElement;
    if (target.closest('button,input,select,textarea,label,a')) {
      return;
    }

    if (!target.closest('[data-drag-region]')) {
      return;
    }

    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const appWindow = getCurrentWindow();
      if (event.detail === 2 && platformCapabilities.usesCustomWindowsTitlebar) {
        await appWindow.toggleMaximize();
      } else if (event.detail === 1) {
        await appWindow.startDragging();
      }
    } catch {
      // ignore
    }
  }

  function setThemeMode(themeMode: ThemeMode) {
    updateDraft({ themeMode });
  }

  function setColorTheme(colorThemeId: string) {
    updateDraft({ colorThemeId });
  }

  function setInterfaceLanguage(interfaceLanguage: InterfaceLanguagePreference) {
    updateDraft({ interfaceLanguage });
  }

  function handleInterfaceLanguageChange(event: Event) {
    const nextLanguage = (event.currentTarget as HTMLSelectElement).value;
    if (
      nextLanguage === 'system' ||
      INTERFACE_LANGUAGE_OPTIONS.some((item) => item.value === nextLanguage)
    ) {
      setInterfaceLanguage(nextLanguage as InterfaceLanguagePreference);
    }
  }

  function setEditorMode(editorMode: EditorModePreference) {
    updateDraft({ editorMode });
  }

  function setSplitViewLayout(splitViewLayout: SplitViewLayoutPreference) {
    updateDraft({ splitViewLayout });
  }

  function setDocumentStyle(documentStyleId: string) {
    updateDraft({ documentStyleId });
  }

  function setOpenBehavior(openDefaultBehavior: OpenDefaultBehavior) {
    updateDraft({ openDefaultBehavior });
  }

  function setCloseWindowBehavior(closeWindowBehavior: CloseWindowBehavior) {
    updateDraft({ closeWindowBehavior });
  }

  function setExternalFileChangeBehavior(externalFileChangeBehavior: ExternalFileChangeBehavior) {
    updateDraft({ externalFileChangeBehavior });
  }

  function setStatsMetric(writingStatsMetric: WritingStatsMetric) {
    updateDraft({ writingStatsMetric });
  }

  function setImageStrategy(imageInsertStrategy: ImageInsertStrategy) {
    updateImageSettings({ imageInsertStrategy });
  }

  function setUploadProvider(uploadProvider: ImageUploadProvider) {
    updateImageSettings({ uploadProvider });
  }

  function setImageDefaultAlign(defaultImageAlign: ImageDefaultAlign) {
    updateImageSettings({ defaultImageAlign });
  }

  function setCodeBlockIndent(codeBlockIndent: CodeBlockIndentPreference) {
    updateDraft({ codeBlockIndent });
  }

  function setRenderMode(renderMode: RenderModePreference) {
    updateDraft({ renderMode });
  }

  function setMarkdownLintRuleSet(markdownLintRuleSet: MarkdownLintRuleSet) {
    updateDraft({ markdownLintRuleSet });
  }

  function updateShortcut(commandId: ShortcutCommandId, event: Event) {
    updateDraft({
      shortcutPreferences: {
        ...draftSettings.shortcutPreferences,
        [commandId]: (event.currentTarget as HTMLInputElement).value,
      },
    });
  }

  function updateNumberSetting(key: keyof AppPreferences, event: Event) {
    updateDraft({ [key]: Number((event.currentTarget as HTMLInputElement).value) });
  }

  function updateStringSetting(key: keyof AppPreferences, event: Event) {
    updateDraft({ [key]: (event.currentTarget as HTMLInputElement).value });
  }

  function updateImageStringSetting(
    key: keyof AppPreferences['imageHandlingSettings'],
    event: Event,
  ) {
    updateImageSettings({ [key]: (event.currentTarget as HTMLInputElement).value });
  }

  function toggleSetting(key: keyof AppPreferences, event: Event) {
    updateDraft({ [key]: (event.currentTarget as HTMLInputElement).checked });
  }

  function toggleImageSetting(key: keyof AppPreferences['imageHandlingSettings'], event: Event) {
    updateImageSettings({ [key]: (event.currentTarget as HTMLInputElement).checked });
  }

  async function testPicgoConnection() {
    if (!desktopEnabled || picgoTesting) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '测试 PicGo 连接');
    picgoTesting = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ ok: boolean; message: string }>('test_picgo_connection', {
        input: {
          provider: draftSettings.imageHandlingSettings.uploadProvider,
          server_url: draftSettings.imageHandlingSettings.picgoServerUrl,
          command: draftSettings.imageHandlingSettings.picgoCoreCommand,
        },
      });
      logToTerminal('info', 'SettingsWindow', 'PicGo 连接测试结果', { ok: result.ok });
      showStatus(result.message);
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', 'PicGo 连接测试失败', {
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(error instanceof Error ? error.message : String(error));
    } finally {
      picgoTesting = false;
    }
  }

  async function refreshMarkdownAssociationStatus(options: { silent?: boolean } = {}) {
    if (!desktopEnabled || !platformCapabilities.isWindows || checkingMdAssociation) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '查询 Markdown 默认打开方式状态');

    checkingMdAssociation = true;
    if (!options.silent) {
      mdAssociationError = '';
    }

    // 超时后备：防止子进程异常导致状态永久卡住
    let completed = false;
    const timeoutId = window.setTimeout(() => {
      if (!completed) {
        checkingMdAssociation = false;
        mdAssociationError = mdAssociationError || '查询超时，请重试';
        logToTerminal('error', 'SettingsWindow', 'Markdown 关联状态查询超时');
      }
    }, 8000);

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      mdAssociationStatus = await invoke<MarkdownAssociationStatus>(
        'get_markdown_file_association_status',
      );
      mdAssociationError = '';
      logToTerminal('info', 'SettingsWindow', 'Markdown 关联状态查询完成', {
        isDefault: mdAssociationStatus?.is_default,
        registered: mdAssociationStatus?.registered,
      });
    } catch (error) {
      mdAssociationError = error instanceof Error ? error.message : String(error);
      logToTerminal('error', 'SettingsWindow', 'Markdown 关联状态查询失败', {
        error: mdAssociationError,
      });
    } finally {
      completed = true;
      window.clearTimeout(timeoutId);
      checkingMdAssociation = false;
    }
  }

  function getMarkdownAssociationLabel() {
    if (!desktopEnabled || !platformCapabilities.isWindows) {
      return t.unsupported();
    }
    if (checkingMdAssociation && !mdAssociationStatus) {
      return t.checking();
    }
    if (mdAssociationError) {
      return t.checkFailed();
    }
    if (mdAssociationStatus?.is_default) {
      return t.bound();
    }
    if (mdAssociationStatus?.registered) {
      return t.pendingSelection();
    }
    return t.unbound();
  }

  function getMarkdownAssociationDescription() {
    if (!desktopEnabled) {
      return t.mdAssociationDesktopOnly();
    }
    if (!platformCapabilities.isWindows) {
      return t.mdAssociationWindowsOnly();
    }
    if (mdAssociationError) {
      return mdAssociationError;
    }
    if (checkingMdAssociation && !mdAssociationStatus) {
      return t.mdAssociationCheckingDescription();
    }
    return mdAssociationStatus?.message ?? t.mdAssociationDefaultDescription();
  }

  function getMarkdownAssociationButtonLabel() {
    if (bindingMdAssociation) {
      return t.opening();
    }
    if (mdAssociationStatus?.is_default) {
      return t.bound();
    }
    if (mdAssociationStatus?.registered) {
      return t.chooseNomo();
    }
    return t.bindMd();
  }

  function getMarkdownAssociationPillClass() {
    if (mdAssociationStatus?.is_default) {
      return 'bound';
    }
    if (mdAssociationError) {
      return 'error';
    }
    if (mdAssociationStatus?.registered) {
      return 'pending';
    }
    return 'idle';
  }

  async function refreshWindowsContextMenuStatus(options: { silent?: boolean } = {}) {
    if (!desktopEnabled || !platformCapabilities.isWindows || checkingContextMenu) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '查询右键菜单状态');

    checkingContextMenu = true;
    if (!options.silent) {
      contextMenuError = '';
    }

    // 超时后备：防止子进程异常导致状态永久卡住
    let completed = false;
    const timeoutId = window.setTimeout(() => {
      if (!completed) {
        checkingContextMenu = false;
        contextMenuError = contextMenuError || '查询超时，请重试';
        logToTerminal('error', 'SettingsWindow', '右键菜单状态查询超时');
      }
    }, 8000);

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      contextMenuStatus = await invoke<WindowsContextMenuStatus>('get_windows_context_menu_status');
      contextMenuError = '';
      logToTerminal('info', 'SettingsWindow', '右键菜单状态查询完成', {
        registered: contextMenuStatus?.registered,
      });
    } catch (error) {
      contextMenuError = error instanceof Error ? error.message : String(error);
      logToTerminal('error', 'SettingsWindow', '右键菜单状态查询失败', {
        error: contextMenuError,
      });
    } finally {
      completed = true;
      window.clearTimeout(timeoutId);
      checkingContextMenu = false;
    }
  }

  function getContextMenuLabel() {
    if (!desktopEnabled || !platformCapabilities.isWindows) {
      return t.unsupported();
    }
    if (checkingContextMenu && !contextMenuStatus) {
      return t.checking();
    }
    if (contextMenuError) {
      return t.checkFailed();
    }
    return contextMenuStatus?.registered ? t.registered() : t.unregistered();
  }

  function getContextMenuDescription() {
    if (!desktopEnabled) {
      return t.contextMenuDesktopOnly();
    }
    if (!platformCapabilities.isWindows) {
      return t.contextMenuWindowsOnly();
    }
    if (contextMenuError) {
      return contextMenuError;
    }
    if (checkingContextMenu && !contextMenuStatus) {
      return t.contextMenuCheckingDescription();
    }
    return contextMenuStatus?.message ?? t.contextMenuDefaultDescription();
  }

  function getContextMenuButtonLabel() {
    if (registeringContextMenu) {
      return t.registering();
    }
    if (contextMenuStatus?.registered) {
      return t.registered();
    }
    return t.registerContextMenu();
  }

  function getContextMenuPillClass() {
    if (contextMenuStatus?.registered) {
      return 'bound';
    }
    if (contextMenuError) {
      return 'error';
    }
    return 'idle';
  }

  // Svelte 5 兼容模式下，函数体内部的依赖无法被编译器穿透追踪，
  // 必须将条件逻辑直接展开到 $: 声明中，确保状态变化时 UI 正确更新。
  $: mdAssociationLabel =
    !desktopEnabled || !platformCapabilities.isWindows
      ? t.unsupported()
      : checkingMdAssociation && !mdAssociationStatus
        ? t.checking()
        : mdAssociationError
          ? t.checkFailed()
          : mdAssociationStatus?.managedByPackage
            ? t.managedByWindows()
            : mdAssociationStatus?.is_default
              ? t.bound()
              : mdAssociationStatus?.registered
                ? t.pendingSelection()
                : t.unbound();

  $: mdAssociationDesc = !desktopEnabled
    ? t.mdAssociationDesktopOnly()
    : !platformCapabilities.isWindows
      ? t.mdAssociationWindowsOnly()
      : mdAssociationError
        ? mdAssociationError
        : checkingMdAssociation && !mdAssociationStatus
          ? t.mdAssociationCheckingDescription()
          : (mdAssociationStatus?.message ?? t.mdAssociationDefaultDescription());

  $: mdAssociationBtnLabel = bindingMdAssociation
    ? t.opening()
    : unbindingMdAssociation
      ? t.unbinding()
      : mdAssociationStatus?.managedByPackage
        ? t.openWindowsDefaultApps()
        : mdAssociationStatus?.is_default || mdAssociationStatus?.registered
          ? t.unbindMd()
          : t.bindMd();

  $: mdAssociationPillClass =
    mdAssociationStatus?.managedByPackage || mdAssociationStatus?.is_default
      ? 'bound'
      : mdAssociationError
        ? 'error'
        : mdAssociationStatus?.registered
          ? 'pending'
          : 'idle';

  $: contextMenuLabel =
    !desktopEnabled || !platformCapabilities.isWindows
      ? t.unsupported()
      : checkingContextMenu && !contextMenuStatus
        ? t.checking()
        : contextMenuError
          ? t.checkFailed()
          : contextMenuStatus?.managedByPackage
            ? contextMenuStatus.enabled
              ? t.enabled()
              : t.disabled()
            : contextMenuStatus?.registered
              ? t.registered()
              : t.unregistered();

  $: contextMenuDesc = !desktopEnabled
    ? t.contextMenuDesktopOnly()
    : !platformCapabilities.isWindows
      ? t.contextMenuWindowsOnly()
      : contextMenuError
        ? contextMenuError
        : checkingContextMenu && !contextMenuStatus
          ? t.contextMenuCheckingDescription()
          : (contextMenuStatus?.message ?? t.contextMenuDefaultDescription());

  $: contextMenuBtnLabel = registeringContextMenu
    ? t.registering()
    : unregisteringContextMenu
      ? t.unregistering()
      : contextMenuStatus?.managedByPackage
        ? contextMenuStatus.enabled
          ? t.disableContextMenu()
          : t.enableContextMenu()
        : contextMenuStatus?.registered
          ? t.unregisterContextMenu()
          : t.registerContextMenu();

  $: contextMenuPillClass = contextMenuStatus?.managedByPackage
    ? contextMenuStatus.enabled
      ? 'bound'
      : 'idle'
    : contextMenuStatus?.registered
      ? 'bound'
      : contextMenuError
        ? 'error'
        : 'idle';

  async function registerWindowsContextMenu() {
    if (!desktopEnabled || !platformCapabilities.isWindows || registeringContextMenu) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '点击注册右键菜单按钮');

    registeringContextMenu = true;
    try {
      // 步骤1：先异步查询当前状态
      await refreshWindowsContextMenuStatus({ silent: true });

      // 步骤2：如果已注册，直接提示并返回
      if (contextMenuStatus?.registered && !contextMenuStatus.managedByPackage) {
        logToTerminal('info', 'SettingsWindow', '右键菜单已注册，跳过');
        showStatus(t.registered());
        return;
      }

      // 步骤3：执行注册
      logToTerminal('info', 'SettingsWindow', '开始注册右键菜单');
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ ok: boolean; message: string }>(
        'register_windows_context_menu',
      );
      logToTerminal('info', 'SettingsWindow', '右键菜单注册完成', { ok: result.ok });
      showStatus(result.message);
      await refreshWindowsContextMenuStatus({ silent: true });
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '右键菜单注册失败', {
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(error instanceof Error ? error.message : String(error));
    } finally {
      registeringContextMenu = false;
    }
  }

  async function bindMarkdownAssociation() {
    if (!desktopEnabled || !platformCapabilities.isWindows || bindingMdAssociation) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '点击绑定默认打开方式按钮');

    bindingMdAssociation = true;
    try {
      // 步骤1：先异步查询当前状态
      await refreshMarkdownAssociationStatus({ silent: true });

      // 步骤2：如果已经是默认，直接提示并返回
      if (mdAssociationStatus?.is_default) {
        logToTerminal('info', 'SettingsWindow', '已是默认打开方式，跳过');
        showStatus(t.bound());
        return;
      }

      // 步骤3：执行绑定
      logToTerminal('info', 'SettingsWindow', '开始绑定 Markdown 默认打开方式');
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ ok: boolean; message: string }>(
        'register_markdown_file_association',
      );
      logToTerminal('info', 'SettingsWindow', '绑定默认打开方式完成', { ok: result.ok });
      showStatus(result.message);
      await refreshMarkdownAssociationStatus({ silent: true });
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '绑定默认打开方式失败', {
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(error instanceof Error ? error.message : String(error));
    } finally {
      bindingMdAssociation = false;
    }
  }

  async function unbindMarkdownAssociation() {
    if (!desktopEnabled || !platformCapabilities.isWindows || unbindingMdAssociation) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '点击取消绑定默认打开方式按钮');

    unbindingMdAssociation = true;
    try {
      logToTerminal('info', 'SettingsWindow', '开始取消 Markdown 默认打开方式绑定');
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ ok: boolean; message: string }>(
        'unregister_markdown_file_association',
      );
      logToTerminal('info', 'SettingsWindow', '取消绑定默认打开方式完成', { ok: result.ok });
      showStatus(result.message);
      await refreshMarkdownAssociationStatus({ silent: true });
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '取消绑定默认打开方式失败', {
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(error instanceof Error ? error.message : String(error));
    } finally {
      unbindingMdAssociation = false;
    }
  }

  async function unregisterWindowsContextMenu() {
    if (!desktopEnabled || !platformCapabilities.isWindows || unregisteringContextMenu) {
      return;
    }
    logToTerminal('info', 'SettingsWindow', '点击取消注册右键菜单按钮');

    unregisteringContextMenu = true;
    try {
      logToTerminal('info', 'SettingsWindow', '开始取消右键菜单注册');
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ ok: boolean; message: string }>(
        'unregister_windows_context_menu',
      );
      logToTerminal('info', 'SettingsWindow', '取消右键菜单注册完成', { ok: result.ok });
      showStatus(result.message);
      await refreshWindowsContextMenuStatus({ silent: true });
    } catch (error) {
      logToTerminal('error', 'SettingsWindow', '取消右键菜单注册失败', {
        error: error instanceof Error ? error.message : String(error),
      });
      showStatus(error instanceof Error ? error.message : String(error));
    } finally {
      unregisteringContextMenu = false;
    }
  }
</script>

<svelte:window on:contextmenu={suppressUnhandledContextMenu} />

<svelte:head>
  {#key interfaceLocale}
    <title>{t.settingsWindowTitle()}</title>
  {/key}
</svelte:head>

{#key interfaceLocale}
  <div class="settings-window-shell" data-interface-locale={interfaceLocale}>
    <aside class="settings-nav" aria-label={t.settingsTitle()}>
      <div
        class="settings-brand"
        data-drag-region
        role="presentation"
        on:mousedown={handleWindowDrag}
      >
        <span class="settings-brand-logo" aria-hidden="true">
          <img class="logo-light" src={nomoLogoLight} alt="" draggable="false" />
          <img class="logo-dark" src={nomoLogoDark} alt="" draggable="false" />
        </span>
        <span>{t.settingsTitle()}</span>
      </div>

      <nav>
        {#each categories as category}
          <button
            type="button"
            class:active={activeCategory === category.id}
            aria-label={t[category.labelKey]()}
            aria-current={activeCategory === category.id ? 'page' : undefined}
            on:click={() => {
              selectCategory(category.id);
            }}
          >
            <svelte:component this={category.icon} size={16} aria-hidden="true" />
            <span>{t[category.labelKey]()}</span>
          </button>
        {/each}
      </nav>
    </aside>

    <section class="settings-main" aria-labelledby="settings-title">
      <header
        class="settings-header"
        data-drag-region
        role="presentation"
        on:mousedown={handleWindowDrag}
      >
        <div class="settings-header-title" data-drag-region>
          <h1 id="settings-title">{categoryTitles[activeCategory]}</h1>
          <span class:visible={statusMessage} role="status" data-drag-region>{statusMessage}</span>
        </div>
        {#if desktopEnabled && platformCapabilities.usesCustomWindowsTitlebar}
          <WindowsCaptionControls onClose={() => handleClose()} />
        {:else if !desktopEnabled}
          <button
            type="button"
            class="close-button"
            aria-label={t.close()}
            on:click={() => void handleClose()}
          >
            <X size={18} />
          </button>
        {/if}
      </header>

      {#if !loaded}
        <div class="settings-loading" role="status">{t.settingsLoading()}</div>
      {:else}
        <div class="settings-content">
          {#if activeCategory === 'general'}
            <div class="settings-group">
              <h2>{t.basicBehavior()}</h2>
              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.editorModeDefault()}</span>
                  <p>{t.editorModeDefaultDescription()}</p>
                </div>
                <div class="triple-control" role="group" aria-label={t.editorModeDefault()}>
                  <button
                    type="button"
                    class:active={draftSettings.editorMode === 'semantic'}
                    aria-pressed={draftSettings.editorMode === 'semantic'}
                    on:click={() => setEditorMode('semantic')}>{t.semanticEditing()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.editorMode === 'source'}
                    aria-pressed={draftSettings.editorMode === 'source'}
                    on:click={() => setEditorMode('source')}>{t.sourceMode()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.editorMode === 'split'}
                    aria-pressed={draftSettings.editorMode === 'split'}
                    on:click={() => setEditorMode('split')}>{t.splitMode()}</button
                  >
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.splitViewLayout()}</span>
                  <p>{t.splitViewLayoutDescription()}</p>
                </div>
                <div class="segmented-control" role="group" aria-label={t.splitViewLayout()}>
                  <button
                    type="button"
                    class:active={draftSettings.splitViewLayout === 'semantic-source'}
                    aria-pressed={draftSettings.splitViewLayout === 'semantic-source'}
                    on:click={() => setSplitViewLayout('semantic-source')}
                    >{t.semanticOnLeft()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.splitViewLayout === 'source-semantic'}
                    aria-pressed={draftSettings.splitViewLayout === 'source-semantic'}
                    on:click={() => setSplitViewLayout('source-semantic')}
                    >{t.sourceOnLeft()}</button
                  >
                </div>
              </div>

              <label class="toggle-row" for="autoSaveEnabled">
                <span>
                  <span class="toggle-title">{t.autoSave()}</span>
                  <span class="toggle-desc">{t.autoSaveDescription()}</span>
                </span>
                <input
                  id="autoSaveEnabled"
                  type="checkbox"
                  checked={draftSettings.autoSaveEnabled}
                  on:change={(event) => toggleSetting('autoSaveEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <label for="autoSaveDelayMs" class="setting-label">{t.autoSaveDelay()}</label>
                  <p>{t.autoSaveDelayDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="autoSaveDelayMs"
                    type="range"
                    min="500"
                    max="5000"
                    step="100"
                    value={draftSettings.autoSaveDelayMs}
                    on:input={(event) => updateNumberSetting('autoSaveDelayMs', event)}
                  />
                  <output for="autoSaveDelayMs">{draftSettings.autoSaveDelayMs}ms</output>
                </div>
              </div>

              <label class="toggle-row" for="createSnapshotBeforeSave">
                <span>
                  <span class="toggle-title">{t.createSnapshotBeforeSave()}</span>
                  <span class="toggle-desc">{t.createSnapshotBeforeSaveDescription()}</span>
                </span>
                <input
                  id="createSnapshotBeforeSave"
                  type="checkbox"
                  checked={draftSettings.createSnapshotBeforeSave}
                  on:change={(event) => toggleSetting('createSnapshotBeforeSave', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.interfaceLanguage()}</span>
                  <p>{t.interfaceLanguageDescription()}</p>
                </div>
                <select
                  class="select-input"
                  aria-label={t.interfaceLanguage()}
                  value={draftSettings.interfaceLanguage}
                  on:change={handleInterfaceLanguageChange}
                >
                  {#each INTERFACE_LANGUAGE_OPTIONS as language}
                    <option value={language.value}>{t[language.labelKey]()}</option>
                  {/each}
                </select>
              </div>
            </div>
          {:else if activeCategory === 'editor'}
            <div class="settings-group">
              <h2>{t.editorScale()}</h2>
              <div class="setting-row">
                <div>
                  <label for="fontSize" class="setting-label">{t.fontSize()}</label>
                  <p>{t.fontSizeDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="fontSize"
                    type="range"
                    min="14"
                    max="22"
                    step="1"
                    value={draftSettings.fontSize}
                    on:input={(event) => updateNumberSetting('fontSize', event)}
                  />
                  <output for="fontSize">{draftSettings.fontSize}px</output>
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <label for="lineHeight" class="setting-label">{t.lineHeight()}</label>
                  <p>{t.lineHeightDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="lineHeight"
                    type="range"
                    min="1.4"
                    max="2.1"
                    step="0.05"
                    value={draftSettings.lineHeight}
                    on:input={(event) => updateNumberSetting('lineHeight', event)}
                  />
                  <output for="lineHeight">{draftSettings.lineHeight.toFixed(2)}</output>
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <label for="contentWidthPercent" class="setting-label">{t.contentWidth()}</label>
                  <p>{t.contentWidthDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="contentWidthPercent"
                    type="range"
                    min="45"
                    max="90"
                    step="1"
                    value={draftSettings.contentWidthPercent}
                    on:input={(event) => updateNumberSetting('contentWidthPercent', event)}
                  />
                  <output for="contentWidthPercent">{draftSettings.contentWidthPercent}%</output>
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.calloutStyle()}</span>
                  <p>{t.calloutStyleDescription()}</p>
                </div>
                <div class="segmented-control" role="group" aria-label={t.calloutStyle()}>
                  <button
                    type="button"
                    class:active={draftSettings.documentStyleId === CLASSIC_DOCUMENT_STYLE_ID}
                    aria-pressed={draftSettings.documentStyleId === CLASSIC_DOCUMENT_STYLE_ID}
                    on:click={() => setDocumentStyle(CLASSIC_DOCUMENT_STYLE_ID)}
                    >{t.classic()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.documentStyleId === DEFAULT_DOCUMENT_STYLE_ID}
                    aria-pressed={draftSettings.documentStyleId === DEFAULT_DOCUMENT_STYLE_ID}
                    on:click={() => setDocumentStyle(DEFAULT_DOCUMENT_STYLE_ID)}
                    >{t.modern()}</button
                  >
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <label for="largeDocumentLimit" class="setting-label"
                    >{t.largeDocumentLimit()}</label
                  >
                  <p>{t.largeDocumentLimitDescription()}</p>
                </div>
                <div class="number-field">
                  <input
                    id="largeDocumentLimit"
                    type="number"
                    min="100000"
                    max="1000000"
                    step="10000"
                    value={draftSettings.largeDocumentLimit}
                    on:input={(event) => updateNumberSetting('largeDocumentLimit', event)}
                  />
                  <span>{t.charByte()}</span>
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.defaultIndent()}</span>
                  <p>{t.defaultIndentDescription()}</p>
                </div>
                <div class="triple-control" role="group" aria-label={t.defaultIndent()}>
                  <button
                    type="button"
                    class:active={draftSettings.codeBlockIndent === 'spaces-2'}
                    aria-pressed={draftSettings.codeBlockIndent === 'spaces-2'}
                    on:click={() => setCodeBlockIndent('spaces-2')}>{t.twoSpaces()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.codeBlockIndent === 'spaces-4'}
                    aria-pressed={draftSettings.codeBlockIndent === 'spaces-4'}
                    on:click={() => setCodeBlockIndent('spaces-4')}>{t.fourSpaces()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.codeBlockIndent === 'tab'}
                    aria-pressed={draftSettings.codeBlockIndent === 'tab'}
                    on:click={() => setCodeBlockIndent('tab')}>Tab</button
                  >
                </div>
              </div>

              <label class="toggle-row" for="codeBlockLineNumbersVisible">
                <span>
                  <span class="toggle-title">{t.codeBlockLineNumbers()}</span>
                  <span class="toggle-desc">{t.codeBlockLineNumbersDescription()}</span>
                </span>
                <input
                  id="codeBlockLineNumbersVisible"
                  type="checkbox"
                  checked={draftSettings.codeBlockLineNumbersVisible}
                  on:change={(event) => toggleSetting('codeBlockLineNumbersVisible', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <label class="toggle-row" for="inlineCodeRenderingEnabled">
                <span>
                  <span class="toggle-title">{t.inlineCodeRendering()}</span>
                  <span class="toggle-desc">{t.inlineCodeRenderingDescription()}</span>
                </span>
                <input
                  id="inlineCodeRenderingEnabled"
                  type="checkbox"
                  checked={draftSettings.inlineCodeRenderingEnabled}
                  on:change={(event) => toggleSetting('inlineCodeRenderingEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <label class="toggle-row" for="copyMarkdownSyntaxEnabled">
                <span>
                  <span class="toggle-title">{t.copyMarkdownSyntax()}</span>
                  <span class="toggle-desc">{t.copyMarkdownSyntaxDescription()}</span>
                </span>
                <input
                  id="copyMarkdownSyntaxEnabled"
                  type="checkbox"
                  checked={draftSettings.copyMarkdownSyntaxEnabled}
                  on:change={(event) => toggleSetting('copyMarkdownSyntaxEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <h2 class="settings-heading-with-badge">
                <span>{t.markdownLint()}</span>
                <span class="beta-badge">Beta</span>
              </h2>
              <label class="toggle-row" for="markdownLintEnabled">
                <span>
                  <span class="toggle-title">{t.markdownLintEnabled()}</span>
                  <span class="toggle-desc">{t.markdownLintEnabledDescription()}</span>
                </span>
                <input
                  id="markdownLintEnabled"
                  type="checkbox"
                  checked={draftSettings.markdownLintEnabled}
                  on:change={(event) => toggleSetting('markdownLintEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row" class:setting-disabled={!draftSettings.markdownLintEnabled}>
                <div>
                  <span class="setting-label">{t.markdownLintRules()}</span>
                  <p>{t.markdownLintRulesDescription()}</p>
                </div>
                <div class="segmented-control" role="group" aria-label={t.markdownLintRules()}>
                  <button
                    type="button"
                    disabled={!draftSettings.markdownLintEnabled}
                    class:active={draftSettings.markdownLintRuleSet === 'relaxed'}
                    aria-pressed={draftSettings.markdownLintRuleSet === 'relaxed'}
                    on:click={() => setMarkdownLintRuleSet('relaxed')}
                    >{t.markdownLintRelaxed()}</button
                  >
                  <button
                    type="button"
                    disabled={!draftSettings.markdownLintEnabled}
                    class:active={draftSettings.markdownLintRuleSet === 'default'}
                    aria-pressed={draftSettings.markdownLintRuleSet === 'default'}
                    on:click={() => setMarkdownLintRuleSet('default')}
                    >{t.markdownLintDefault()}</button
                  >
                </div>
              </div>
            </div>
          {:else if activeCategory === 'appearance'}
            <div class="settings-group">
              <h2>{t.theme()}</h2>
              <div class="appearance-theme-row">
                <span class="setting-label">{t.themeMode()}</span>
                <div class="triple-control" role="group" aria-label={t.themeMode()}>
                  <button
                    type="button"
                    class:active={draftSettings.themeMode === 'system'}
                    aria-pressed={draftSettings.themeMode === 'system'}
                    on:click={() => setThemeMode('system')}>{t.themeSystem()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.themeMode === 'light'}
                    aria-pressed={draftSettings.themeMode === 'light'}
                    on:click={() => setThemeMode('light')}>{t.themeLight()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.themeMode === 'dark'}
                    aria-pressed={draftSettings.themeMode === 'dark'}
                    on:click={() => setThemeMode('dark')}>{t.themeDark()}</button
                  >
                </div>
              </div>

              <div class="theme-picker">
                <span class="setting-label">{t.colorTheme()}</span>
                <div class="theme-card-grid" role="group" aria-label={t.colorTheme()}>
                  {#each availableThemes as colorTheme}
                    <button
                      type="button"
                      class="theme-card"
                      class:active={draftSettings.colorThemeId === colorTheme.id}
                      aria-pressed={draftSettings.colorThemeId === colorTheme.id}
                      aria-label={getThemeDisplayName(colorTheme, interfaceLocale)}
                      on:click={() => setColorTheme(colorTheme.id)}
                    >
                      <span class="theme-card-preview" aria-hidden="true">
                        <span
                          class="theme-card-variant"
                          style={`--preview-bg: ${colorTheme.variants.light.preview.background}; --preview-surface: ${colorTheme.variants.light.preview.surface}; --preview-accent: ${colorTheme.variants.light.preview.accent}; --preview-fg: ${colorTheme.variants.light.preview.foreground}; --preview-radius: ${colorTheme.variants.light.styleTokens.radiusMd}; --preview-border-width: ${colorTheme.variants.light.styleTokens.borderWidth}; --preview-shadow: ${colorTheme.variants.light.styleTokens.shadowRaised}; --preview-space: ${colorTheme.variants.light.styleTokens.spaceMd}`}
                        ></span>
                        <span
                          class="theme-card-variant"
                          style={`--preview-bg: ${colorTheme.variants.dark.preview.background}; --preview-surface: ${colorTheme.variants.dark.preview.surface}; --preview-accent: ${colorTheme.variants.dark.preview.accent}; --preview-fg: ${colorTheme.variants.dark.preview.foreground}; --preview-radius: ${colorTheme.variants.dark.styleTokens.radiusMd}; --preview-border-width: ${colorTheme.variants.dark.styleTokens.borderWidth}; --preview-shadow: ${colorTheme.variants.dark.styleTokens.shadowRaised}; --preview-space: ${colorTheme.variants.dark.styleTokens.spaceMd}`}
                        ></span>
                      </span>
                      <span class="theme-card-footer">
                        <span>{getThemeDisplayName(colorTheme, interfaceLocale)}</span>
                        {#if draftSettings.colorThemeId === colorTheme.id}
                          <span class="theme-card-check" aria-hidden="true">✓</span>
                        {/if}
                      </span>
                    </button>
                  {/each}
                </div>
              </div>

              <h2>{t.visualZoom()}</h2>
              <div class="setting-row">
                <div>
                  <label for="zoomPercent" class="setting-label">{t.zoomLevel()}</label>
                  <p>{t.zoomLevelDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="zoomPercent"
                    type="range"
                    min="80"
                    max="160"
                    step="5"
                    value={draftSettings.zoomPercent}
                    on:input={(event) => updateNumberSetting('zoomPercent', event)}
                  />
                  <output for="zoomPercent">{draftSettings.zoomPercent}%</output>
                </div>
              </div>

              <label class="toggle-row" for="ctrlWheelZoomEnabled">
                <span>
                  <span class="toggle-title">{t.ctrlWheelZoom()}</span>
                  <span class="toggle-desc">{t.ctrlWheelZoomDescription()}</span>
                </span>
                <input
                  id="ctrlWheelZoomEnabled"
                  type="checkbox"
                  checked={draftSettings.ctrlWheelZoomEnabled}
                  on:change={(event) => toggleSetting('ctrlWheelZoomEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>
            </div>
          {:else if activeCategory === 'files'}
            <div class="settings-group">
              <h2>{t.filesAndWindows()}</h2>
              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.folderOpenDefaultBehavior()}</span>
                  <p>{t.folderOpenDefaultBehaviorDescription()}</p>
                </div>
                <div class="triple-control" role="group" aria-label={t.folderOpenDefaultBehavior()}>
                  <button
                    type="button"
                    class:active={draftSettings.openDefaultBehavior === 'ask-every-time'}
                    aria-pressed={draftSettings.openDefaultBehavior === 'ask-every-time'}
                    on:click={() => setOpenBehavior('ask-every-time')}>{t.askEveryTime()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.openDefaultBehavior === 'current-window'}
                    aria-pressed={draftSettings.openDefaultBehavior === 'current-window'}
                    on:click={() => setOpenBehavior('current-window')}>{t.currentWindow()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.openDefaultBehavior === 'new-window'}
                    aria-pressed={draftSettings.openDefaultBehavior === 'new-window'}
                    on:click={() => setOpenBehavior('new-window')}>{t.newWindow()}</button
                  >
                </div>
              </div>

              <label class="toggle-row" for="filePreviewEnabled">
                <span>
                  <span class="toggle-title">{t.filePreviewTab()}</span>
                  <span class="toggle-desc">{t.filePreviewTabDescription()}</span>
                </span>
                <input
                  id="filePreviewEnabled"
                  type="checkbox"
                  checked={draftSettings.filePreviewEnabled}
                  on:change={(event) => toggleSetting('filePreviewEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <label class="toggle-row" for="sidebarHidden">
                <span>
                  <span class="toggle-title">{t.hideExplorerOnLaunch()}</span>
                  <span class="toggle-desc">{t.hideExplorerOnLaunchDescription()}</span>
                </span>
                <input
                  id="sidebarHidden"
                  type="checkbox"
                  checked={draftSettings.sidebarHidden}
                  on:change={(event) => toggleSetting('sidebarHidden', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.closeWindowBehavior()}</span>
                  <p>{t.closeWindowBehaviorDescription()}</p>
                </div>
                <div class="triple-control" role="group" aria-label={t.closeWindowBehavior()}>
                  <button
                    type="button"
                    class:active={draftSettings.closeWindowBehavior === 'ask-every-time'}
                    aria-pressed={draftSettings.closeWindowBehavior === 'ask-every-time'}
                    on:click={() => setCloseWindowBehavior('ask-every-time')}
                    >{t.closeWindowBehaviorAskEveryTime()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.closeWindowBehavior === 'close-window'}
                    aria-pressed={draftSettings.closeWindowBehavior === 'close-window'}
                    on:click={() => setCloseWindowBehavior('close-window')}
                    >{t.closeWindowBehaviorCloseWindow()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.closeWindowBehavior === 'close-to-tray'}
                    aria-pressed={draftSettings.closeWindowBehavior === 'close-to-tray'}
                    on:click={() => setCloseWindowBehavior('close-to-tray')}
                    >{t.closeWindowBehaviorCloseToTray()}</button
                  >
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.externalFileChangeBehavior()}</span>
                  <p>{t.externalFileChangeBehaviorDescription()}</p>
                </div>
                <div
                  class="triple-control"
                  role="group"
                  aria-label={t.externalFileChangeBehavior()}
                >
                  <button
                    type="button"
                    class:active={draftSettings.externalFileChangeBehavior === 'reload-external'}
                    aria-pressed={draftSettings.externalFileChangeBehavior === 'reload-external'}
                    on:click={() => setExternalFileChangeBehavior('reload-external')}
                    >{t.externalFileChangeBehaviorReload()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.externalFileChangeBehavior === 'overwrite-external'}
                    aria-pressed={draftSettings.externalFileChangeBehavior === 'overwrite-external'}
                    on:click={() => setExternalFileChangeBehavior('overwrite-external')}
                    >{t.externalFileChangeBehaviorOverwrite()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.externalFileChangeBehavior === 'ignore'}
                    aria-pressed={draftSettings.externalFileChangeBehavior === 'ignore'}
                    on:click={() => setExternalFileChangeBehavior('ignore')}
                    >{t.externalFileChangeBehaviorIgnore()}</button
                  >
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.bindMdDefaultApp()}</span>
                  <p>{mdAssociationDesc}</p>
                </div>
                <div class="association-action">
                  <span class={`association-pill ${mdAssociationPillClass}`}>
                    {mdAssociationLabel}
                  </span>
                  <button
                    type="button"
                    class="action-button"
                    disabled={!desktopEnabled ||
                      !platformCapabilities.isWindows ||
                      bindingMdAssociation ||
                      unbindingMdAssociation ||
                      checkingMdAssociation}
                    on:click={() => {
                      if (
                        !mdAssociationStatus?.managedByPackage &&
                        (mdAssociationStatus?.is_default || mdAssociationStatus?.registered)
                      ) {
                        void unbindMarkdownAssociation();
                      } else {
                        void bindMarkdownAssociation();
                      }
                    }}
                  >
                    {mdAssociationBtnLabel}
                  </button>
                </div>
              </div>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.registerMdContextMenu()}</span>
                  <p>{contextMenuDesc}</p>
                </div>
                <div class="association-action">
                  <span class={`association-pill ${contextMenuPillClass}`}>
                    {contextMenuLabel}
                  </span>
                  <button
                    type="button"
                    class="action-button"
                    disabled={!desktopEnabled ||
                      !platformCapabilities.isWindows ||
                      registeringContextMenu ||
                      unregisteringContextMenu ||
                      checkingContextMenu}
                    on:click={() => {
                      if (
                        contextMenuStatus?.managedByPackage
                          ? contextMenuStatus.enabled
                          : contextMenuStatus?.registered
                      ) {
                        void unregisterWindowsContextMenu();
                      } else {
                        void registerWindowsContextMenu();
                      }
                    }}
                  >
                    {contextMenuBtnLabel}
                  </button>
                </div>
              </div>
            </div>
          {:else if activeCategory === 'images'}
            <div class="settings-group">
              <h2>{t.imageImport()}</h2>
              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.imageHandlingStrategy()}</span>
                  <p>{t.imageHandlingStrategyDescription()}</p>
                </div>
                <div class="quad-control" role="group" aria-label={t.imageHandlingStrategy()}>
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-current-folder'}
                    aria-pressed={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-current-folder'}
                    on:click={() => setImageStrategy('copy-current-folder')}
                    >{t.currentFolder()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-assets'}
                    aria-pressed={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-assets'}
                    on:click={() => setImageStrategy('copy-assets')}>{t.assetsFolder()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-document-assets'}
                    aria-pressed={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'copy-document-assets'}
                    on:click={() => setImageStrategy('copy-document-assets')}
                    >{t.documentAssetsFolder()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'upload'}
                    aria-pressed={draftSettings.imageHandlingSettings.imageInsertStrategy ===
                      'upload'}
                    on:click={() => setImageStrategy('upload')}>{t.upload()}</button
                  >
                </div>
              </div>

              <label class="toggle-row" for="autoDeleteUnusedLocalImages">
                <span>
                  <span class="toggle-title">{t.autoCleanLocalImages()}</span>
                  <span class="toggle-desc">{t.autoCleanLocalImagesDescription()}</span>
                </span>
                <input
                  id="autoDeleteUnusedLocalImages"
                  type="checkbox"
                  checked={draftSettings.imageHandlingSettings.autoDeleteUnusedLocalImages}
                  on:change={(event) => toggleImageSetting('autoDeleteUnusedLocalImages', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              {#if draftSettings.imageHandlingSettings.imageInsertStrategy === 'upload'}
                <div class="setting-row">
                  <div>
                    <span class="setting-label">{t.uploadProvider()}</span>
                    <p>{t.uploadProviderDescription()}</p>
                  </div>
                  <div class="segmented-control" role="group" aria-label={t.uploadProvider()}>
                    <button
                      type="button"
                      class:active={draftSettings.imageHandlingSettings.uploadProvider === 'picgo'}
                      aria-pressed={draftSettings.imageHandlingSettings.uploadProvider === 'picgo'}
                      on:click={() => setUploadProvider('picgo')}>PicGo</button
                    >
                    <button
                      type="button"
                      class:active={draftSettings.imageHandlingSettings.uploadProvider ===
                        'picgo-core'}
                      aria-pressed={draftSettings.imageHandlingSettings.uploadProvider ===
                        'picgo-core'}
                      on:click={() => setUploadProvider('picgo-core')}>PicGo-Core</button
                    >
                  </div>
                </div>

                {#if draftSettings.imageHandlingSettings.uploadProvider === 'picgo'}
                  <div class="setting-row">
                    <div>
                      <label for="picgoServerUrl" class="setting-label">{t.picgoServerUrl()}</label>
                      <p>{t.picgoServerUrlDescription()}</p>
                    </div>
                    <input
                      id="picgoServerUrl"
                      class="text-input"
                      type="url"
                      value={draftSettings.imageHandlingSettings.picgoServerUrl}
                      on:input={(event) => updateImageStringSetting('picgoServerUrl', event)}
                    />
                  </div>
                {:else}
                  <div class="setting-row">
                    <div>
                      <label for="picgoCoreCommand" class="setting-label"
                        >{t.picgoCoreCommand()}</label
                      >
                      <p>{t.picgoCoreCommandDescription()}</p>
                    </div>
                    <input
                      id="picgoCoreCommand"
                      class="text-input"
                      type="text"
                      value={draftSettings.imageHandlingSettings.picgoCoreCommand}
                      on:input={(event) => updateImageStringSetting('picgoCoreCommand', event)}
                    />
                  </div>
                  <div class="setting-row">
                    <div>
                      <label for="picgoCoreConfigPath" class="setting-label"
                        >{t.picgoCoreConfigPath()}</label
                      >
                      <p>{t.picgoCoreConfigPathDescription()}</p>
                    </div>
                    <input
                      id="picgoCoreConfigPath"
                      class="text-input"
                      type="text"
                      value={draftSettings.imageHandlingSettings.picgoCoreConfigPath}
                      on:input={(event) => updateImageStringSetting('picgoCoreConfigPath', event)}
                    />
                  </div>
                {/if}
                <div class="setting-row">
                  <div>
                    <span class="setting-label">{t.connectionTest()}</span>
                    <p>{t.connectionTestDescription()}</p>
                  </div>
                  <button
                    type="button"
                    class="action-button"
                    disabled={!desktopEnabled || picgoTesting}
                    on:click={testPicgoConnection}
                  >
                    {picgoTesting ? t.testing() : t.testConnection()}
                  </button>
                </div>
              {/if}

              <div class="setting-row">
                <div>
                  <label for="defaultImageWidth" class="setting-label"
                    >{t.imageDefaultWidth()}</label
                  >
                  <p>{t.imageDefaultWidthDescription()}</p>
                </div>
                <input
                  id="defaultImageWidth"
                  class="text-input compact"
                  type="text"
                  placeholder={t.emptyPlaceholder()}
                  value={draftSettings.imageHandlingSettings.defaultImageWidth}
                  on:input={(event) => updateImageStringSetting('defaultImageWidth', event)}
                />
              </div>
              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.imageDefaultAlign()}</span>
                  <p>{t.imageDefaultAlignDescription()}</p>
                </div>
                <div class="quad-control" role="group" aria-label={t.imageDefaultAlign()}>
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.defaultImageAlign === 'none'}
                    aria-pressed={draftSettings.imageHandlingSettings.defaultImageAlign === 'none'}
                    on:click={() => setImageDefaultAlign('none')}>{t.followText()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.defaultImageAlign === 'left'}
                    aria-pressed={draftSettings.imageHandlingSettings.defaultImageAlign === 'left'}
                    on:click={() => setImageDefaultAlign('left')}>{t.alignLeft()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.defaultImageAlign ===
                      'center'}
                    aria-pressed={draftSettings.imageHandlingSettings.defaultImageAlign ===
                      'center'}
                    on:click={() => setImageDefaultAlign('center')}>{t.alignCenter()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.imageHandlingSettings.defaultImageAlign === 'right'}
                    aria-pressed={draftSettings.imageHandlingSettings.defaultImageAlign === 'right'}
                    on:click={() => setImageDefaultAlign('right')}>{t.alignRight()}</button
                  >
                </div>
              </div>
            </div>
          {:else if activeCategory === 'stats'}
            <div class="settings-group">
              <h2>{t.statsAndNavigation()}</h2>
              <label class="toggle-row" for="outlineVisible">
                <span>
                  <span class="toggle-title">{t.showDocumentOutline()}</span>
                  <span class="toggle-desc">{t.showDocumentOutlineDescription()}</span>
                </span>
                <input
                  id="outlineVisible"
                  type="checkbox"
                  checked={draftSettings.outlineVisible}
                  on:change={(event) => toggleSetting('outlineVisible', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <label class="toggle-row" for="writingStatsVisible">
                <span>
                  <span class="toggle-title">{t.showDocumentStats()}</span>
                  <span class="toggle-desc">{t.showDocumentStatsDescription()}</span>
                </span>
                <input
                  id="writingStatsVisible"
                  type="checkbox"
                  checked={draftSettings.writingStatsVisible}
                  on:change={(event) => toggleSetting('writingStatsVisible', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.defaultStatsMetric()}</span>
                  <p>{t.defaultStatsMetricDescription()}</p>
                </div>
                <div class="quad-control" role="group" aria-label={t.defaultStatsMetric()}>
                  <button
                    type="button"
                    class:active={draftSettings.writingStatsMetric === 'lines'}
                    aria-pressed={draftSettings.writingStatsMetric === 'lines'}
                    on:click={() => setStatsMetric('lines')}>{t.lines()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.writingStatsMetric === 'words'}
                    aria-pressed={draftSettings.writingStatsMetric === 'words'}
                    on:click={() => setStatsMetric('words')}>{t.words()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.writingStatsMetric === 'visibleChars'}
                    aria-pressed={draftSettings.writingStatsMetric === 'visibleChars'}
                    on:click={() => setStatsMetric('visibleChars')}>{t.visibleChars()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.writingStatsMetric === 'chars'}
                    aria-pressed={draftSettings.writingStatsMetric === 'chars'}
                    on:click={() => setStatsMetric('chars')}>{t.chars()}</button
                  >
                </div>
              </div>

              <label class="toggle-row" for="readingTimeVisible">
                <span>
                  <span class="toggle-title">{t.readingTime()}</span>
                  <span class="toggle-desc">{t.readingTimeDescription()}</span>
                </span>
                <input
                  id="readingTimeVisible"
                  type="checkbox"
                  checked={draftSettings.readingTimeVisible}
                  on:change={(event) => toggleSetting('readingTimeVisible', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <label for="outlineDefaultExpandLevel" class="setting-label"
                    >{t.outlineDefaultExpandLevel()}</label
                  >
                  <p>{t.outlineDefaultExpandLevelDescription()}</p>
                </div>
                <div class="range-setting">
                  <input
                    id="outlineDefaultExpandLevel"
                    type="range"
                    min="1"
                    max="6"
                    step="1"
                    value={draftSettings.outlineDefaultExpandLevel}
                    on:input={(event) => updateNumberSetting('outlineDefaultExpandLevel', event)}
                  />
                  <output for="outlineDefaultExpandLevel"
                    >H{draftSettings.outlineDefaultExpandLevel}</output
                  >
                </div>
              </div>
            </div>
          {:else if activeCategory === 'advanced'}
            <div class="settings-group">
              <h2>{t.renderingMode()}</h2>
              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.renderMode()}</span>
                  <p>{t.renderModeDescription()}</p>
                </div>
                <div class="segmented-control" role="group" aria-label={t.renderMode()}>
                  <button
                    type="button"
                    class:active={draftSettings.renderMode === 'hardware'}
                    aria-pressed={draftSettings.renderMode === 'hardware'}
                    on:click={() => setRenderMode('hardware')}>{t.renderModeHardware()}</button
                  >
                  <button
                    type="button"
                    class:active={draftSettings.renderMode === 'software'}
                    aria-pressed={draftSettings.renderMode === 'software'}
                    on:click={() => setRenderMode('software')}>{t.renderModeSoftware()}</button
                  >
                </div>
              </div>

              <h2>{t.defaultInsertBehavior()}</h2>
              <div class="setting-row">
                <div>
                  <label for="defaultCodeBlockLanguage" class="setting-label"
                    >{t.defaultCodeBlockLanguage()}</label
                  >
                  <p>{t.defaultCodeBlockLanguageDescription()}</p>
                </div>
                <input
                  id="defaultCodeBlockLanguage"
                  class="text-input compact"
                  type="text"
                  spellcheck="false"
                  value={draftSettings.defaultCodeBlockLanguage}
                  on:input={(event) => updateStringSetting('defaultCodeBlockLanguage', event)}
                />
              </div>

              <div class="setting-row">
                <div>
                  <label for="defaultDiagramType" class="setting-label"
                    >{t.defaultDiagramType()}</label
                  >
                  <p>{t.defaultDiagramTypeDescription()}</p>
                </div>
                <select
                  id="defaultDiagramType"
                  class="select-input"
                  value={draftSettings.defaultDiagramType}
                  on:change={(event) => updateStringSetting('defaultDiagramType', event)}
                >
                  {#each DIAGRAM_TEMPLATES as template}
                    <option value={template.type}>{getDiagramTypeLabel(template.type)}</option>
                  {/each}
                </select>
              </div>

              <div class="shortcut-settings">
                <h2>{t.customShortcuts()}</h2>
                {#each shortcutItems as shortcut}
                  <div class="setting-row compact-row">
                    <div>
                      <label for={`shortcut-${shortcut.id}`} class="setting-label"
                        >{t[shortcut.labelKey]()}</label
                      >
                      <p>{t.shortcutDescription()}</p>
                    </div>
                    <input
                      id={`shortcut-${shortcut.id}`}
                      class="text-input compact"
                      type="text"
                      spellcheck="false"
                      value={draftSettings.shortcutPreferences[shortcut.id]}
                      on:input={(event) => updateShortcut(shortcut.id, event)}
                    />
                  </div>
                {/each}
              </div>

              <h2>{t.developerOptions()}</h2>
              <label class="toggle-row" for="developerMode">
                <span>
                  <span class="toggle-title">{t.developerMode()}</span>
                  <span class="toggle-desc">{t.developerModeDescription()}</span>
                </span>
                <input
                  id="developerMode"
                  type="checkbox"
                  checked={draftSettings.developerMode}
                  on:change={(event) => toggleSetting('developerMode', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="disabled-row" aria-disabled="true">
                <div>
                  <span class="setting-label">{t.exportSettings()}</span>
                  <p>{t.exportSettingsDescription()}</p>
                </div>
                <span class="disabled-pill">{t.futureVersionSupport()}</span>
              </div>
            </div>
          {:else if activeCategory === 'about'}
            <div class="settings-group about-group">
              <div class="about-identity">
                <div class="about-mark" aria-hidden="true">
                  <img class="logo-light" src={nomoLogoLight} alt="" draggable="false" />
                  <img class="logo-dark" src={nomoLogoDark} alt="" draggable="false" />
                </div>
                <div class="about-identity-copy">
                  <h2>Nomo</h2>
                  <span
                    class="about-version-badge"
                    aria-label={`${t.version()} ${packageInfo.version}`}
                  >
                    v{packageInfo.version}
                  </span>
                </div>
              </div>
              <dl>
                <div>
                  <dt>{t.positioning()}</dt>
                  <dd>{t.positioningDescription()}</dd>
                </div>
                <div>
                  <dt>{t.platformStrategy()}</dt>
                  <dd>{t.platformStrategyDescription()}</dd>
                </div>
              </dl>

              <div class="project-link-grid">
                <button
                  type="button"
                  class="project-link-card"
                  on:click={() => void openProjectLink(GITHUB_REPOSITORY_URL)}
                >
                  <span class="project-link-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" focusable="false">
                      <path
                        d="M10.226 17.284c-2.965-.36-5.054-2.493-5.054-5.256 0-1.123.404-2.336 1.078-3.144-.292-.741-.247-2.314.09-2.965.898-.112 2.111.36 2.83 1.01.853-.269 1.752-.404 2.853-.404 1.1 0 1.999.135 2.807.382.696-.629 1.932-1.1 2.83-.988.315.606.36 2.179.067 2.942.72.854 1.101 2 1.101 3.167 0 2.763-2.089 4.852-5.098 5.234.763.494 1.28 1.572 1.28 2.807v2.336c0 .674.561 1.056 1.235.786 4.066-1.55 7.255-5.615 7.255-10.646C23.5 6.188 18.334 1 11.978 1 5.62 1 .5 6.188.5 12.545c0 4.986 3.167 9.12 7.435 10.669.606.225 1.19-.18 1.19-.786V20.63a2.9 2.9 0 0 1-1.078.224c-1.483 0-2.359-.808-2.987-2.313-.247-.607-.517-.966-1.034-1.033-.27-.023-.359-.135-.359-.27 0-.27.45-.471.898-.471.652 0 1.213.404 1.797 1.235.45.651.921.943 1.483.943.561 0 .92-.202 1.437-.719.382-.381.674-.718.944-.943"
                      />
                    </svg>
                  </span>
                  <span class="project-link-copy">
                    <span class="project-link-title">{t.githubRepository()}</span>
                    <span class="project-link-description">{t.githubRepositoryDescription()}</span>
                  </span>
                  <svg
                    class="project-link-external"
                    viewBox="0 0 16 16"
                    aria-hidden="true"
                    focusable="false"
                  >
                    <path
                      d="M3.75 2h3.5a.75.75 0 0 1 0 1.5h-3.5a.25.25 0 0 0-.25.25v8.5c0 .138.112.25.25.25h8.5a.25.25 0 0 0 .25-.25v-3.5a.75.75 0 0 1 1.5 0v3.5A1.75 1.75 0 0 1 12.25 14h-8.5A1.75 1.75 0 0 1 2 12.25v-8.5C2 2.784 2.784 2 3.75 2Zm6.854-1h4.146a.25.25 0 0 1 .25.25v4.146a.25.25 0 0 1-.427.177L13.03 4.03 9.28 7.78a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042l3.75-3.75-1.543-1.543A.25.25 0 0 1 10.604 1Z"
                    />
                  </svg>
                </button>

                <button
                  type="button"
                  class="project-link-card"
                  on:click={() => void openProjectLink(GITHUB_ISSUE_URL)}
                >
                  <span class="project-link-icon issue" aria-hidden="true">
                    <svg viewBox="0 0 24 24" focusable="false">
                      <path
                        d="M12 1c6.075 0 11 4.925 11 11s-4.925 11-11 11S1 18.075 1 12 5.925 1 12 1ZM2.5 12a9.5 9.5 0 0 0 9.5 9.5 9.5 9.5 0 0 0 9.5-9.5A9.5 9.5 0 0 0 12 2.5 9.5 9.5 0 0 0 2.5 12Zm9.5 2a2 2 0 1 1-.001-3.999A2 2 0 0 1 12 14Z"
                      />
                    </svg>
                  </span>
                  <span class="project-link-copy">
                    <span class="project-link-title">{t.reportIssue()}</span>
                    <span class="project-link-description">{t.reportIssueDescription()}</span>
                  </span>
                  <svg
                    class="project-link-external"
                    viewBox="0 0 16 16"
                    aria-hidden="true"
                    focusable="false"
                  >
                    <path
                      d="M3.75 2h3.5a.75.75 0 0 1 0 1.5h-3.5a.25.25 0 0 0-.25.25v8.5c0 .138.112.25.25.25h8.5a.25.25 0 0 0 .25-.25v-3.5a.75.75 0 0 1 1.5 0v3.5A1.75 1.75 0 0 1 12.25 14h-8.5A1.75 1.75 0 0 1 2 12.25v-8.5C2 2.784 2.784 2 3.75 2Zm6.854-1h4.146a.25.25 0 0 1 .25.25v4.146a.25.25 0 0 1-.427.177L13.03 4.03 9.28 7.78a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042l3.75-3.75-1.543-1.543A.25.25 0 0 1 10.604 1Z"
                    />
                  </svg>
                </button>
              </div>

              <label class="toggle-row" for="softwareUpdateAutoCheckEnabled">
                <span>
                  <span class="toggle-title">{t.softwareUpdateAutoCheck()}</span>
                  <span class="toggle-desc">{t.softwareUpdateAutoCheckDescription()}</span>
                </span>
                <input
                  id="softwareUpdateAutoCheckEnabled"
                  type="checkbox"
                  checked={draftSettings.softwareUpdateAutoCheckEnabled}
                  disabled={softwareUpdateSnapshot.installationKind === 'store'}
                  on:change={(event) => toggleSetting('softwareUpdateAutoCheckEnabled', event)}
                />
                <span class="toggle-switch" aria-hidden="true"></span>
              </label>

              <div class="setting-row">
                <div>
                  <span class="setting-label">{t.updateCheck()}</span>
                  <p>{softwareUpdateDescription}</p>
                </div>
                <div class="association-action">
                  <span class={`association-pill ${softwareUpdatePillClass}`}>
                    {softwareUpdatePillLabel}
                  </span>
                  <button
                    type="button"
                    class="action-button"
                    disabled={softwareUpdateButtonDisabled}
                    on:click={handleSoftwareUpdateButton}
                  >
                    {#if updateBusy}
                      <span class="update-busy-spinner" aria-hidden="true"></span>
                    {/if}
                    {softwareUpdateButtonLabel}
                  </button>
                </div>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </section>
  </div>
{/key}

{#if softwareUpdateDialogOpen}
  <SoftwareUpdateDialog
    state={softwareUpdateSnapshot}
    onClose={() => (softwareUpdateDialogOpen = false)}
    onLater={() => (softwareUpdateDialogOpen = false)}
    onDownload={() => void downloadAvailableUpdate()}
    onInstall={() => void installDownloadedSoftwareUpdate()}
    onRetry={() => void checkForSoftwareUpdate()}
    onOpenStore={() => void openMicrosoftStoreProduct()}
  />
{/if}

<style>
  :global(html),
  :global(body) {
    overflow: hidden;
  }

  .settings-window-shell {
    width: 100vw;
    height: 100vh;
    min-height: 0;
    display: grid;
    grid-template-columns: 212px minmax(0, 1fr);
    background: var(--md-editor-bg);
    color: var(--md-editor-fg);
    font-family: var(--md-editor-font-body);
  }

  .settings-nav {
    border-right: 1px solid var(--md-editor-border);
    background: color-mix(in srgb, var(--md-editor-rail) 78%, var(--md-editor-bg));
    display: flex;
    flex-direction: column;
    min-height: 0;
    -webkit-user-select: none;
    user-select: none;
  }

  .settings-brand {
    height: 42px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    color: var(--md-editor-fg);
    font-size: var(--md-editor-ui-font-size);
    font-weight: 700;
    border-bottom: 1px solid var(--md-editor-border);
    user-select: none;
  }

  .settings-brand-logo {
    width: 22px;
    height: 22px;
    position: relative;
    flex: 0 0 22px;
    display: inline-grid;
    place-items: center;
    overflow: hidden;
    border-radius: 6px;
  }

  .settings-brand-logo img,
  .about-mark img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
    pointer-events: none;
  }

  .logo-dark {
    display: none;
  }

  :global(:root[data-theme='dark']) .logo-light {
    display: none;
  }

  :global(:root[data-theme='dark']) .logo-dark {
    display: block;
  }

  .settings-nav nav {
    display: grid;
    gap: 4px;
    padding: 12px 10px;
  }

  .settings-nav button {
    width: 100%;
    min-height: var(--md-editor-control-height-lg);
    display: flex;
    align-items: center;
    gap: 10px;
    border: 0;
    border-radius: var(--md-editor-radius-sm);
    background: transparent;
    color: var(--md-editor-muted-fg);
    font: inherit;
    font-size: var(--md-editor-ui-font-size);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    transition:
      background-color 160ms ease,
      color 160ms ease;
  }

  .settings-nav button:hover {
    background: color-mix(in srgb, var(--md-editor-accent) 9%, transparent);
    color: var(--md-editor-fg);
  }

  .settings-nav button.active {
    background: var(--md-editor-sidebar-active);
    color: var(--md-editor-accent-strong);
  }

  .settings-main {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    min-width: 0;
    min-height: 0;
    background: var(--md-editor-bg);
  }

  .settings-header {
    height: 42px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 0 0 0 16px;
    border-bottom: 1px solid var(--md-editor-border);
    user-select: none;
  }

  .settings-header-title {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .settings-header h1 {
    margin: 0;
    color: var(--md-editor-heading-fg);
    font-size: 16px;
    line-height: 1.2;
    font-weight: 700;
    letter-spacing: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .settings-header-title span {
    flex-shrink: 0;
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
    line-height: 1.2;
    opacity: 0;
    transition: opacity 160ms ease;
  }

  .settings-header-title span.visible {
    opacity: 1;
  }

  .close-button {
    width: var(--md-editor-control-height-md);
    height: var(--md-editor-control-height-md);
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--md-editor-radius-sm);
    background: transparent;
    color: var(--md-editor-muted-fg);
    cursor: pointer;
    margin-right: 14px;
  }

  .close-button:hover {
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
  }

  .settings-content,
  .settings-loading {
    min-height: 0;
    overflow-y: auto;
    padding: var(--md-editor-space-xl);
    -webkit-user-select: none;
    user-select: none;
  }

  .settings-loading {
    color: var(--md-editor-muted-fg);
    font-size: 14px;
  }

  .settings-group {
    display: grid;
    gap: 0;
    max-width: 740px;
  }

  .settings-group h2 {
    margin: 0;
    padding: 0 0 12px;
    color: var(--md-editor-heading-fg);
    font-size: var(--md-editor-ui-font-size);
    font-weight: 750;
    letter-spacing: 0;
  }

  .settings-group h2:not(:first-child) {
    margin-top: 28px;
  }

  .settings-group h2.settings-heading-with-badge {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .beta-badge {
    display: inline-flex;
    align-items: center;
    min-height: 18px;
    padding: 1px 6px;
    border: 1px solid color-mix(in srgb, var(--md-editor-accent) 42%, var(--md-editor-border));
    border-radius: 999px;
    color: var(--md-editor-accent);
    background: color-mix(in srgb, var(--md-editor-accent) 8%, var(--md-editor-surface));
    font-size: 10px;
    font-weight: 750;
    line-height: 1;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .appearance-theme-row,
  .theme-picker {
    display: grid;
    gap: var(--md-editor-space-md);
    padding: var(--md-editor-space-md) 0;
    border-top: 1px solid color-mix(in srgb, var(--md-editor-border) 72%, transparent);
  }

  .appearance-theme-row {
    grid-template-columns: minmax(180px, 1fr) minmax(260px, 300px);
    align-items: center;
  }

  .theme-card-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--md-editor-space-md);
  }

  .theme-card {
    min-width: 0;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color 160ms ease,
      box-shadow 160ms ease,
      transform 160ms ease;
  }

  .theme-card:hover {
    border-color: color-mix(in srgb, var(--md-editor-accent) 55%, var(--md-editor-border));
    transform: translateY(-1px);
  }

  .theme-card.active {
    border-color: var(--md-editor-accent);
    box-shadow: 0 0 0 1px var(--md-editor-accent);
  }

  .theme-card-preview {
    height: 74px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-bottom: 1px solid var(--md-editor-border);
  }

  .theme-card-variant {
    position: relative;
    display: block;
    background: var(--preview-bg);
  }

  .theme-card-variant::before {
    content: '';
    position: absolute;
    inset: var(--preview-space, 12px);
    border: var(--preview-border-width, 1px) solid
      color-mix(in srgb, var(--preview-fg) 15%, transparent);
    border-radius: var(--preview-radius, 4px);
    background: var(--preview-surface);
    box-shadow: var(--preview-shadow, none);
  }

  .theme-card-variant::after {
    content: '';
    position: absolute;
    left: calc(var(--preview-space, 12px) + 6px);
    right: calc(var(--preview-space, 12px) + 6px);
    bottom: 20px;
    height: 5px;
    border-radius: 99px;
    background: var(--preview-accent);
    box-shadow: 0 -13px 0 -1px var(--preview-fg);
  }

  .theme-card-footer {
    min-height: 42px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 0 12px;
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 650;
  }

  .theme-card-check {
    width: 18px;
    height: 18px;
    display: inline-grid;
    place-items: center;
    flex: 0 0 18px;
    border-radius: 50%;
    background: var(--md-editor-accent);
    color: var(--md-editor-on-accent);
    font-size: 11px;
    line-height: 1;
  }

  .setting-row,
  .toggle-row,
  .disabled-row {
    min-height: 64px;
    display: grid;
    grid-template-columns: minmax(220px, 1fr) minmax(220px, 300px);
    align-items: center;
    gap: var(--md-editor-space-xl);
    padding: var(--md-editor-space-md) 0;
    border-top: 1px solid color-mix(in srgb, var(--md-editor-border) 72%, transparent);
  }

  .compact-row {
    min-height: 54px;
    padding: 10px 0;
  }

  .shortcut-settings {
    display: grid;
    gap: 0;
    padding-top: 16px;
  }

  .toggle-row {
    cursor: pointer;
    position: relative;
  }

  .setting-label,
  .toggle-title {
    display: block;
    color: var(--md-editor-fg);
    font-size: var(--md-editor-ui-font-size);
    font-weight: 650;
    line-height: 1.35;
  }

  .setting-row p,
  .disabled-row p,
  .toggle-desc {
    display: block;
    margin: 4px 0 0;
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
    line-height: 1.45;
  }

  .setting-row.setting-disabled {
    opacity: 0.55;
  }

  .segmented-control,
  .triple-control,
  .quad-control {
    display: grid;
    gap: 4px;
    padding: 4px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: color-mix(in srgb, var(--md-editor-surface) 82%, var(--md-editor-bg));
  }

  .segmented-control {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .triple-control {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .quad-control {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .segmented-control button,
  .triple-control button,
  .quad-control button {
    min-width: 0;
    min-height: var(--md-editor-control-height-md);
    border: 0;
    border-radius: var(--md-editor-radius-sm);
    background: transparent;
    color: var(--md-editor-muted-fg);
    font: inherit;
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 650;
    cursor: pointer;
    transition:
      background-color 160ms ease,
      color 160ms ease,
      box-shadow 160ms ease;
  }

  .segmented-control button:hover,
  .triple-control button:hover,
  .quad-control button:hover {
    background: color-mix(in srgb, var(--md-editor-accent) 8%, transparent);
    color: var(--md-editor-fg);
  }

  .segmented-control button:disabled {
    cursor: not-allowed;
  }

  .segmented-control button:disabled:hover {
    background: transparent;
    color: var(--md-editor-muted-fg);
  }

  .segmented-control button.active,
  .triple-control button.active,
  .quad-control button.active {
    background: var(--md-editor-bg);
    color: var(--md-editor-accent-strong);
    box-shadow: var(--md-editor-shadow-raised);
  }

  .range-setting {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 74px;
    align-items: center;
    gap: var(--md-editor-space-md);
  }

  .range-setting input {
    width: 100%;
    accent-color: var(--md-editor-accent);
  }

  .range-setting output,
  .number-field span {
    color: var(--md-editor-muted-fg);
    font-family: var(--md-editor-font-mono);
    font-size: var(--md-editor-ui-font-size-sm);
    text-align: right;
  }

  .number-field {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
  }

  .text-input,
  .select-input,
  .number-field input {
    width: 100%;
    min-width: 0;
    height: var(--md-editor-control-height-md);
    padding: 0 10px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-sm);
    background: var(--md-editor-surface);
    color: var(--md-editor-fg);
    font: inherit;
    font-size: var(--md-editor-ui-font-size);
    outline: none;
  }

  .text-input.compact {
    max-width: 180px;
    justify-self: end;
  }

  .action-button {
    justify-self: end;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 116px;
    height: var(--md-editor-control-height-md);
    padding: 0 12px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-sm);
    background: var(--md-editor-accent-fill);
    color: var(--md-editor-on-accent);
    font: inherit;
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 700;
    cursor: pointer;
    transition:
      opacity 160ms ease,
      transform 160ms ease,
      background-color 160ms ease;
  }

  .action-button:hover:not(:disabled) {
    opacity: 0.92;
  }

  .action-button:active:not(:disabled) {
    transform: translateY(1px);
  }

  .action-button:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .update-busy-spinner {
    width: 12px;
    height: 12px;
    border: 2px solid color-mix(in srgb, currentColor 25%, transparent);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: update-busy-spin 700ms linear infinite;
  }

  @keyframes update-busy-spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  .association-action {
    justify-self: end;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    min-width: 0;
  }

  .association-pill {
    min-width: 62px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 9px;
    border: 1px solid var(--md-editor-border);
    border-radius: 999px;
    color: var(--md-editor-muted-fg);
    background: color-mix(in srgb, var(--md-editor-surface) 78%, transparent);
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 750;
    line-height: 1;
    white-space: nowrap;
  }

  .association-pill.bound {
    border-color: color-mix(in srgb, var(--md-editor-success) 58%, var(--md-editor-border));
    color: var(--md-editor-success);
    background: color-mix(in srgb, var(--md-editor-success) 6%, var(--md-editor-surface));
  }

  .association-pill.pending {
    border-color: color-mix(in srgb, var(--md-editor-warning) 58%, var(--md-editor-border));
    color: var(--md-editor-warning);
    background: color-mix(in srgb, var(--md-editor-warning) 6%, var(--md-editor-surface));
  }

  .association-pill.error {
    border-color: color-mix(in srgb, var(--md-editor-danger) 58%, var(--md-editor-border));
    color: var(--md-editor-danger);
    background: color-mix(in srgb, var(--md-editor-danger) 6%, var(--md-editor-surface));
  }

  .toggle-row input {
    position: absolute;
    top: 50%;
    right: 0;
    width: 42px;
    height: 24px;
    margin: 0;
    transform: translateY(-50%);
    opacity: 0;
    pointer-events: none;
  }

  .toggle-switch {
    justify-self: end;
    position: relative;
    width: 42px;
    height: 24px;
    border-radius: 999px;
    border: 1px solid var(--md-editor-border);
    background: color-mix(in srgb, var(--md-editor-muted-fg) 16%, var(--md-editor-bg));
    transition:
      background-color 160ms ease,
      border-color 160ms ease;
  }

  .toggle-switch::before {
    content: '';
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    background: var(--md-editor-bg);
    box-shadow: var(--md-editor-shadow-raised);
    transition: transform 160ms ease;
  }

  .toggle-row input:checked + .toggle-switch {
    border-color: var(--md-editor-accent);
    background: var(--md-editor-accent);
  }

  .toggle-row input:checked + .toggle-switch::before {
    transform: translateX(18px);
  }

  .disabled-row {
    opacity: 0.76;
  }

  .disabled-pill {
    justify-self: end;
    padding: 5px 9px;
    border: 1px solid var(--md-editor-border);
    border-radius: 999px;
    color: var(--md-editor-muted-fg);
    background: color-mix(in srgb, var(--md-editor-surface) 72%, transparent);
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 650;
    white-space: nowrap;
  }

  .about-group {
    max-width: 640px;
  }

  .about-identity {
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 0 0 14px;
  }

  .about-mark {
    width: 54px;
    height: 54px;
    flex: 0 0 auto;
    position: relative;
    display: inline-grid;
    place-items: center;
    border-radius: var(--md-editor-radius-md);
    overflow: hidden;
    background: color-mix(in srgb, var(--md-editor-surface) 88%, var(--md-editor-accent));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--md-editor-border) 72%, transparent);
  }

  .about-identity-copy {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 9px;
  }

  .about-identity h2 {
    padding: 0;
    font-size: 18px;
    line-height: 1.2;
  }

  .about-version-badge {
    display: inline-flex;
    align-items: center;
    min-height: 20px;
    padding: 1px 7px;
    border: 1px solid color-mix(in srgb, var(--md-editor-accent) 34%, var(--md-editor-border));
    border-radius: 999px;
    color: var(--md-editor-accent);
    background: color-mix(in srgb, var(--md-editor-accent) 7%, var(--md-editor-surface));
    font-size: 11px;
    font-weight: 700;
    line-height: 1;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }

  .about-group dl {
    display: grid;
    gap: 0;
    margin: 0;
  }

  .about-group dl > div {
    display: grid;
    grid-template-columns: minmax(68px, max-content) minmax(0, 1fr);
    gap: 12px;
    padding: 13px 0;
    border-top: 1px solid color-mix(in srgb, var(--md-editor-border) 72%, transparent);
  }

  .about-group dt {
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
    font-weight: 650;
  }

  .about-group dd {
    margin: 0;
    color: var(--md-editor-fg);
    font-size: var(--md-editor-ui-font-size);
    line-height: 1.55;
  }

  .project-link-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin: 16px 0 8px;
  }

  .project-link-card {
    min-width: 0;
    min-height: 76px;
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr) 16px;
    align-items: center;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--md-editor-border);
    border-radius: var(--md-editor-radius-md);
    background: color-mix(in srgb, var(--md-editor-surface) 88%, var(--md-editor-bg));
    color: var(--md-editor-fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color 160ms ease,
      background-color 160ms ease,
      box-shadow 160ms ease;
  }

  .project-link-card:hover {
    border-color: color-mix(in srgb, var(--md-editor-accent) 48%, var(--md-editor-border));
    background: color-mix(in srgb, var(--md-editor-accent) 7%, var(--md-editor-surface));
  }

  .project-link-card:active {
    background: color-mix(in srgb, var(--md-editor-accent) 12%, var(--md-editor-surface));
  }

  .project-link-icon {
    width: 36px;
    height: 36px;
    display: inline-grid;
    place-items: center;
    border: 1px solid color-mix(in srgb, var(--md-editor-border) 72%, transparent);
    border-radius: 50%;
    background: var(--md-editor-bg);
    color: var(--md-editor-fg);
  }

  .project-link-icon.issue {
    color: var(--md-editor-success);
  }

  .project-link-icon svg {
    width: 24px;
    height: 24px;
    fill: currentColor;
  }

  .project-link-copy {
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .project-link-title {
    color: var(--md-editor-fg);
    font-size: var(--md-editor-ui-font-size);
    font-weight: 700;
    line-height: 1.3;
  }

  .project-link-description {
    color: var(--md-editor-muted-fg);
    font-size: var(--md-editor-ui-font-size-sm);
    line-height: 1.4;
  }

  .project-link-external {
    width: 16px;
    height: 16px;
    align-self: start;
    color: var(--md-editor-muted-fg);
    fill: currentColor;
  }

  button:focus-visible,
  input:focus-visible,
  select:focus-visible,
  .toggle-row input:focus-visible + .toggle-switch {
    outline: 2px solid var(--md-editor-accent);
    outline-offset: 2px;
  }

  @media (max-width: 760px) {
    .settings-window-shell {
      grid-template-columns: 64px minmax(0, 1fr);
    }

    .settings-brand span,
    .settings-nav button span {
      display: none;
    }

    .settings-brand,
    .settings-nav button {
      justify-content: center;
      padding-left: 0;
      padding-right: 0;
    }

    .setting-row,
    .toggle-row,
    .disabled-row,
    .appearance-theme-row {
      grid-template-columns: minmax(0, 1fr);
      gap: 10px;
    }

    .association-action {
      justify-self: stretch;
      justify-content: space-between;
    }

    .settings-content {
      padding: var(--md-editor-space-lg);
    }
  }

  @media (max-width: 520px) {
    .project-link-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .theme-card-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .theme-card {
      transition: none;
    }

    .theme-card:hover {
      transform: none;
    }
  }
</style>
