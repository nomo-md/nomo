import { describe, expect, it } from 'vitest';
import { createMemoryMarkdownPort, type SnapshotReference } from './port';
import { MarkdownStructureScanner } from './structure';
import { buildMarkdownIndex } from './incrementalIndex';
import type { scanMarkdownSnapshot } from './structureIndex';
import { invalidateMarkdownIndex } from './indexInvalidation';
import { extractOutline } from '../outline/outlineService';

function scannerSpy(reads: number[]): typeof scanMarkdownSnapshot {
  return async (port, snapshot, options) => {
    const scanner = new MarkdownStructureScanner(snapshot.revision, options.checkpoint);
    let offset = options.checkpoint?.fromByte ?? 0;
    do {
      const window = await port.readSnapshot(
        snapshot,
        offset,
        Math.min(snapshot.byteLength, offset + 1024),
      );
      reads.push(window.text.length);
      offset = window.toByte;
      if (
        (await options.onBatch(scanner.push(window.text, offset === snapshot.byteLength))) === false
      )
        break;
    } while (offset < snapshot.byteLength);
  };
}

describe('结构索引增量收敛', () => {
  it('连续输入只重扫变化附近，保留全文覆盖与文末引用', async () => {
    const port = createMemoryMarkdownPort();
    const status = await port.openDocument({
      text: '# 标题\n\n' + '正文内容\n\n'.repeat(5000) + '[label]: /tail\n',
    });
    const refs: SnapshotReference[] = [];
    const initial = await port.captureSnapshot(status.documentId);
    refs.push(initial);
    const reads: number[] = [];
    const scan = scannerSpy(reads);
    const options = { signal: new AbortController().signal, onProgress() {}, scan };
    const before = await buildMarkdownIndex(port, initial, options);
    const first = [{ fromByte: 2, toByte: 2, insertedText: '新' }];
    let next = await port.applyTransaction(status.documentId, {
      baseRevision: 0,
      operationId: '1',
      historyGroupId: 'g',
      edits: first,
    });
    const second = [{ fromByte: 5, toByte: 5, insertedText: '的' }];
    next = await port.applyTransaction(status.documentId, {
      baseRevision: next.revision,
      operationId: '2',
      historyGroupId: 'g',
      edits: second,
    });
    const invalidation = invalidateMarkdownIndex(invalidateMarkdownIndex(undefined, first), second);
    const current = await port.captureSnapshot(status.documentId);
    refs.push(current);
    reads.length = 0;
    const after = await buildMarkdownIndex(port, current, {
      ...options,
      previous: { index: before, snapshot: initial },
      invalidation,
    });
    expect(reads.reduce((a, b) => a + b, 0)).toBeLessThan(4096);
    expect(after.progress.state).toBe('ready');
    expect(after.blocks.at(-1)?.toByte).toBe(current.byteLength);
    expect(after.references.LABEL.href).toBe('/tail');
    expect(after.blocks.slice(1).map(block => block.id)).toEqual(before.blocks.slice(1).map(block => block.id));
    expect(after.blocks[1].contentVersion).toBe(before.blocks[1].contentVersion);
    const complete = await buildMarkdownIndex(port, current, options);
    expect(after.outline.map(({ blockId: _id, ...item }) => item)).toEqual(
      complete.outline.map(({ blockId: _id, ...item }) => item),
    );
    expect(
      after.blocks.map((block) => [
        block.fromByte,
        block.toByte,
        block.fromLine,
        block.toLine,
        block.kind,
        block.headings,
      ]),
    ).toEqual(
      complete.blocks.map((block) => [
        block.fromByte,
        block.toByte,
        block.fromLine,
        block.toLine,
        block.kind,
        block.headings,
      ]),
    );
    for (const ref of refs) await port.releaseSnapshot(ref);
    await port.close(status.documentId, true);
  });
  it('编辑引用定义后重建首个有效定义，不沿用旧字典', async () => {
    const port = createMemoryMarkdownPort();
    const status = await port.openDocument({
      text: '[a]: /before\n\n中间\n\n[a]: /duplicate\n\n[a]\n',
    });
    const scan = scannerSpy([]);
    const options = { signal: new AbortController().signal, onProgress() {}, scan };
    const initial = await port.captureSnapshot(status.documentId);
    const before = await buildMarkdownIndex(port, initial, options);
    const edits = [{ fromByte: 5, toByte: 12, insertedText: '/after' }];
    await port.applyTransaction(status.documentId, {
      baseRevision: 0,
      operationId: '1',
      historyGroupId: 'g',
      edits,
    });
    const current = await port.captureSnapshot(status.documentId);
    const after = await buildMarkdownIndex(port, current, {
      ...options,
      previous: { index: before, snapshot: initial },
      edits,
    });
    expect(after.references.A.href).toBe('/after');
    await port.releaseSnapshot(initial);
    await port.releaseSnapshot(current);
    await port.close(status.documentId, true);
  });
  it('跨批次标题 ID 全局去重，部分结果不改写原文中的旧目录', async () => {
    const source =
      '<!-- toc -->\n- [保留原目录](#old)\n<!-- /toc -->\n\n# 重复\n\n' +
      '普通正文\n\n'.repeat(300) +
      '# 重复\n\n# 重复-2\n\n## **格式**😀\n\n# 😀\n';
    const port = createMemoryMarkdownPort();
    const status = await port.openDocument({ text: source });
    const snapshot = await port.captureSnapshot(status.documentId);
    const states: string[] = [];
    const index = await buildMarkdownIndex(port, snapshot, {
      signal: new AbortController().signal,
      scan: scannerSpy([]),
      onProgress(index) {
        states.push(index.progress.state);
      },
    });
    expect(states).toContain('partial');
    expect(index.outline.map(({ byte: _byte, blockId: _id, ...item }) => item)).toEqual(
      extractOutline(source),
    );
    expect(new Set(index.outline.map((item) => item.id)).size).toBe(index.outline.length);
    expect((await port.status(status.documentId)).revision).toBe(0);
    expect((await port.readSnapshot(snapshot, 0, snapshot.byteLength)).text).toBe(source);
    await port.releaseSnapshot(snapshot);
    await port.close(status.documentId, false);
  });
});
