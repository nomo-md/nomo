import type { MarkdownEnhancements } from './enhancements';

/** 所有正文范围都是工作正文的 UTF-8 字节范围；禁止传入 PM/CM 的 UTF-16 位置。 */
export interface SourceRange {
  fromByte: number;
  toByte: number;
}
export interface SourceEdit extends SourceRange {
  insertedText: string;
}
export interface DocumentSelection {
  anchorByte: number;
  headByte: number;
}
export interface DocumentTransaction {
  /** 派生内容持久化，但不建立独立撤销步骤或清空重做。 */
  derived?: boolean;
  baseRevision: number;
  operationId: string;
  historyGroupId: string;
  edits: readonly SourceEdit[];
  selectionBefore?: DocumentSelection;
  selectionAfter?: DocumentSelection;
}
export interface DocumentStatus {
  hasDerivedEdits?: boolean;
  documentId: string;
  revision: number;
  byteLength: number;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  selection?: DocumentSelection;
}
export interface DocumentWindow extends SourceRange {
  documentId: string;
  revision: number;
  text: string;
}
export type DocumentTaskState = 'building' | 'partial' | 'ready' | 'failed';
export interface DocumentIndexProgress {
  revision: number;
  state: DocumentTaskState;
  indexedBytes: number;
  totalBytes: number;
  layoutUnits: number;
  enhancementObjects: number;
  enhancements: MarkdownEnhancements;
  metricsComplete?: boolean;
}
export class DocumentConflictError extends Error {
  constructor(message = '正文已经变化，请基于当前版本重试。') {
    super(message);
    this.name = 'DocumentConflictError';
  }
}
