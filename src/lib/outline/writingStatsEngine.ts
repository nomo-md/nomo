import { schema } from '../editor-core/schema';
import {
  prepareMarkdownSelection,
  measureMarkdownSelection,
  serializeClipboardText,
} from '../editor-core/markdownSerialization';
import { removeEmptyTrailingParagraph } from '../editor-core/plugins/trailingParagraph';
import {
  buildSourceWritingStats,
  buildSemanticWritingStats,
  queryWritingStats,
  sourceRangeToRaw,
} from './writingStats';
import type { StatsRequest, StatsResponse } from './writingStatsProtocol';
import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { utf8Length, utf8ToUtf16 } from '../markdown-document/utf8Text';

/** Worker 单文档缓存：范围请求不再解析或构建文档快照。 */
export function createWritingStatsEngine() {
  let cached: {
    documentId: string;
    revision: number;
    markdown: string;
    source: ReturnType<typeof buildSourceWritingStats>;
    semantic: ReturnType<typeof buildSemanticWritingStats> | null;
    doc: ProseMirrorNode | null;
    clipboardDoc: ProseMirrorNode | null;
    byteCheckpoints?: Array<{ byte: number; offset: number }>;
  } | null = null;
  return (request: StatsRequest): StatsResponse => {
    try {
      const { context, snapshot, selection } = request;
      if (snapshot?.markdown !== undefined) {
        cached = {
          documentId: context.documentId,
          revision: context.contentRevision,
          markdown: snapshot.markdown,
          source: buildSourceWritingStats(snapshot.markdown),
          doc: null,
          clipboardDoc: null,
          semantic: null,
        };
      }
      if (
        !cached ||
        cached.documentId !== context.documentId ||
        cached.revision !== context.contentRevision
      ) {
        throw new Error('Statistics snapshot is outdated');
      }
      if (snapshot?.semanticDoc) {
        const json =
          typeof snapshot.semanticDoc === 'string'
            ? JSON.parse(snapshot.semanticDoc)
            : snapshot.semanticDoc;
        cached.doc = schema.nodeFromJSON(json);
        cached.clipboardDoc = removeEmptyTrailingParagraph(cached.doc);
        cached.semantic = buildSemanticWritingStats(cached.doc);
        prepareMarkdownSelection(cached.clipboardDoc);
      }
      const full = queryWritingStats(cached.source);
      let selected = null;
      if (selection && selection.from !== selection.to) {
        const from = Math.min(selection.from, selection.to),
          to = Math.max(selection.from, selection.to);
        if (context.mode === 'source') {
          const byteToOffset = (byte: number) => {
            if (!cached!.byteCheckpoints) {
              const checkpoints = [{ byte: 0, offset: 0 }];
              for (let offset = 0, bytes = 0; offset < cached!.markdown.length;) {
                let end = Math.min(cached!.markdown.length, offset + 4096);
                if (end < cached!.markdown.length && /[\uDC00-\uDFFF]/.test(cached!.markdown[end])) end--;
                bytes += utf8Length(cached!.markdown.slice(offset, end)); offset = end; checkpoints.push({ byte: bytes, offset });
              }
              cached!.byteCheckpoints = checkpoints;
            }
            const points = cached!.byteCheckpoints;
            let low = 0, high = points.length - 1;
            while (low < high) { const mid = Math.ceil((low + high) / 2); if (points[mid].byte <= byte) low = mid; else high = mid - 1; }
            const start = points[low];
            return start.offset + utf8ToUtf16(cached!.markdown.slice(start.offset, points[low + 1]?.offset), byte - start.byte);
          };
          const raw = selection.sourceCoordinates === 'utf8' ? [byteToOffset(from), byteToOffset(to)] :
            selection.sourceCoordinates === 'normalized'
              ? sourceRangeToRaw(cached.source, from, to)
              : [from, to];
          selected = queryWritingStats(cached.source, raw[0], raw[1]);
        } else {
          if (!cached.semantic || !cached.doc)
            throw new Error('Semantic statistics snapshot unavailable');
          if (from <= 1 && to >= cached.doc.content.size - 1) selected = full;
          else {
            selected = queryWritingStats(cached.semantic, from, to);
            const doc = cached.clipboardDoc!;
            const end = Math.min(to, doc.content.size);
            const measured = measureMarkdownSelection(doc, from, end);
            if (measured) {
              selected.chars = measured.chars;
              selected.lines = measured.lines;
            } else {
              const text = serializeClipboardText(cached.doc.slice(from, to), true);
              selected.chars = text.length;
              selected.lines = text.split(/\r\n|\r|\n/).length;
            }
          }
        }
      }
      return { requestId: request.requestId, context, full, selected };
    } catch (error) {
      return {
        requestId: request.requestId,
        context: request.context,
        error: error instanceof Error ? error.message : 'Statistics unavailable',
      };
    }
  };
}
