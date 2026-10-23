import type { DocumentSelection } from './protocol';
import type { MarkdownSessionStatus } from './port';

export const MARKDOWN_SEARCH_EVENT = 'nomo://markdown-task-progress';
export const SEARCH_PAGE_SIZE = 128;
export interface SearchQuery {
  documentId: string; baseRevision: number; requestId: string; query: string;
  caseSensitive?: boolean; wholeWord?: boolean;
}
export interface SearchIdentity { requestId: string; taskId: string; documentId: string; baseRevision: number }
export interface SearchMatch {
  index: number; fromByte: number; toByte: number; preservedPrefix: string; preservedSuffix: string;
}
export interface SearchProgress extends SearchIdentity {
  state: 'queued' | 'running' | 'ready' | 'prepared' | 'completed' | 'cancelled' | 'conflict' | 'failed';
  scannedBytes: number; totalBytes: number; matchCount: number;
  error?: { code: string; message: string } | null;
}
export interface SearchPage extends SearchIdentity { totalCount: number; fromIndex: number; matches: SearchMatch[] }
export interface ReplaceQuery extends SearchQuery {
  operationId: string; replacement: string; selection?: DocumentSelection; anchors?: number[];
}
export interface ReplaceResult extends SearchIdentity {
  status: MarkdownSessionStatus; count: number; changedCount: number; anchors: number[];
}
export interface MarkdownSearchPort {
  listenSearch(listener: (progress: SearchProgress) => void): Promise<() => void>;
  startSearch(query: SearchQuery): Promise<SearchIdentity>;
  readSearchPage(taskId: string, fromIndex: number, limit: number): Promise<SearchPage>;
  searchTaskProgress(taskId: string): Promise<SearchProgress>;
  prepareReplaceAll(query: ReplaceQuery): Promise<SearchIdentity>;
  commitReplaceAll(documentId: string, taskId: string, operationId: string): Promise<ReplaceResult>;
  cancelTask(taskId: string): Promise<void>;
  releaseTask(taskId: string): Promise<void>;
}

export interface SearchOptions {
  caseSensitive?: boolean; wholeWord?: boolean; signal: AbortSignal;
  onProgress?(progress: SearchProgress): void;
}
