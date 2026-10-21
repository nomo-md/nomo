import {
  extractTocRanges,
  serializeTocItems,
  TOC_START_MARKER,
  TOC_END_MARKER,
} from '../toc/tocService';
import type { MarkdownSessionController } from './controller';
import type { MarkdownDocumentPort, SnapshotReference } from './port';
import type { buildMarkdownIndex } from './incrementalIndex';
import type { MarkdownStructureIndex } from './structureIndex';
import type { SourceEdit } from './protocol';
import { mapMarkdownSelection } from './positions';
import { utf16ToUtf8 } from './utf8Text';

const signature = (index: MarkdownStructureIndex) =>
  JSON.stringify(index.outline.map(({ id, level, title }) => [id, level, title]));

/** 只从完整同版本索引派生目录；原始快照用于恢复最初目录的字节片段。 */
export class AutomaticMarkdownToc {
  private original: { snapshot: SnapshotReference; index: MarkdownStructureIndex } | null = null;
  private running: Promise<boolean> | null = null;
  private abort = new AbortController();
  constructor(
    private session: MarkdownSessionController,
    private build: typeof buildMarkdownIndex,
  ) {}

  apply(index: MarkdownStructureIndex, snapshot: SnapshotReference): Promise<boolean> {
    if (this.running) return this.running;
    this.running = this.materialize(index, snapshot).finally(() => {
      this.running = null;
    });
    return this.running;
  }
  private async read(
    port: MarkdownDocumentPort,
    snapshot: SnapshotReference,
    from: number,
    to: number,
  ) {
    const chunks: string[] = [];
    while (from < to) {
      this.abort.signal.throwIfAborted();
      const part = await port.readSnapshot(snapshot, from, Math.min(to, from + 256 * 1024));
      if (part.fromByte !== from || part.toByte <= from) throw new Error('目录范围读取不连续。');
      chunks.push(part.text);
      from = part.toByte;
    }
    return chunks.join('');
  }
  private async tocRanges(index: MarkdownStructureIndex, snapshot: SnapshotReference) {
    const result: Array<{ fromByte: number; toByte: number; text: string }> = [];
    for (const block of index.blocks) {
      if (block.kind !== 'toc_block' && block.kind !== 'html_block') continue;
      const source = await this.read(this.session.port, snapshot, block.fromByte, block.toByte);
      for (const range of extractTocRanges(source))
        result.push({
          fromByte: block.fromByte + utf16ToUtf8(source, range.start),
          toByte: block.fromByte + utf16ToUtf8(source, range.end),
          text: source.slice(range.start, range.end),
        });
    }
    return result;
  }
  private async materialize(index: MarkdownStructureIndex, snapshot: SnapshotReference) {
    if (
      index.progress.state !== 'ready' ||
      index.revision !== snapshot.revision ||
      snapshot.revision === 0
    )
      return false;
    const generation = this.session.state.viewGeneration;
    if (
      this.session.state.pendingTransactions ||
      snapshot.revision !== this.session.state.committed.revision
    )
      return false;
    const ranges = await this.tocRanges(index, snapshot);
    if (!ranges.length) return false;
    if (!this.original) {
      const original = await this.session.port.captureOriginal(this.session.documentId);
      try {
        const originalIndex = await this.build(this.session.port, original, {
          signal: this.abort.signal,
          onProgress() {},
          shouldYield: () => new Promise((resolve) => setTimeout(resolve, 0)),
        });
        this.original = { snapshot: original, index: originalIndex };
      } catch (error) {
        await this.session.port.releaseSnapshot(original);
        throw error;
      }
    }
    const sameHeadings = signature(index) === signature(this.original.index);
    // 单纯打开、正文编辑或建立索引不规范化用户原有目录。
    if (sameHeadings && !this.session.state.committed.hasDerivedEdits) return false;
    const originalRanges = sameHeadings
      ? await this.tocRanges(this.original.index, this.original.snapshot)
      : [];
    const content = serializeTocItems(index.outline);
    const edits: SourceEdit[] = [];
    for (let i = 0; i < ranges.length; i++) {
      const range = ranges[i];
      const newline = range.text.match(/\r\n|\n|\r/)?.[0] ?? '\n';
      const generated = `${TOC_START_MARKER}${newline}${content.replace(/\n/g, newline)}${content ? newline : ''}${TOC_END_MARKER}`;
      const insertedText = sameHeadings ? originalRanges[i]?.text ?? generated : generated;
      if (insertedText !== range.text)
        edits.push({ fromByte: range.fromByte, toByte: range.toByte, insertedText });
    }
    this.abort.signal.throwIfAborted();
    if (
      !edits.length ||
      generation !== this.session.state.viewGeneration ||
      this.session.state.pendingTransactions
    )
      return false;
    const selection = this.session.state.committed.selection;
    await this.session.applyEdits({
      derived: true,
      viewGeneration: generation,
      edits,
      historyGroupId: 'automatic-toc',
      selectionBefore: selection,
      selectionAfter: selection && mapMarkdownSelection(selection, edits),
    });
    return true;
  }
  async dispose() {
    this.abort.abort();
    await this.running?.catch(() => undefined);
    if (this.original) await this.session.port.releaseSnapshot(this.original.snapshot);
    this.original = null;
  }
}
