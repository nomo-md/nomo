import type { MarkdownStructureBlock, MarkdownReference } from './structure';
import type { MarkdownOutlineItem } from './structureIndex';

export const STRUCTURE_SYNTAX_VERSION = 'nomo-markdown-v1';
export interface StructureIdentity {
  documentId: string;
  revision: number;
  indexGeneration: number;
  syntaxVersion: string;
}
export interface StructureCursor { generation: number; ordinal: number; offset: number }
export type StructureQuery =
  | { kind: 'openOutline'; defaultLevel: number }
  | { kind: 'outlineView'; viewId: string; fromIndex: number; viewGeneration?: number }
  | { kind: 'setOutlineFold'; viewId: string; headingId: string; collapsed: boolean }
  | { kind: 'setOutlineLevel'; viewId: string; level: number }
  | { kind: 'closeOutline'; viewId: string }
  | { kind: 'progress' }
  | { kind: 'locateOutline'; viewId: string; headingId: string; byte: number; parent: boolean }
  | { kind: 'changes'; observedGeneration: number }
  | { kind: 'blocks'; fromByte: number; toByte: number; cursor?: StructureCursor }
  | { kind: 'outline'; cursor?: StructureCursor }
  | { kind: 'headings'; fromByte: number; toByte: number; cursor?: StructureCursor }
  | { kind: 'geometry'; fromByte: number; toByte: number; groupSize: number; cursor?: StructureCursor }
  | { kind: 'resolve'; namespace: 'heading' | 'slug' | 'link' | 'footnote' | 'footnoteReference'; key: string }
  | { kind: 'references'; keys: string[] };
export interface StructureResponse<T = unknown> extends StructureIdentity {
  state: 'building' | 'ready' | 'failed';
  indexedBytes: number;
  totalBytes: number;
  semanticComplete: boolean;
  failure: string | null;
  items: T[];
  next: StructureCursor | null;
  fragment: string | null;
}
/** 正式实现仅在原生 Markdown IPC；内存端口只用于现有测试。 */
export interface StructureQueryPort {
  startStructure(documentId: string, revision: number): Promise<StructureIdentity>;
  queryStructure<T = unknown>(identity: StructureIdentity, query: StructureQuery): Promise<StructureResponse<T>>;
  updateStructureDemand?(identity: StructureIdentity, active: boolean, priority: 0 | 1 | 2): Promise<void>;
  closeStructureOutline?(documentId: string, viewId: string): Promise<void>;
}
export type StructureBlock = MarkdownStructureBlock & { semanticComplete: boolean; headings: Array<MarkdownStructureBlock['headings'][number] & { title: string }> };
export type StructureHeading = MarkdownOutlineItem;
export interface StructureOutlineRow extends MarkdownOutlineItem {
  local?: boolean;
  position: number;
  collapsed: boolean;
  parentByte: number | null;
  hasChildren: boolean;
}
export interface StructureOutlineView {
  viewId: string;
  viewGeneration: number;
  visibleCount: number;
  projectionComplete: boolean;
  fromIndex: number;
  rows: StructureOutlineRow[];
}
export interface StructureReference { key: string; definition: MarkdownReference }
export interface StructureChanges {
  fromGeneration: number;
  invalidateAll: boolean;
  ranges: Array<{ fromByte: number; toByte: number }>;
  mappings: Array<{ fromByte: number; toByte: number; byteDelta: number; lineDelta: number }>;
  references: Array<{ namespace: string; key: string }>;
  referenceVersion: number;
  outlineChanged: boolean;
}
export type StructureTarget = { status: 'incomplete' | 'missing'; target: null } | {
  status: 'found'; target: { fromByte: number; toByte: number; blockId?: string; slug?: string;
    line?: number; blockHeadingIndex?: number; definition?: string };
};

export function assertStructureIdentity(expected: StructureIdentity, actual: StructureIdentity): void {
  if (expected.documentId !== actual.documentId || expected.revision !== actual.revision ||
      expected.indexGeneration !== actual.indexGeneration || actual.syntaxVersion !== STRUCTURE_SYNTAX_VERSION) {
    throw new Error('结构查询返回了不匹配的修订或生成版本。');
  }
}
