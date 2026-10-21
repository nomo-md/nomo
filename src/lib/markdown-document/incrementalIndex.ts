import type { MarkdownDocumentPort, SnapshotReference } from './port';
import type { SourceEdit } from './protocol';
import { MarkdownStructureIndex, scanMarkdownSnapshot } from './structureIndex';
import type { MarkdownStructureBatch } from './structure';
import { invalidateMarkdownIndex, type MarkdownIndexInvalidation } from './indexInvalidation';

export interface MarkdownIndexedSnapshot {
  index: MarkdownStructureIndex;
  snapshot: SnapshotReference;
}

/**
 * 从变更前的安全块重新扫描；只有修改之后某个完整语义块的内容、边界重新一致，
 * 才复用其后的结构。引用字典重新合并，渲染层根据实际变更的定义失效依赖块。
 */
export async function buildMarkdownIndex(
  port: MarkdownDocumentPort,
  snapshot: SnapshotReference,
  options: {
    signal: AbortSignal;
    previous?: MarkdownIndexedSnapshot;
    edits?: readonly SourceEdit[];
    invalidation?: MarkdownIndexInvalidation;
    onProgress(index: MarkdownStructureIndex): void;
    shouldYield?: () => Promise<void>;
    scan?: typeof scanMarkdownSnapshot;
  },
) {
  const previous = options.previous;
  const invalidation =
    options.invalidation ?? invalidateMarkdownIndex(undefined, options.edits ?? []);
  const canReuse = previous && invalidation;
  const first = canReuse
    ? previous.index.blockAt(Math.max(0, invalidation.fromByte - 1))
    : undefined;
  const checkpoint = {
    fromByte: first?.fromByte ?? 0,
    fromLine: first?.fromLine ?? 1,
    shift: canReuse ? invalidation.byteDelta : 0,
  };
  const index = canReuse
    ? previous.index.forkPrefix(snapshot.revision, snapshot.byteLength, checkpoint.fromByte)
    : new MarkdownStructureIndex(snapshot.revision, snapshot.byteLength);
  const lastChangedByte = invalidation?.toByte ?? Infinity;
  await (options.scan ?? scanMarkdownSnapshot)(port, snapshot, {
    signal: options.signal,
    shouldYield: options.shouldYield,
    checkpoint: {
      fromByte: checkpoint.fromByte,
      fromLine: checkpoint.fromLine,
      references: index.references,
    },
    onBatch: async (batch: MarkdownStructureBatch) => {
      let through = batch.blocks.length;
      let converged: { oldEnd: number; lineDelta: number } | null = null;
      if (canReuse && previous) {
        for (let blockIndex = 0; blockIndex < batch.blocks.length; blockIndex++) {
          const block = batch.blocks[blockIndex];
          if (block.mode !== 'semantic') continue;
          const unchangedPrefix = block.toByte <= invalidation.fromByte;
          if (!unchangedPrefix && block.fromByte < lastChangedByte + checkpoint.shift) continue;
          const shift = unchangedPrefix ? 0 : checkpoint.shift;
          const old = previous.index.blockAt(block.fromByte - shift);
          if (
            !old ||
            old.mode !== 'semantic' ||
            old.kind !== block.kind ||
            old.sourceHash !== block.sourceHash ||
            old.fromByte !== block.fromByte - shift ||
            old.toByte !== block.toByte - shift
          )
            continue;
          const [before, after] = await Promise.all([
            port.readSnapshot(previous.snapshot, old.fromByte, old.toByte),
            port.readSnapshot(snapshot, block.fromByte, block.toByte),
          ]);
          options.signal.throwIfAborted();
          if (before.text !== after.text) continue;
          block.id = old.id;
          block.contentVersion = old.contentVersion ?? old.version;
          if (unchangedPrefix || previous.index.progress.state !== 'ready') continue;
          through = blockIndex + 1;
          converged = { oldEnd: old.toByte, lineDelta: block.toLine - old.toLine };
          break;
        }
      }
      const published = batch.blocks.slice(0, through);
      if (converged) {
        // batch.references 包含本批次后面尚未发布的定义，不能把它们提前混入。
        const references = { ...index.references };
        for (const block of published)
          for (const [key, value] of Object.entries(block.definitions ?? {}))
            references[key] ??= value;
        index.publish({
          blocks: published,
          references,
          consumedBytes: published.at(-1)?.toByte ?? checkpoint.fromByte,
          complete: false,
        });
        index.appendUnchangedTail(
          previous!.index,
          converged.oldEnd,
          checkpoint.shift,
          converged.lineDelta,
        );
      } else index.publish(batch);
      options.onProgress(index);
      return converged ? false : undefined;
    },
  });
  return index;
}
