import type { EditorCore, EditorMode } from '../../lib/editor-core';
import type { OutlineItem } from '../../lib/outline/outlineService';
import type { MarkdownSourceEditorHandle, MarkdownSourceRuntimeState } from '../components/markdownSourceEditor';
import type { EditorViewMode } from '../types';
import type { createEditorInteractionController } from './editorInteractionController';
import type { createImageInsertionHandlers } from './imageInsertion';
import type { OutlineScrollAnchor } from './outlineNavigation';
import type { createOutlineInteractionController } from './outlineInteractionController';

/** 一次挂载所拥有的文档与 DOM；销毁回调用此快照，避免读取后来切换的全局焦点。 */
export interface MarkdownWorkspaceRuntimeBinding {
  documentId: string;
  editorCore: EditorCore;
  host: HTMLDivElement;
  sourceEditor: MarkdownSourceEditorHandle;
  sourcePane: HTMLElement;
  semanticPane: HTMLElement;
  mode: EditorViewMode;
  sourceRuntimeState?: MarkdownSourceRuntimeState;
}

/** 文档状态随标签保留，DOM 宿主只在文档可见时存在。 */
export interface MarkdownDocumentRuntime {
  tabId: string;
  editor: EditorCore;
  mode: EditorViewMode;
  sourceState?: MarkdownSourceRuntimeState;
  sourceEditor?: MarkdownSourceEditorHandle;
  sourcePane?: HTMLElement;
  semanticPane?: HTMLElement;
  host?: HTMLDivElement;
  outline: OutlineItem[];
  semanticScrollTop: number;
  sourceScrollTop: number;
  pendingSourceScrollTop: number | null;
  readingAnchor?: OutlineScrollAnchor | null;
  readingAnchorMode?: 'semantic' | 'source';
  activeOutlineId?: string;
  collapsedOutlineIds?: Set<string>;
  suppressOutlineScrollUntil?: number;
  outlineInteraction?: ReturnType<typeof createOutlineInteractionController>;
  transferSelection?: { semantic?: { anchor: number; head: number }; source?: { anchor: number; head: number } };
  interaction: ReturnType<typeof createEditorInteractionController>;
  images: ReturnType<typeof createImageInsertionHandlers>;
  unsubscribe: () => void;
  disposed: boolean;
}

export function getRuntimeEditorMode(runtime: MarkdownDocumentRuntime): EditorMode {
  return runtime.mode === 'source' ? 'source' : 'semantic';
}
