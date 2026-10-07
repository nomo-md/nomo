import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { MarkdownTabState, PersistedWorkspaceState, WorkspaceItem } from '../types';
import type { WindowOpenTargetsSnapshot } from './desktopWindow';

export type TabDropPlacement =
  | { kind: 'insert'; insertionIndex: number }
  | { kind: 'combine'; targetItemId: string };

export interface WorkspaceWriteStamp {
  ownershipEpoch: number;
  revision: number;
}

export interface TabTransferInput {
  item: WorkspaceItem;
  tabs: MarkdownTabState[];
  positions?: Record<string, unknown>;
  sourceWorkspace: PersistedWorkspaceState;
  sourceOpenTargets: WindowOpenTargetsSnapshot;
  targetWindowLabel?: string;
  placement: TabDropPlacement;
  screenPosition?: { x: number; y: number };
}

export interface TabTransferSnapshot extends TabTransferInput {
  token: string;
  status: 'prepared' | 'target-ready' | 'committed' | 'cancelled';
  sourceWindowLabel: string;
  targetWindowLabel: string;
  sourceEpoch: number;
  targetEpoch: number;
  newWindow: boolean;
  targetWorkspace?: PersistedWorkspaceState;
  targetOpenTargets?: WindowOpenTargetsSnapshot;
}

export interface NativeTabDragInput {
  dragId: string;
  itemId: string;
  tabId?: string;
  canCombine: boolean;
}

export interface NativeTabDragEvent extends NativeTabDragInput {
  phase: 'move' | 'drop' | 'cancel';
  sourceWindowLabel: string;
  targetWindowLabel?: string;
  placement?: TabDropPlacement;
  screenPosition: { x: number; y: number };
  sourceClientPosition?: { x: number; y: number } | null;
}

export interface TabDropZone {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  placement: TabDropPlacement;
}

let cachedStamp: WorkspaceWriteStamp | undefined;
let stampRequest: Promise<WorkspaceWriteStamp> | undefined;

export async function getWindowWorkspaceStamp(): Promise<WorkspaceWriteStamp> {
  if (cachedStamp) return { ...cachedStamp };
  stampRequest ??= invoke<WorkspaceWriteStamp>('get_window_workspace_stamp');
  let stamp: WorkspaceWriteStamp;
  try {
    stamp = await stampRequest;
  } catch (error) {
    stampRequest = undefined;
    throw error;
  }
  cachedStamp ??= stamp;
  return { ...cachedStamp };
}

/** 在开始生成快照时捕获，不能在异步磁盘读取完成后才取最新 epoch。 */
export async function takeWorkspaceWriteStamp(): Promise<WorkspaceWriteStamp> {
  if (!cachedStamp) await getWindowWorkspaceStamp();
  cachedStamp!.revision += 1;
  return { ...cachedStamp! };
}

export function applyWindowWorkspaceEpoch(epoch: number): void {
  if (!cachedStamp || epoch > cachedStamp.ownershipEpoch) {
    cachedStamp = { ownershipEpoch: epoch, revision: 0 };
  }
}

export function prepareTabTransfer(input: TabTransferInput): Promise<TabTransferSnapshot> {
  return invoke('prepare_tab_transfer', { input });
}

export function targetReadyTabTransfer(
  token: string,
  targetWorkspace: PersistedWorkspaceState,
  targetOpenTargets: WindowOpenTargetsSnapshot,
): Promise<TabTransferSnapshot> {
  return invoke('target_ready_tab_transfer', { token, targetWorkspace, targetOpenTargets });
}

export function commitTabTransfer(token: string): Promise<TabTransferSnapshot> {
  return invoke('commit_tab_transfer', { token });
}

export function cancelTabTransfer(token: string): Promise<TabTransferSnapshot> {
  return invoke('cancel_tab_transfer', { token });
}

export function queryTabTransfer(token: string): Promise<TabTransferSnapshot> {
  return invoke('query_tab_transfer', { token });
}

export function getTabTransferBootstrap(): Promise<TabTransferSnapshot | null> {
  return invoke('get_tab_transfer_bootstrap');
}

export function registerTabDropZones(zones: TabDropZone[], ready = true): Promise<void> {
  return invoke('register_tab_drop_zones', { zones, ready });
}

export function beginNativeTabDrag(input: NativeTabDragInput): Promise<void> {
  return invoke('begin_native_tab_drag', { input });
}

export function cancelNativeTabDrag(dragId: string): Promise<void> {
  return invoke('cancel_native_tab_drag', { dragId });
}

export function onNativeTabDrag(handler: (event: NativeTabDragEvent) => void): Promise<UnlistenFn> {
  return listen<NativeTabDragEvent>('nomo://native-tab-drag', (event) => handler(event.payload));
}

export function onTabTransfer(
  phase: 'prepared' | 'ready' | 'committed' | 'cancelled',
  handler: (transfer: TabTransferSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<TabTransferSnapshot>(`nomo://tab-transfer-${phase}`, (event) =>
    handler(event.payload),
  );
}

export function closeEmptyTransferredWindow(token: string): Promise<void> {
  return invoke('close_empty_transferred_window', { token });
}
