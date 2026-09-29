import type { NativeDocument, RecentEntry } from '../../lib/desktop/tauriStorage';
import type { EditorCore } from '../../lib/editor-core';
import { calculateDocumentStats } from '../../lib/outline/outlineService';
import { createEmptyExternalFileChange, type ExternalFileChangeState, type Tab } from '../types';
import { getDirectoryLabel, sameNativePath } from '../utils/pathLabels';
import {
  exportMarkdownInBrowser,
  findDroppedMarkdownPath,
  getExternalFileChange,
  loadRecentEntries,
  openMarkdownFromDialog,
  readMarkdownFromPath,
  rememberNativeDocument,
  saveNativeMarkdownFile,
} from './documentFiles';
import { normalizeMarkdownForSave } from '../../lib/markdown/normalize';
import { logInfo } from '../../lib/services/logger';
import {
  DEFAULT_MARKDOWN_ENCODING,
  normalizeMarkdownEncoding,
  type MarkdownEncoding,
} from '../../lib/services/storage';
import { confirmAction } from './confirmAction';
import {
  createBlankTab,
  getNativeDocumentTargetTab,
  getDocumentKindFromPath,
  getOrCreateReusableTab,
  isMarkdownTab,
} from './tabs';
import { t } from '../i18n';

// 从 Markdown 中提取第一个 H1 标题，生成建议文件名（清理非法字符）
function suggestFileNameFromH1(markdown: string, fallbackName: string): string {
  const match = markdown.match(/^#\s+(.+)$/m);
  if (!match) return fallbackName;

  let name = match[1].trim().replace(/[<>:"\/\\|?*]/g, '');
  if (!name) return fallbackName;

  return name.endsWith('.md') ? name : `${name}.md`;
}

function logCloseDiagnostics(message: string, data?: Record<string, unknown>) {
  logInfo('CloseGuard', message, data);
  // eslint-disable-next-line no-console
  console.info('[CloseGuard]', message, data ?? '');
}

function saveMarkdownWithSourceEncoding(
  path: string | null,
  markdown: string,
  fileName: string,
  snapshotPath: string | null,
  sourceEncoding: MarkdownEncoding | undefined,
) {
  const encoding = normalizeMarkdownEncoding(sourceEncoding);
  return encoding !== DEFAULT_MARKDOWN_ENCODING
    ? saveNativeMarkdownFile(path, markdown, fileName, snapshotPath, encoding)
    : saveNativeMarkdownFile(path, markdown, fileName, snapshotPath);
}

interface DocumentActionsOptions {
  recoveryKey: string;
  getLargeDocumentLimit(): number;
  getAutoSaveDelayMs(): number;
  getCreateSnapshotBeforeSave(): boolean;
  getDesktopEnabled(): boolean;
  getDirty(): boolean;
  getAutoSaveEnabled(): boolean;
  getNativePath(): string | null;
  setMarkdown(value: string): void;
  setSavedMarkdown(value: string): void;
  setNativePath(value: string | null): void;
  getFileName(): string;
  setFileName(value: string): void;
  getFilePath(): string;
  setFilePath(value: string): void;
  getLastKnownModifiedAt(): number;
  setLastKnownModifiedAt(value: number): void;
  getExternalFileChange(): ExternalFileChangeState;
  setExternalFileChange(value: ExternalFileChangeState): void;
  setDirty(value: boolean): void;
  setLargeDocumentMode(value: boolean): void;
  setReadonlyDocumentMode(value: boolean): void;
  setDiskReadonly(value: boolean): void;
  getCurrentFolderPath(): string;
  getFileInput(): HTMLInputElement;
  getEditor(): EditorCore;
  beforeMarkdownCommit?(): void;
  getTabs(): Tab[];
  setTabs(value: Tab[]): void;
  getActiveTabId(): string;
  setActiveTabId(value: string): void;
  getPreviewTabId(): string | null;
  setPreviewTabId(value: string | null): void;
  setStatusMessage(value: string): void;
  setRecentFiles(value: Awaited<ReturnType<typeof loadRecentEntries>>): void;
  saveActiveTabState(): void;
  loadTabState(tab: Tab): void;
  switchTab(tabId: string): void;
  writeRecoveryDraft(reason: string): void;
  updateWindowTitle(): void;
  loadFolder(folderPath: string): Promise<void>;
  expandAncestors(filePath: string, rootPath: string): void;
}

export function createDocumentActionsController(options: DocumentActionsOptions) {
  async function openDroppedMarkdown(paths: string[]) {
    const target = findDroppedMarkdownPath(paths);
    if (!target) {
      options.setStatusMessage(t.dragDropNoMarkdown());
      return;
    }
    if (options.getDirty()) {
      options.writeRecoveryDraft('drag-open-blocked');
      options.setStatusMessage(t.dragOpenBlockedUnsaved());
      return;
    }

    const { document, error } = await readMarkdownFromPath(target, t.dragOpenFailed());
    if (error) {
      options.setStatusMessage(error);
    }
    if (document) {
      await applyNativeDocument(document, t.openedByDragDrop());
    }
  }

  async function openFileDialog() {
    if (options.getDirty()) {
      options.writeRecoveryDraft('open-dialog');
    }
    if (options.getDesktopEnabled()) {
      const { document, error } = await openMarkdownFromDialog();
      if (error) {
        options.setStatusMessage(error);
      }
      if (document) {
        await applyNativeDocument(document, t.openedByTauri());
      }
      return;
    }

    options.getFileInput().click();
  }

  async function openMarkdownFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) {
      return;
    }
    if (getDocumentKindFromPath(file.name) !== 'markdown') {
      // 浏览器模式没有 Rust session，宁可拒绝也不能把 TXT/JSON 全文读入 WebView。
      options.setStatusMessage(t.unsupported());
      input.value = '';
      return;
    }

    const text = await file.text();
    options.saveActiveTabState();

    const browserFileTarget = getOrCreateReusableTab(options.getTabs(), options.getActiveTabId());
    options.setTabs(browserFileTarget.tabs);
    options.setActiveTabId(browserFileTarget.activeTabId);
    const targetTab = browserFileTarget.targetTab;

    targetTab.fileName = file.name;
    targetTab.filePath = t.localBrowserFile({ name: file.name });
    targetTab.nativePath = null;
    targetTab.draftId = null;
    targetTab.markdown = text;
    targetTab.savedMarkdown = text;
    targetTab.dirty = false;
    targetTab.lastKnownModifiedAt = 0;
    targetTab.largeDocumentMode = text.length > options.getLargeDocumentLimit();
    targetTab.readonlyDocumentMode = targetTab.largeDocumentMode;
    targetTab.diskReadonly = false;
    targetTab.externalFileChange = createEmptyExternalFileChange();

    options.setTabs([...options.getTabs()]);
    options.loadTabState(targetTab);

    options.setStatusMessage(t.markdownFileOpened());
    input.value = '';
  }

  async function saveMarkdownFile(saveAs = false): Promise<boolean> {
    const activeTab = options.getTabs().find((tab) => tab.id === options.getActiveTabId());
    if (!isMarkdownTab(activeTab)) {
      // TXT/JSON 由分段会话保存，绝不能回退到 EditorCore 的 Markdown 全量保存链路。
      return false;
    }
    if (activeTab.largeDocumentMode && !saveAs) {
      options.setStatusMessage(t.largeDocumentReadonlySaveBlocked());
      return false;
    }

    options.beforeMarkdownCommit?.();
    const markdownToSave = normalizeMarkdownForSave(options.getEditor().getMarkdown());

    if (options.getDesktopEnabled()) {
      const saveAsTarget = saveAs || activeTab.diskReadonly;
      if (!saveAsTarget && hasExternalFileChange()) {
        options.setStatusMessage(t.externalChangeChooseAction());
        return false;
      }
      if (!saveAs && activeTab.diskReadonly) {
        options.setStatusMessage(t.readonlySourceSaveAsRequired());
      }

      const path = saveAsTarget ? null : options.getNativePath();
      // 步骤1：新文件保存时，尝试用文档第一个 H1 标题作为建议文件名
      const fileName = path
        ? options.getFileName()
        : suggestFileNameFromH1(markdownToSave, options.getFileName());
      options.writeRecoveryDraft(saveAsTarget ? 'before-save-as' : 'before-save');
      const { document, error } = await saveMarkdownWithSourceEncoding(
        path,
        markdownToSave,
        fileName,
        options.getCreateSnapshotBeforeSave() ? options.getNativePath() : null,
        activeTab.encoding,
      );
      if (error) {
        options.setStatusMessage(error);
        return false;
      }
      if (document) {
        localStorage.removeItem(options.recoveryKey);
        await applySavedNativeDocument(document, markdownToSave, t.savedByTauri());
        return true;
      }
      return false;
    }

    // 浏览器模式下同样用 H1 作为建议文件名
    const fileName = suggestFileNameFromH1(markdownToSave, options.getFileName());
    exportMarkdownInBrowser(markdownToSave, fileName);
    options.setStatusMessage(t.markdownExported());
    options.setMarkdown(markdownToSave);
    options.setSavedMarkdown(markdownToSave);
    options.setDirty(false);
    options.getEditor().setDirty(false);
    const browserSavedTab = options.getTabs().find((tab) => tab.id === options.getActiveTabId());
    if (isMarkdownTab(browserSavedTab)) {
      browserSavedTab.markdown = markdownToSave;
      browserSavedTab.savedMarkdown = markdownToSave;
      browserSavedTab.dirty = false;
      options.setTabs([...options.getTabs()]);
    }
    return true;
  }

  async function openRecentFile(path: string) {
    if (!options.getDesktopEnabled()) {
      return;
    }

    const { document, error } = await readMarkdownFromPath(path, t.openRecentFailed());
    if (error) {
      options.setStatusMessage(error);
    }

    if (document) {
      await applyNativeDocument(document, t.recentFileOpened());
    }
  }

  async function applyNativeDocument(document: NativeDocument, message: string, saved = false) {
    const isLargeDocument =
      document.markdown.length > options.getLargeDocumentLimit() ||
      document.sizeBytes > options.getLargeDocumentLimit();
    const existingTab = options
      .getTabs()
      .find(
        (tab) =>
          isMarkdownTab(tab) && tab.nativePath && sameNativePath(tab.nativePath, document.path),
      );
    if (existingTab && !saved) {
      // 预览标签再次通过“正式打开”路径打开时，升级为固定标签。
      if (existingTab.id === options.getPreviewTabId()) {
        options.setPreviewTabId(null);
      }
      options.switchTab(existingTab.id);
      options.setStatusMessage(t.switchedToOpenedTab());
      return;
    }

    options.saveActiveTabState();

    const nativeDocumentTarget = getNativeDocumentTargetTab(
      options.getTabs(),
      options.getActiveTabId(),
      existingTab,
      saved,
    );
    options.setTabs(nativeDocumentTarget.tabs);
    options.setActiveTabId(nativeDocumentTarget.activeTabId);
    const targetTab = nativeDocumentTarget.targetTab;

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

    options.setActiveTabId(targetTab.id);
    options.setTabs([...options.getTabs()]);
    options.loadTabState(targetTab);

    options.setStatusMessage(message);
    await rememberNativeDocument(document, calculateDocumentStats(document.markdown).words);
    await refreshRecentFiles();
    if (isLargeDocument) {
      options.setStatusMessage(t.largeDocumentReadonlyOpened());
    }

    await revealDocumentInExplorer(document.path);
  }

  async function applySavedNativeDocument(
    document: NativeDocument,
    markdownToSave: string,
    message: string,
  ) {
    const previousPath = options
      .getTabs()
      .find((tab) => tab.id === options.getActiveTabId())?.nativePath;
    const isNewPath = !previousPath || !sameNativePath(previousPath, document.path);
    const isLargeDocument =
      document.markdown.length > options.getLargeDocumentLimit() ||
      document.sizeBytes > options.getLargeDocumentLimit();
    const existingTab = options
      .getTabs()
      .find(
        (tab) =>
          isMarkdownTab(tab) && tab.nativePath && sameNativePath(tab.nativePath, document.path),
      );

    options.saveActiveTabState();

    const nativeDocumentTarget = getNativeDocumentTargetTab(
      options.getTabs(),
      options.getActiveTabId(),
      existingTab,
      true,
    );
    options.setTabs(nativeDocumentTarget.tabs);
    options.setActiveTabId(nativeDocumentTarget.activeTabId);
    const targetTab = nativeDocumentTarget.targetTab;

    targetTab.fileName = document.fileName;
    targetTab.filePath = document.path;
    targetTab.nativePath = document.path;
    targetTab.draftId = null;
    targetTab.markdown = markdownToSave;
    targetTab.savedMarkdown = markdownToSave;
    targetTab.encoding = normalizeMarkdownEncoding(document.encoding);
    targetTab.dirty = false;
    targetTab.lastKnownModifiedAt = document.modifiedAt;
    targetTab.largeDocumentMode = isLargeDocument;
    targetTab.readonlyDocumentMode = isLargeDocument;
    targetTab.diskReadonly = document.readonly;
    targetTab.externalFileChange = createEmptyExternalFileChange();

    options.setFileName(targetTab.fileName);
    options.setFilePath(targetTab.filePath);
    options.setNativePath(targetTab.nativePath);
    options.setMarkdown(markdownToSave);
    options.setSavedMarkdown(markdownToSave);
    options.setDirty(false);
    options.getEditor().setDirty(false);
    options.setLastKnownModifiedAt(targetTab.lastKnownModifiedAt);
    options.setLargeDocumentMode(targetTab.largeDocumentMode);
    options.setReadonlyDocumentMode(targetTab.readonlyDocumentMode);
    options.setDiskReadonly(targetTab.diskReadonly);
    options.setExternalFileChange(targetTab.externalFileChange);
    options.setTabs([...options.getTabs()]);

    options.setStatusMessage(message);
    if (isNewPath) {
      await rememberNativeDocument(document, calculateDocumentStats(markdownToSave).words);
      await refreshRecentFiles();
    }
    if (isLargeDocument) {
      options.setStatusMessage(t.largeDocumentReadonlyOpened());
    }

    await revealDocumentInExplorer(document.path);
  }

  async function revealDocumentInExplorer(documentPath: string) {
    const parentDir = getDirectoryLabel(documentPath);
    if (!parentDir || parentDir === t.currentFolder()) {
      return;
    }

    const currentFolderPath = options.getCurrentFolderPath();
    if (!currentFolderPath) {
      await options.loadFolder(parentDir).catch(() => undefined);
      return;
    }

    options.expandAncestors(documentPath, currentFolderPath);
  }

  function createNewFile() {
    if (options.getDirty()) {
      options.writeRecoveryDraft('new-file-blocked');
      options.setStatusMessage(t.newFileWithRecovery());
    }

    options.saveActiveTabState();

    const newTab = createBlankTab();
    options.setTabs([...options.getTabs(), newTab]);
    options.setActiveTabId(newTab.id);
    options.loadTabState(newTab);
    options.updateWindowTitle();
  }

  async function closeTab(tabId: string, event?: Event, discardChanges = false) {
    event?.stopPropagation();
    const tabToClose = options.getTabs().find((tab) => tab.id === tabId);
    if (!tabToClose) {
      logCloseDiagnostics('documentActions.closeTab: 未找到目标标签', {
        tabId,
        activeTabId: options.getActiveTabId(),
        tabCount: options.getTabs().length,
      });
      return;
    }
    if (!isMarkdownTab(tabToClose)) {
      // 分段会话关闭前需要 flush journal/close session，由其工作区统一编排。
      return;
    }

    logCloseDiagnostics('documentActions.closeTab: 进入关闭流程', {
      tabId,
      activeTabId: options.getActiveTabId(),
      targetDirty: tabToClose.dirty,
      fileName: tabToClose.fileName,
      version: tabToClose.version,
      markdownLength: tabToClose.markdown.length,
      savedMarkdownLength: tabToClose.savedMarkdown?.length ?? null,
    });

    if (tabToClose.dirty && !discardChanges) {
      const message = t.confirmCloseModifiedFile();
      logCloseDiagnostics('documentActions.closeTab: 准备弹出未保存确认框', {
        tabId,
        fileName: tabToClose.fileName,
      });
      const confirmClose = await confirmAction(message, {
        title: tabToClose.fileName,
        okLabel: t.discardChanges(),
        cancelLabel: t.cancel(),
        saveLabel: tabToClose.nativePath ? t.save() : undefined,
      });
      logCloseDiagnostics('documentActions.closeTab: 未保存确认框返回', {
        tabId,
        confirmClose,
      });

      // cancel → 取消关闭
      if (confirmClose === false) return;

      // 用户选择保存后再关闭
      if (confirmClose === 'save') {
        const saved = await saveMarkdownFile(false);
        if (!saved) {
          logCloseDiagnostics('documentActions.closeTab: 保存失败或取消，停止关闭标签', {
            tabId,
            fileName: tabToClose.fileName,
          });
          return;
        }
      }
    } else {
      logCloseDiagnostics('documentActions.closeTab: 标签未标记 dirty，跳过确认', {
        tabId,
        fileName: tabToClose.fileName,
      });
    }

    const index = options.getTabs().findIndex((tab) => tab.id === tabId);
    const nextTabs = options.getTabs().filter((tab) => tab.id !== tabId);
    options.setTabs(nextTabs);

    if (options.getActiveTabId() === tabId) {
      if (nextTabs.length > 0) {
        const newActiveIndex = Math.min(index, nextTabs.length - 1);
        options.setActiveTabId(nextTabs[newActiveIndex].id);
        options.loadTabState(nextTabs[newActiveIndex]);
      } else {
        options.setActiveTabId('');
      }
    }
  }

  let recentRefreshGeneration = 0;
  async function refreshRecentFiles() {
    const generation = ++recentRefreshGeneration;
    const entries = await loadRecentEntries(options.getDesktopEnabled());
    if (generation === recentRefreshGeneration) options.setRecentFiles(entries);
  }

  async function reloadExternalFile() {
    const activeTab = options.getTabs().find((tab) => tab.id === options.getActiveTabId());
    if (!isMarkdownTab(activeTab)) {
      return;
    }
    const path = options.getNativePath();
    if (!options.getDesktopEnabled() || !path || !hasExternalFileChange()) {
      return;
    }

    const { document, error } = await readMarkdownFromPath(path, t.reloadExternalFailed());
    if (error) {
      options.setStatusMessage(error);
      return;
    }
    if (document) {
      await applyNativeDocument(document, t.reloadedExternalVersion(), true);
    }
  }

  async function overwriteExternalFile() {
    const activeTab = options.getTabs().find((tab) => tab.id === options.getActiveTabId());
    if (!isMarkdownTab(activeTab)) {
      return;
    }
    const path = options.getNativePath();
    if (!options.getDesktopEnabled() || !path) {
      return;
    }
    const externalChangeType = options.getExternalFileChange().type;
    if (activeTab.diskReadonly && externalChangeType !== 'deleted') {
      options.setStatusMessage(t.readonlySourceSaveAsRequired());
      return;
    }
    if (externalChangeType !== 'modified' && externalChangeType !== 'deleted') {
      options.setStatusMessage(t.noExternalChangeToOverwrite());
      return;
    }

    options.beforeMarkdownCommit?.();
    const markdownToSave = normalizeMarkdownForSave(options.getEditor().getMarkdown());
    options.writeRecoveryDraft('before-overwrite-external');
    const { document, error } = await saveMarkdownWithSourceEncoding(
      path,
      markdownToSave,
      options.getFileName(),
      options.getCreateSnapshotBeforeSave() ? path : null,
      activeTab.encoding,
    );
    if (error) {
      options.setStatusMessage(error);
    }
    if (document) {
      localStorage.removeItem(options.recoveryKey);
      await applySavedNativeDocument(document, markdownToSave, t.overwrittenExternalVersion());
    }
  }

  let saveTimers: Record<string, ReturnType<typeof setTimeout>> = {};

  function debouncedAutoSave(tabId: string) {
    if (!options.getAutoSaveEnabled()) return;

    const targetTab = options.getTabs().find((tab) => tab.id === tabId);
    if (!isMarkdownTab(targetTab) || !targetTab.nativePath) return;
    if (targetTab.diskReadonly) {
      cancelPendingAutoSave(tabId);
      if (options.getActiveTabId() === tabId) {
        options.setStatusMessage(t.readonlySourceAutoSavePaused());
      }
      return;
    }
    if (
      targetTab.externalFileChange.type !== 'none' ||
      (options.getActiveTabId() === tabId && hasExternalFileChange())
    ) {
      if (options.getActiveTabId() === tabId) {
        options.setStatusMessage(t.externalChangeAutoSavePaused());
      }
      return;
    }

    cancelPendingAutoSave(tabId);

    saveTimers[tabId] = setTimeout(async () => {
      delete saveTimers[tabId];
      await autoSaveLatestTab(tabId);
    }, options.getAutoSaveDelayMs());
  }

  async function autoSaveLatestTab(
    tabId: string,
    force: boolean = false,
    diskBaseline?: { markdown: string; modifiedAt: number },
  ) {
    if (!options.getAutoSaveEnabled() || !options.getDesktopEnabled()) {
      if (force && diskBaseline) {
        applyAutoSaveDiskBaseline(tabId, diskBaseline);
      }
      return;
    }

    flushActiveAutoSaveTab(tabId);
    const targetTab = options.getTabs().find((tab) => tab.id === tabId);
    if (!isMarkdownTab(targetTab) || !targetTab.nativePath) return;
    if (!force && !targetTab.dirty) return;
    const externalFileChangePending =
      targetTab.externalFileChange.type !== 'none' ||
      (options.getActiveTabId() === tabId && hasExternalFileChange());
    if (targetTab.diskReadonly || externalFileChangePending) {
      if (force && diskBaseline) {
        applyAutoSaveDiskBaseline(tabId, diskBaseline);
      }
      if (options.getActiveTabId() === tabId) {
        options.setStatusMessage(
          targetTab.diskReadonly
            ? t.readonlySourceAutoSavePaused()
            : t.externalChangeAutoSavePaused(),
        );
      }
      return;
    }

    const path = targetTab.nativePath;
    const fileName = targetTab.fileName;
    const markdownToSave = normalizeMarkdownForSave(targetTab.markdown);
    const { document, error } = await saveMarkdownWithSourceEncoding(
      path,
      markdownToSave,
      fileName,
      null,
      targetTab.encoding,
    );

    if (error) {
      if (force && diskBaseline) {
        applyAutoSaveDiskBaseline(tabId, diskBaseline);
      }
      if (options.getActiveTabId() === tabId) {
        options.setStatusMessage(t.autoSaveFailed({ error }));
      }
      return;
    }
    if (!document) return;

    flushActiveAutoSaveTab(tabId);
    const tabs = options.getTabs();
    const latestTab = tabs.find((tab) => tab.id === tabId);
    if (!isMarkdownTab(latestTab) || latestTab.nativePath !== path) return;

    const latestMarkdownToSave = normalizeMarkdownForSave(latestTab.markdown);
    if (latestMarkdownToSave !== markdownToSave) {
      latestTab.lastKnownModifiedAt = document.modifiedAt;
      if (options.getActiveTabId() === tabId) {
        options.setLastKnownModifiedAt(document.modifiedAt);
      }
      options.setTabs([...tabs]);
      cancelPendingAutoSave(tabId);
      await autoSaveLatestTab(tabId, true, {
        markdown: markdownToSave,
        modifiedAt: document.modifiedAt,
      });
      return;
    }

    latestTab.markdown = markdownToSave;
    latestTab.savedMarkdown = markdownToSave;
    latestTab.encoding = normalizeMarkdownEncoding(document.encoding);
    latestTab.dirty = false;
    latestTab.draftId = null;
    latestTab.lastKnownModifiedAt = document.modifiedAt;
    cancelPendingAutoSave(tabId);

    if (options.getActiveTabId() === tabId) {
      options.setDirty(false);
      options.getEditor().setDirty(false);
      options.setSavedMarkdown(markdownToSave);
      options.setLastKnownModifiedAt(document.modifiedAt);
      options.setStatusMessage(t.saved());
    }
    options.setTabs([...tabs]);
    // 自动保存只更新当前文件内容，不隐式重命名文件，避免用户未确认时改变磁盘路径。
  }

  function flushActiveAutoSaveTab(tabId: string) {
    if (options.getActiveTabId() === tabId) {
      options.getEditor().flushMarkdown();
    }
  }

  function applyAutoSaveDiskBaseline(
    tabId: string,
    diskBaseline: { markdown: string; modifiedAt: number },
  ) {
    const tabs = options.getTabs();
    const targetTab = tabs.find((tab) => tab.id === tabId);
    if (!isMarkdownTab(targetTab)) return;

    targetTab.savedMarkdown = diskBaseline.markdown;
    targetTab.dirty = normalizeMarkdownForSave(targetTab.markdown) !== diskBaseline.markdown;
    targetTab.lastKnownModifiedAt = diskBaseline.modifiedAt;

    if (options.getActiveTabId() === tabId) {
      options.setSavedMarkdown(diskBaseline.markdown);
      options.setDirty(targetTab.dirty);
      options.setLastKnownModifiedAt(diskBaseline.modifiedAt);
      if (targetTab.dirty) {
        options.getEditor().setMarkdown(targetTab.markdown, {
          reason: 'programmatic-update',
          dirty: true,
          savedMarkdown: diskBaseline.markdown,
        });
      }
    }
    options.setTabs([...tabs]);
  }

  function cancelPendingAutoSave(tabId: string) {
    const timer = saveTimers[tabId];
    if (timer === undefined) return;
    clearTimeout(timer);
    delete saveTimers[tabId];
  }

  function cancelPendingAutoSaves() {
    for (const timer of Object.values(saveTimers)) {
      clearTimeout(timer);
    }
    saveTimers = {};
  }

  async function checkExternalFileChange() {
    const nextChange = await getExternalFileChange(
      options.getDesktopEnabled(),
      options.getNativePath(),
      options.getLastKnownModifiedAt(),
      options.getDirty(),
    );
    options.setExternalFileChange(nextChange);
    if (nextChange.type !== 'none') {
      cancelPendingAutoSaves();
      if (options.getDirty()) {
        options.setStatusMessage(t.externalChangeAutoSavePaused());
      }
    }
  }

  function hasExternalFileChange() {
    return options.getExternalFileChange().type !== 'none';
  }

  return {
    openDroppedMarkdown,
    openFileDialog,
    openMarkdownFile,
    saveMarkdownFile,
    openRecentFile,
    applyNativeDocument,
    createNewFile,
    closeTab,
    refreshRecentFiles,
    reloadExternalFile,
    overwriteExternalFile,
    checkExternalFileChange,
    debouncedAutoSave,
    cancelPendingAutoSave,
    cancelPendingAutoSaves,
  };
}
