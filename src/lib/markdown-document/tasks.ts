import { isWholeWordRange } from '../search/textSearch';
import type { MarkdownDocumentPort, SnapshotReference } from './port';
import type { MarkdownSessionController } from './controller';
import { DocumentConflictError, type SourceEdit } from './protocol';
import { utf8Length, utf8ToUtf16, utf8OffsetMap } from './utf8Text';

export interface MarkdownSearchMatch {
  fromByte: number;
  toByte: number;
  text: string;
  /** CRLF 是事务边界；只匹配其中一部分时，补丁必须保留另一部分。 */
  preservedPrefix: string;
  preservedSuffix: string;
}
export interface MarkdownTaskProgress {
  state: 'partial' | 'ready';
  scannedBytes: number;
  totalBytes: number;
  matches: number;
}
const escaped = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** 全文文字搜索与窗口无关，跨传输边界保留完整查询和相邻 Unicode 字符。 */
export async function searchMarkdownSnapshot(
  port: MarkdownDocumentPort,
  snapshot: SnapshotReference,
  query: string,
  options: {
    caseSensitive?: boolean;
    wholeWord?: boolean;
    signal: AbortSignal;
    onProgress?(progress: MarkdownTaskProgress): void;
    onMatches?(matches: readonly MarkdownSearchMatch[]): void;
    retainMatches?: boolean;
    onMatch?(match: MarkdownSearchMatch, index: number): boolean | void;
  },
): Promise<MarkdownSearchMatch[]> {
  const result: MarkdownSearchMatch[] = [];
  let count = 0;
  if (!query) {
    options.onProgress?.({
      state: 'ready',
      scannedBytes: snapshot.byteLength,
      totalBytes: snapshot.byteLength,
      matches: 0,
    });
    return result;
  }
  const expression = new RegExp(escaped(query), options.caseSensitive ? 'gu' : 'giu');
  let carry = '';
  let from = 0;
  let nextSearchByte = 0;
  while (from < snapshot.byteLength) {
    options.signal.throwIfAborted();
    const window = await port.readSnapshot(
      snapshot,
      from,
      Math.min(snapshot.byteLength, from + 64 * 1024),
    );
    if (
      window.documentId !== snapshot.documentId ||
      window.fromByte !== from ||
      window.toByte <= from ||
      window.revision !== snapshot.revision
    )
      throw new Error('全文搜索读取到不连续的快照。');
    const startByte = from - utf8Length(carry);
    const buffer = carry + window.text;
    const byteAt = utf8OffsetMap(buffer);
    const complete = window.toByte === snapshot.byteLength;
    let safeEnd = complete ? buffer.length : Math.max(0, buffer.length - query.length - 2);
    if (safeEnd > 0 && /[\uDC00-\uDFFF]/.test(buffer[safeEnd] ?? '')) safeEnd--;
    expression.lastIndex = utf8ToUtf16(buffer, nextSearchByte - startByte);
    let consumed = expression.lastIndex;
    const matches: MarkdownSearchMatch[] = [];
    let started = performance.now();
    let checked = 0;
    for (
      let match = expression.exec(buffer);
      match && match.index < safeEnd;
      match = expression.exec(buffer)
    ) {
      if (++checked % 256 === 0 && performance.now() - started >= 4) {
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        options.signal.throwIfAborted();
        started = performance.now();
      }
      const from = match.index;
      const to = from + match[0].length;
      consumed = expression.lastIndex;
      if (options.wholeWord && !isWholeWordRange(buffer, from, to)) continue;
      const prefix = buffer[from] === '\n' && buffer[from - 1] === '\r' ? '\r' : '';
      const suffix = buffer[to - 1] === '\r' && buffer[to] === '\n' ? '\n' : '';
      const item = {
        fromByte: startByte + byteAt(from - prefix.length),
        toByte: startByte + byteAt(to + suffix.length),
        text: match[0],
        preservedPrefix: prefix,
        preservedSuffix: suffix,
      };
      if (options.retainMatches !== false || options.onMatches) matches.push(item);
      if (options.onMatch?.(item, count) === false) return result;
      count++;
    }
    if (options.retainMatches !== false) for (const match of matches) result.push(match);
    options.onMatches?.(matches);
    consumed = Math.max(consumed, safeEnd);
    nextSearchByte = startByte + byteAt(consumed);
    let keepFrom = Math.max(0, consumed - 2);
    if (keepFrom && /[\uDC00-\uDFFF]/.test(buffer[keepFrom] ?? '')) keepFrom--;
    carry = buffer.slice(keepFrom);
    from = window.toByte;
    options.onProgress?.({
      state: complete ? 'ready' : 'partial',
      scannedBytes: from,
      totalBytes: snapshot.byteLength,
      matches: count,
    });
    // 每个主线程批次仅搜索 64 KiB，输入和取消可在批次之间抢占。
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
  options.signal.throwIfAborted();
  return result;
}

/** 全文替换先计算完整补丁，再校验快照与视图世代。用户输入不会被旧任务覆盖。 */
export async function replaceMarkdownSnapshotMatches(
  session: MarkdownSessionController,
  snapshot: SnapshotReference,
  matches: readonly MarkdownSearchMatch[],
  replacement: string,
  viewGeneration: number,
) {
  if (
    session.state.viewGeneration !== viewGeneration ||
    session.state.pendingTransactions ||
    session.state.committed.revision !== snapshot.revision
  )
    throw new DocumentConflictError();
  const edits: SourceEdit[] = [];
  let suffix = '';
  for (const match of matches) {
    const previous = edits.at(-1);
    if (previous && match.fromByte < previous.toByte) {
      // 相邻命中可落在同一 CRLF 的中点。合并扩大的事务范围，
      // 只保留最外侧换行字节，不能重复写回内部 CR/LF。
      if (
        suffix !== '\n' ||
        match.preservedPrefix !== '\r' ||
        match.fromByte + 2 !== previous.toByte
      )
        throw new Error('搜索结果范围重叠，未修改正文。');
      previous.toByte = match.toByte;
      previous.insertedText =
        previous.insertedText.slice(0, -suffix.length) + replacement + match.preservedSuffix;
    } else
      edits.push({
        fromByte: match.fromByte,
        toByte: match.toByte,
        insertedText: match.preservedPrefix + replacement + match.preservedSuffix,
      });
    suffix = match.preservedSuffix;
  }
  return session.applyEdits({
    viewGeneration,
    historyGroupId: globalThis.crypto.randomUUID(),
    edits,
  });
}
