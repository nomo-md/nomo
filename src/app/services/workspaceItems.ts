import type {
  ComparisonWorkspaceItem,
  DocumentKind,
  SingleWorkspaceItem,
  WorkspaceItem,
} from '../types';
import type { EditorMode } from '../../lib/editor-core';

/** 正文无关的文档元数据，运行时标签和恢复元数据均可参与组合校验。 */
export interface WorkspaceItemTabMetadata {
  id: string;
  documentKind: DocumentKind;
  largeDocumentMode?: boolean;
}

export function createSingleWorkspaceItem(tabId: string): SingleWorkspaceItem {
  return { kind: 'single', id: `item-${tabId}`, tabId };
}

export function createComparisonWorkspaceItem(
  leftTabId: string,
  rightTabId: string,
  mode: EditorMode = 'semantic',
): ComparisonWorkspaceItem {
  if (!leftTabId || !rightTabId || leftTabId === rightTabId) {
    throw new Error('A comparison requires two different documents');
  }
  return {
    kind: 'comparison',
    id: `comparison-${Date.now()}-${Math.random().toString(36).slice(2, 11)}`,
    leftTabId,
    rightTabId,
    focusedTabId: rightTabId,
    mode,
    leftPercent: 50,
  };
}

export function getWorkspaceItemTabIds(item: WorkspaceItem): string[] {
  return item.kind === 'single' ? [item.tabId] : [item.leftTabId, item.rightTabId];
}

export function findWorkspaceItemForTab(items: readonly WorkspaceItem[], tabId: string) {
  return items.find((item) => getWorkspaceItemTabIds(item).includes(tabId));
}

export function getWorkspaceItemFocusedTabId(item: WorkspaceItem): string {
  if (item.kind === 'single') return item.tabId;
  return item.focusedTabId === item.rightTabId ? item.rightTabId : item.leftTabId;
}

/** 损坏或重复的组合降级为普通项，确保每篇有效文档恰好出现一次。 */
export function normalizeWorkspaceItems(
  tabs: readonly WorkspaceItemTabMetadata[],
  items: readonly unknown[] | null | undefined = [],
): WorkspaceItem[] {
  const tabsById = new Map(tabs.map((tab) => [tab.id, tab]));
  const usedTabIds = new Set<string>();
  const usedItemIds = new Set<string>();
  const result: WorkspaceItem[] = [];

  function appendSingle(tabId: unknown, preferredId?: unknown) {
    if (typeof tabId !== 'string' || !tabsById.has(tabId) || usedTabIds.has(tabId)) return;
    const item = createSingleWorkspaceItem(tabId);
    item.id = reserveItemId(preferredId, item.id);
    result.push(item);
    usedTabIds.add(tabId);
  }

  function reserveItemId(preferredId: unknown, fallbackId: string) {
    let id = typeof preferredId === 'string' && preferredId ? preferredId : fallbackId;
    if (usedItemIds.has(id)) id = fallbackId;
    const baseId = id;
    let suffix = 2;
    while (usedItemIds.has(id)) id = `${baseId}-${suffix++}`;
    usedItemIds.add(id);
    return id;
  }

  for (const value of Array.isArray(items) ? items : []) {
    if (!value || typeof value !== 'object') continue;
    const item = value as Partial<WorkspaceItem>;
    if (item.kind === 'single') {
      appendSingle(item.tabId, item.id);
      continue;
    }
    if (item.kind !== 'comparison') continue;

    const left = typeof item.leftTabId === 'string' ? tabsById.get(item.leftTabId) : undefined;
    const right = typeof item.rightTabId === 'string' ? tabsById.get(item.rightTabId) : undefined;
    const validPair = left?.documentKind === 'markdown' && right?.documentKind === 'markdown' &&
      left.id !== right.id && !usedTabIds.has(left.id) && !usedTabIds.has(right.id);
    if (!validPair || !left || !right) {
      appendSingle(item.leftTabId);
      appendSingle(item.rightTabId);
      continue;
    }

    const fallbackId = `comparison-${left.id}-${right.id}`;
    result.push({
      kind: 'comparison',
      id: reserveItemId(item.id, fallbackId),
      leftTabId: left.id,
      rightTabId: right.id,
      focusedTabId: item.focusedTabId === right.id ? right.id : left.id,
      mode: left.largeDocumentMode || right.largeDocumentMode || item.mode === 'source'
        ? 'source'
        : 'semantic',
      leftPercent: typeof item.leftPercent === 'number' && Number.isFinite(item.leftPercent)
        ? Math.min(80, Math.max(20, item.leftPercent))
        : 50,
    });
    usedTabIds.add(left.id);
    usedTabIds.add(right.id);
  }

  for (const tab of tabs) appendSingle(tab.id);
  return result;
}

/** 移出或关闭组合成员后，另一篇原位恢复成普通标签。 */
export function removeWorkspaceItemMember(
  items: readonly WorkspaceItem[],
  tabId: string,
): WorkspaceItem[] {
  return items.flatMap<WorkspaceItem>((item) => {
    if (item.kind === 'single') return item.tabId === tabId ? [] : [item];
    if (item.leftTabId === tabId) return [createSingleWorkspaceItem(item.rightTabId)];
    if (item.rightTabId === tabId) return [createSingleWorkspaceItem(item.leftTabId)];
    return [item];
  });
}

export function ungroupWorkspaceItem(
  items: readonly WorkspaceItem[],
  itemId: string,
): WorkspaceItem[] {
  return items.flatMap<WorkspaceItem>((item) => item.id === itemId && item.kind === 'comparison'
    ? [createSingleWorkspaceItem(item.leftTabId), createSingleWorkspaceItem(item.rightTabId)]
    : [item]);
}

export function swapComparisonMembers(
  items: readonly WorkspaceItem[],
  itemId: string,
): WorkspaceItem[] {
  return items.map((item) => item.id === itemId && item.kind === 'comparison'
    ? { ...item, leftTabId: item.rightTabId, rightTabId: item.leftTabId }
    : item);
}
