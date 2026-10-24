import { NativeStructureClient } from './nativeStructureClient';
import type { MarkdownReference } from './structure';
import type { StructureBlock, StructureCursor, StructureHeading, StructureReference, StructureTarget } from './structureProtocol';

const SEMANTIC_BYTES = 32 * 1024;
const DETAIL_BYTES = 2 * 1024 * 1024;
export type NativeWindowFacts =
  | { state: 'preparing'; fromByte: number; toByte: number }
  | { state: 'restricted'; fromByte: number; toByte: number; blockId: string }
  | { state: 'ready'; fromByte: number; toByte: number; blocks: StructureBlock[];
      headings: StructureHeading[]; references: Record<string, MarkdownReference> };

/** 正文挂载前取得同代次局部事实；不读取正文，不扫描完整标题或引用表。 */
export class NativeWindowStructure {
  constructor(readonly client: NativeStructureClient) {}
  async prepare(byte: number, signal: AbortSignal): Promise<NativeWindowFacts> {
    signal.throwIfAborted();
    const page = await this.client.blocks(byte, byte + 1, undefined, signal);
    const block = page.items.find(item => item.fromByte <= byte && byte < item.toByte);
    if (!block) return { state: 'preparing', fromByte: byte, toByte: byte };
    if (block.mode === 'raw' || block.toByte - block.fromByte > SEMANTIC_BYTES)
      return { state: 'restricted', fromByte: block.fromByte, toByte: block.toByte, blockId: block.id };
    if (block.semanticComplete === false) throw new Error('当前块语义分析不完整，保留原文并报告失败。');
    // 文末定义尚未扫描时，不能把局部解析的“未定义”发布成正式链接结果。
    if (page.state !== 'ready') return { state: 'preparing', fromByte: block.fromByte, toByte: block.toByte };
    let bytes = JSON.stringify(block).length * 2;
    const account = (value: unknown) => {
      bytes += JSON.stringify(value).length * 2;
      if (bytes > DETAIL_BYTES) throw new Error('当前块的结构详情超过挂载预算，保留原文并报告失败。');
    };
    const headings: StructureHeading[] = [];
    let cursor: StructureCursor | undefined;
    do {
      const page = await this.client.query<StructureHeading>({ kind: 'headings', fromByte: block.fromByte, toByte: block.toByte, cursor }, signal);
      account(page.items); headings.push(...page.items);
      if (page.next && cursor && page.next.ordinal <= cursor.ordinal) throw new Error('标题详情分页没有前进。');
      cursor = page.next ?? undefined;
    } while (cursor);
    const references: Record<string, MarkdownReference> = Object.create(null);
    const keys = [...new Set(block.references)];
    const readReferences = async (batch: string[]): Promise<void> => {
      try {
        const response = await this.client.query<StructureReference>({ kind: 'references', keys: batch }, signal);
        account(response.items);
        for (const { key, definition } of response.items) references[key] = definition;
      } catch (error) {
        // 按实际编码字节分片；一项仍超预算时保留明确错误，不省略有效定义。
        if ((error as { code?: string })?.code !== 'structure-page-limit' || batch.length < 2) throw error;
        const middle = Math.ceil(batch.length / 2);
        await readReferences(batch.slice(0, middle)); await readReferences(batch.slice(middle));
      }
    };
    for (let start = 0; start < keys.length; start += 256) await readReferences(keys.slice(start, start + 256));
    signal.throwIfAborted();
    return { state: 'ready', fromByte: block.fromByte, toByte: block.toByte, blocks: [block], headings, references };
  }
  /** 调用方必须以当前提交修订创建 client；旧行仅提供稳定身份。 */
  async resolveHeading(item: Pick<StructureHeading, 'blockId' | 'blockHeadingIndex'>, signal: AbortSignal): Promise<StructureTarget> {
    return this.resolve('heading', `${item.blockId}:${item.blockHeadingIndex}`, signal);
  }
  async resolve(namespace: 'heading' | 'slug' | 'link' | 'footnote', key: string, signal: AbortSignal): Promise<StructureTarget> {
    const response = await this.client.query<StructureTarget>({ kind: 'resolve', namespace, key }, signal);
    const result = response.items[0];
    if (!result) throw new Error('原生目标查询缺少状态。');
    return result;
  }
}
