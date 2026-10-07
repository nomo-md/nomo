import type { MarkdownTabState, PersistedWorkspaceState, Tab, WorkspaceItem } from '../types';
import { sameNativePath } from '../utils/pathLabels';
import { isMarkdownTab } from './tabs';
import { createPersistedWorkspaceState } from './workspacePersistence';
import {
  createComparisonWorkspaceItem,
  createSingleWorkspaceItem,
  findWorkspaceItemForTab,
  getWorkspaceItemFocusedTabId,
  getWorkspaceItemTabIds,
  normalizeWorkspaceItems,
  removeWorkspaceItemMember,
} from './workspaceItems';
import {
  applyWindowWorkspaceEpoch,
  cancelTabTransfer,
  closeEmptyTransferredWindow,
  commitTabTransfer,
  prepareTabTransfer,
  queryTabTransfer,
  takeWorkspaceWriteStamp,
  targetReadyTabTransfer,
  type TabDropPlacement,
  type TabTransferSnapshot,
} from './tabTransfer';

interface LocalWorkspace {
  tabs: Tab[];
  items: WorkspaceItem[];
  activeTabId: string;
  activeItemId: string;
  currentFolderPath: string;
}

interface WorkspaceTransferOptions {
  getWindowLabel(): string;
  getWorkspace(): LocalWorkspace;
  flushWorkspace(): Promise<void>;
  capturePosition(tabId: string): unknown;
  setBusy(busy: boolean): void;
  applyWorkspace(workspace: LocalWorkspace, positions?: Record<string, unknown>): void;
  beforeCloseEmptySource?(state: TabTransferSnapshot): Promise<void>;
  showError(error: unknown): void;
}

function validateIncomingNativePaths(incoming: readonly Tab[], existing: readonly Tab[] = []) {
  for (let index = 0; index < incoming.length; index += 1) {
    const path = incoming[index].nativePath;
    if (!path) continue;
    if (incoming.slice(0, index).some((tab) => tab.nativePath && sameNativePath(tab.nativePath, path))) {
      throw new Error('移交成员不能指向同一文件');
    }
    if (existing.some((tab) => tab.nativePath && sameNativePath(tab.nativePath, path))) {
      throw new Error('目标窗口已经打开同一文件');
    }
  }
}

