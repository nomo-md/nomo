import { describe, expect, it, vi } from 'vitest';
import { NativeStructureClient } from './nativeStructureClient';
import { NativeWindowStructure } from './nativeWindowStructure';
import { STRUCTURE_SYNTAX_VERSION, type StructureQueryPort, type StructureResponse, type StructureQuery } from './structureProtocol';
const identity = { documentId: 'window-test', revision: 0, indexGeneration: 1, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
const response = (items: unknown[], state: 'ready' | 'building' = 'ready'): StructureResponse => ({ ...identity, state, indexedBytes: 100, totalBytes: 100, semanticComplete: state === 'ready', failure: null, items, next: null, fragment: null });
const block = { id: 'stable', version: 0, fromByte: 10, toByte: 30, mode: 'semantic', headings: [], references: ['ID'] };
async function fixture(query: (q: StructureQuery) => Promise<StructureResponse>) {
  NativeStructureClient.releaseDocument(identity.documentId);
  const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn((_, q) => query(q)) as StructureQueryPort['queryStructure'] };
  const client = await NativeStructureClient.open(port, identity.documentId, 0, new AbortController().signal);
  return { client, facts: new NativeWindowStructure(client), port };
}
describe('原生正文局部结构消费', () => {
  it('只查询覆盖块、该范围标题及所需引用；不需要正文扫描接口', async () => {
    const { client, facts, port } = await fixture(async q => response(q.kind === 'blocks' ? [block] : q.kind === 'references' ? [{ key: 'ID', definition: { href: '/global-first', title: '' } }] : []));
    const result = await facts.prepare(12, new AbortController().signal);
    expect(result.state).toBe('ready');
    if (result.state === 'ready') expect(result.references.ID.href).toBe('/global-first');
    expect(vi.mocked(port.queryStructure).mock.calls.map(([, q]) => q)).toEqual([
      { kind: 'blocks', fromByte: 12, toByte: 13, cursor: undefined },
      { kind: 'headings', fromByte: 10, toByte: 30, cursor: undefined },
      { kind: 'references', keys: ['ID'] },
    ]); client.dispose();
  });
  it('文末引用未完成时保持准备状态，巨型语法块保持原生限制', async () => {
    const first = await fixture(async () => response([block], 'building'));
    expect((await first.facts.prepare(12, new AbortController().signal)).state).toBe('preparing');
    expect(first.port.queryStructure).toHaveBeenCalledTimes(1); first.client.dispose();
    const second = await fixture(async () => response([{ ...block, mode: 'raw', toByte: 100000 }]));
    expect((await second.facts.prepare(12, new AbortController().signal)).state).toBe('restricted'); second.client.dispose();
  });
  it('旧大纲条目只传稳定身份给当前修订，不能拿旧 slug 或字节定位', async () => {
    const { client, facts, port } = await fixture(async () => response([{ status: 'found', target: { fromByte: 80, toByte: 100, slug: 'renamed' } }]));
    expect((await facts.resolveHeading({ blockId: 'stable', blockHeadingIndex: 0 }, new AbortController().signal)).status).toBe('found');
    expect(port.queryStructure).toHaveBeenCalledWith(identity, { kind: 'resolve', namespace: 'heading', key: 'stable:0' }); client.dispose();
  });
});
