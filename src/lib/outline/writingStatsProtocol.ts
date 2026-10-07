import type { DocumentStats } from './outlineService';

export interface StatsRange {
  from: number;
  to: number;
  sourceCoordinates?: 'normalized';
}
export interface StatsContext {
  documentId: string;
  contentRevision: number;
  selectionRevision: number;
  mode: 'source' | 'semantic';
}
export interface StatsRequest {
  requestId: number;
  context: StatsContext;
  snapshot?: { markdown?: string; semanticDoc?: Record<string, unknown> | string | null };
  selection: StatsRange | null;
}
export interface StatsResponse {
  requestId: number;
  context: StatsContext;
  full?: DocumentStats;
  selected?: DocumentStats | null;
  error?: string;
}
export interface WritingStatsState {
  status: 'pending' | 'ready' | 'error';
  full: DocumentStats | null;
  selected: DocumentStats | null;
}