/** 一个外层项是一笔移交事务，确认磁盘提交前不改变任何可见文档的归属。 */
export function createWorkspaceTransferController(options: WorkspaceTransferOptions) {
  const sourceStages = new Map<string, LocalWorkspace>();
  const targetStages = new Map<string, LocalWorkspace>();
  const terminalResults = new Map<string, TabTransferSnapshot>();
  const cancelledTokens = new Set<string>();
  const applied = new Set<string>();
  const committing = new Set<string>();
  const applying = new Set<string>();
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const reconciling = new Set<string>();
  const reportedRecoveryErrors = new Set<string>();
  let preparing = false;
  let preparingTarget: string | undefined;
  let closingSource = false;
  let emptySourceCloseState: TabTransferSnapshot | undefined;
  let emptySourceCloseTimer: ReturnType<typeof setTimeout> | undefined;
  let emptySourceCloseInFlight = false;
  let emptySourceCloseRequested = false;
  let emptySourceCloseError: string | undefined;
  let disposed = false;

  function updateBusy() {
    if (!disposed) {
      options.setBusy(preparing || Boolean(preparingTarget) || closingSource ||
        sourceStages.size > 0 || targetStages.size > 0 || applying.size > 0);
    }
  }

  function isParticipant(state: TabTransferSnapshot) {
    const label = options.getWindowLabel();
    return state.sourceWindowLabel === label || state.targetWindowLabel === label;
  }

  async function closeEmptySource() {
    if (disposed || emptySourceCloseInFlight || emptySourceCloseRequested || !emptySourceCloseState) return;
    emptySourceCloseInFlight = true;
    try {
      await options.beforeCloseEmptySource?.(emptySourceCloseState);
      if (disposed) return;
      await closeEmptyTransferredWindow(emptySourceCloseState.token);
      emptySourceCloseRequested = true;
      if (emptySourceCloseTimer !== undefined) clearTimeout(emptySourceCloseTimer);
      emptySourceCloseTimer = undefined;
      // 内部关闭命令确认窗口已销毁后才成功，不再重放关闭请求。
    } catch (error) {
      if (disposed) return;
      const message = error instanceof Error ? error.message : String(error);
      if (emptySourceCloseError !== message) {
        emptySourceCloseError = message;
        options.showError(error);
      }
      if (emptySourceCloseTimer === undefined) {
        emptySourceCloseTimer = setTimeout(() => {
          emptySourceCloseTimer = undefined;
          void closeEmptySource();
        }, 5000);
      }
    } finally {
      emptySourceCloseInFlight = false;
      updateBusy();
    }
  }

  function openTargets(workspace: LocalWorkspace) {
    return {
      folderPath: workspace.currentFolderPath || null,
      filePaths: [...new Set(workspace.tabs.map((tab) => tab.nativePath)
        .filter((path): path is string => Boolean(path)))],
    };
  }

  async function persistable(workspace: LocalWorkspace): Promise<PersistedWorkspaceState> {
    const stamp = await takeWorkspaceWriteStamp();
    const state = await createPersistedWorkspaceState({
      ...workspace,
      desktopEnabled: true,
      draftWritePolicy: 'skip',
    });
    Object.assign(state, stamp);
    return state;
  }

  function stopTimer(token: string) {
    const timer = timers.get(token);
    if (timer !== undefined) clearTimeout(timer);
    timers.delete(token);
  }

  function hasStage(token: string) {
    return sourceStages.has(token) || targetStages.has(token) || preparingTarget === token;
  }

  // 仅重复查询未知结果。后台取消、commit 响应和事件都以同一 terminal 分发处理。
  function armTimer(token: string, delay = 15000) {
    stopTimer(token);
    if (disposed || applied.has(token) || !hasStage(token)) return;
    timers.set(token, setTimeout(() => {
      timers.delete(token);
      armTimer(token, 5000);
      void reconcile(token, true);
    }, delay));
  }

  async function settle(state: TabTransferSnapshot) {
    if (state.status === 'committed') await committed(state);
    else if (state.status === 'cancelled') cancelled(state);
  }

  async function reconcile(token: string, cancelPending = false) {
    if (disposed || applied.has(token) || reconciling.has(token)) return;
    reconciling.add(token);
    try {
      const observed = await queryTabTransfer(token);
      if (disposed) return;
      if (observed.status === 'committed' || observed.status === 'cancelled') {
        await settle(observed);
      } else if (observed.status === 'target-ready' && sourceStages.has(token)) {
        // 不等待可能丢失的 commit invoke 响应；计时器仍可独立查询已提交结果。
        void ready(observed);
      } else if (cancelPending) {
        await settle(await cancelTabTransfer(token));
      }
      reportedRecoveryErrors.delete(token);
    } catch (error) {
      if (!disposed && !reportedRecoveryErrors.has(token)) {
        reportedRecoveryErrors.add(token);
        options.showError(error);
      }
      // IPC 失败不能推断事务已取消；保留暂存和冻结状态，继续只读核对。
      if (hasStage(token)) armTimer(token, 5000);
    } finally {
      reconciling.delete(token);
    }
  }

  async function cancelAndSettle(token: string) {
    try {
      const observed = await queryTabTransfer(token);
      if (observed.status === 'committed' || observed.status === 'cancelled') {
        await settle(observed);
        return;
      }
      await settle(await cancelTabTransfer(token));
    } catch (error) {
      options.showError(error);
      await reconcile(token);
      if (hasStage(token)) armTimer(token, 5000);
    }
  }

  async function transfer(
    itemId: string,
    targetWindowLabel: string | undefined,
    placement: TabDropPlacement,
    screenPosition?: { x: number; y: number },
    memberId?: string,
  ) {
    if (disposed || preparing || preparingTarget || sourceStages.size || targetStages.size) return;
    preparing = true;
    updateBusy();
    let token: string | undefined;
    try {
      await options.flushWorkspace();
      if (disposed) return;
      const before = options.getWorkspace();
      const sourceItem = before.items.find((item) => item.id === itemId);
      if (!sourceItem || (memberId && !getWorkspaceItemTabIds(sourceItem).includes(memberId))) return;
      const item = memberId ? createSingleWorkspaceItem(memberId) : { ...sourceItem };
      const ids = getWorkspaceItemTabIds(item);
      const documents = ids.map((id) => before.tabs.find((tab) => tab.id === id)).filter(isMarkdownTab);
      if (documents.length !== ids.length) throw new Error('只能移动 Markdown 文档');
      validateIncomingNativePaths(documents);
      const afterItems = memberId
        ? removeWorkspaceItemMember(before.items, memberId)
        : before.items.filter((candidate) => candidate.id !== itemId);
      const afterTabs = before.tabs.filter((tab) => !ids.includes(tab.id));
      const sourceIndex = before.items.findIndex((candidate) => candidate.id === itemId);
      const activeOwner = findWorkspaceItemForTab(afterItems, before.activeTabId) ??
        afterItems[Math.min(sourceIndex, afterItems.length - 1)];
      const after: LocalWorkspace = {
        ...before,
        tabs: afterTabs,
        items: afterItems,
        activeTabId: activeOwner ? getWorkspaceItemFocusedTabId(activeOwner) : '',
        activeItemId: activeOwner?.id ?? '',
      };
      const positions = Object.fromEntries(ids.map((id) => [id, options.capturePosition(id)]));
      const sourceWorkspace = await persistable(after);
      if (disposed) return;
      const state = await prepareTabTransfer({
        item,
        tabs: structuredClone(documents) as MarkdownTabState[],
        positions,
        sourceWorkspace,
        sourceOpenTargets: openTargets(after),
        targetWindowLabel,
        placement,
        screenPosition,
      });
      token = state.token;
      if (disposed) { await cancelTabTransfer(token); return; }
      if (cancelledTokens.has(token)) return;
      sourceStages.set(token, after);
      armTimer(token);
      const terminal = terminalResults.get(token);
      if (terminal) await settle(terminal);
      else void reconcile(token);
    } catch (error) {
      if (token) await cancelAndSettle(token);
      if (!disposed) options.showError(error);
    } finally {
      preparing = false;
      updateBusy();
    }
  }

  async function prepared(state: TabTransferSnapshot) {
    if (disposed || state.targetWindowLabel !== options.getWindowLabel() || applied.has(state.token)) return;
    if (state.status === 'cancelled') { cancelled(state); return; }
    if (cancelledTokens.has(state.token)) return;
    if (preparingTarget === state.token || targetStages.has(state.token)) return;
    if (preparing || preparingTarget || sourceStages.size || targetStages.size) {
      await cancelAndSettle(state.token);
      return;
    }
    // 在首个 await 前预约并冻结，重复 prepared 不得并行 flush 或创建另一份组合。
    preparingTarget = state.token;
    updateBusy();
    armTimer(state.token);
    try {
      await options.flushWorkspace();
      if (disposed || cancelledTokens.has(state.token)) return;
      const before = options.getWorkspace();
      const members = getWorkspaceItemTabIds(state.item);
      if (state.tabs.length !== members.length || members.some((id) =>
        !state.tabs.some((tab) => tab.id === id && tab.documentKind === 'markdown' &&
          typeof tab.markdown === 'string' && typeof tab.savedMarkdown === 'string'))) {
        throw new Error('迁移文档快照不完整');
      }
      if (state.tabs.some((tab) => before.tabs.some((existing) => existing.id === tab.id))) {
        throw new Error('目标窗口已有同一文档');
      }
      validateIncomingNativePaths(state.tabs, before.tabs);
      const incomingTabs = structuredClone(state.tabs);
      const nextTabs = [...before.tabs, ...incomingTabs];
      let nextItems = [...before.items];
      let incomingItem: WorkspaceItem = { ...state.item };
      if (state.placement.kind === 'combine') {
        if (state.item.kind !== 'single') throw new Error('组合只能包含两篇文档');
        const targetId = state.placement.targetItemId;
        const target = nextItems.find((item) => item.id === targetId);
        const targetTab = target?.kind === 'single'
          ? before.tabs.find((tab) => tab.id === target.tabId) : undefined;
        if (target?.kind !== 'single' || !isMarkdownTab(targetTab)) throw new Error('目标标签不能组合');
        incomingItem = createComparisonWorkspaceItem(target.tabId, state.item.tabId,
          targetTab.largeDocumentMode || incomingTabs.some((tab) => tab.largeDocumentMode)
            ? 'source' : 'semantic');
        nextItems = nextItems.map((item) => item.id === targetId ? incomingItem : item);
      } else {
        const index = Math.max(0, Math.min(nextItems.length, state.placement.insertionIndex));
        nextItems.splice(index, 0, incomingItem);
      }
      nextItems = normalizeWorkspaceItems(nextTabs, nextItems);
      const activeItem = findWorkspaceItemForTab(nextItems, getWorkspaceItemFocusedTabId(incomingItem));
      if (!activeItem) throw new Error('迁入文档未分配工作区项');
      const staged: LocalWorkspace = {
        ...before,
        tabs: nextTabs,
        items: nextItems,
        activeTabId: getWorkspaceItemFocusedTabId(activeItem),
        activeItemId: activeItem.id,
      };
      targetStages.set(state.token, staged);
      // 后续接收 invoke 可能丢失响应；已建立的 stage 自身负责保持冻结。
      preparingTarget = undefined;
      const terminal = terminalResults.get(state.token);
      if (terminal) { await settle(terminal); return; }
      const targetWorkspace = await persistable(staged);
      if (disposed || cancelledTokens.has(state.token)) return;
      await settle(await targetReadyTabTransfer(state.token, targetWorkspace, openTargets(staged)));
    } catch (error) {
      await cancelAndSettle(state.token);
      if (!disposed) options.showError(error);
    } finally {
      if (preparingTarget === state.token) preparingTarget = undefined;
      if (hasStage(state.token)) armTimer(state.token);
      updateBusy();
    }
  }

  async function ready(state: TabTransferSnapshot) {
    if (disposed || state.status !== 'target-ready' || state.sourceWindowLabel !== options.getWindowLabel() ||
        !sourceStages.has(state.token) || applied.has(state.token) || committing.has(state.token)) return;
    committing.add(state.token);
    try {
      await settle(await commitTabTransfer(state.token));
    } catch (error) {
      // commit 可能已落盘，只是 invoke 响应丢失；cancel 也可能返回 committed。
      await cancelAndSettle(state.token);
      if (!disposed) options.showError(error);
    } finally {
      committing.delete(state.token);
      if (hasStage(state.token)) armTimer(state.token, 5000);
      updateBusy();
    }
  }

  async function committed(state: TabTransferSnapshot) {
    if (disposed || state.status !== 'committed' || !isParticipant(state) || applied.has(state.token)) return;
    const source = options.getWindowLabel() === state.sourceWindowLabel;
    const staged = source ? sourceStages.get(state.token) : targetStages.get(state.token);
    if (!staged && !preparing && preparingTarget !== state.token) return;
    terminalResults.set(state.token, state);
    if (applying.has(state.token)) return;
    if (!staged) return;
    applying.add(state.token);
    updateBusy();
    try {
      applyWindowWorkspaceEpoch(source ? state.sourceEpoch : state.targetEpoch);
      options.applyWorkspace(staged, source ? undefined : state.positions);
      applied.add(state.token);
      stopTimer(state.token);
      sourceStages.delete(state.token);
      targetStages.delete(state.token);
      terminalResults.delete(state.token);
      reportedRecoveryErrors.delete(state.token);
      if (source && staged.items.length === 0) {
        closingSource = true;
        emptySourceCloseState = state;
        await closeEmptySource();
      }
    } catch (error) {
      if (!disposed) options.showError(error);
      if (!applied.has(state.token)) armTimer(state.token, 5000);
    } finally {
      applying.delete(state.token);
      updateBusy();
    }
  }

  function cancelled(state: TabTransferSnapshot) {
    if (state.status === 'committed') { void committed(state); return; }
    if (disposed || state.status !== 'cancelled' || !isParticipant(state) || applied.has(state.token)) return;
    if (terminalResults.get(state.token)?.status === 'committed') return;
    cancelledTokens.add(state.token);
    terminalResults.delete(state.token);
    stopTimer(state.token);
    sourceStages.delete(state.token);
    targetStages.delete(state.token);
    reportedRecoveryErrors.delete(state.token);
    updateBusy();
  }

  function dispose() {
    disposed = true;
    for (const timer of timers.values()) clearTimeout(timer);
    timers.clear();
    if (emptySourceCloseTimer !== undefined) clearTimeout(emptySourceCloseTimer);
    emptySourceCloseTimer = undefined;
  }

  return { transfer, prepared, ready, committed, cancelled, dispose };
}
