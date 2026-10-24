import { describe, expect, it, vi } from 'vitest';
import { NativeStructureClient } from './nativeStructureClient';
import { STRUCTURE_SYNTAX_VERSION, type StructureQueryPort, type StructureResponse } from './structureProtocol';
const identity = { documentId: 'native-test', revision: 2, indexGeneration: 7, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
const response = (overrides: Partial<StructureResponse> = {}): StructureResponse => ({ ...identity, state: 'ready', indexedBytes: 10, totalBytes: 10, semanticComplete: true, failure: null, items: [], next: null, fragment: null, ...overrides });
describe('原生结构查询端口', () => {
  it('拒绝过期结果而不调用 JS 扫描或正文读取', async () => {
    const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn(async () => response({ revision: 1 })) as StructureQueryPort['queryStructure'] };
    const client = await NativeStructureClient.open(port, identity.documentId, 2, new AbortController().signal);
    await expect(client.query({ kind: 'progress' })).rejects.toThrow('生成版本');
    client.dispose();
  });
  it('只续取当前超长单项，保留下一页游标', async () => {
    const block = { id: 'block', version: 2, fromByte: 0, toByte: 10 };
    const serialized = JSON.stringify(block);
    const queryStructure = vi.fn().mockResolvedValueOnce(response({ fragment: serialized.slice(0, 15), next: { generation: 7, ordinal: 0, offset: 15 } }))
      .mockResolvedValueOnce(response({ fragment: serialized.slice(15), next: { generation: 7, ordinal: 1, offset: 0 } }));
    const client = await NativeStructureClient.open({ startStructure: async () => identity, queryStructure }, identity.documentId, 2, new AbortController().signal);
    const page = await client.blocks(0, 10);
    expect(page.items).toEqual([block]); expect(page.next?.ordinal).toBe(1); expect(queryStructure).toHaveBeenCalledTimes(2);
    client.dispose();
  });
  it('共享缓存遵守几何及总字节预算，关闭文档释放纯数据', async () => {
    const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn(async () => response({ items: [{ text: '中'.repeat(30000) }] })) as StructureQueryPort['queryStructure'] };
    const client = await NativeStructureClient.open(port, identity.documentId, 2, new AbortController().signal);
    for (let i = 0; i < 30; i++) await client.query({ kind: 'geometry', fromByte: i, toByte: i + 1, groupSize: 256 });
    const usage = NativeStructureClient.cacheUsage();
    expect(usage.geometryBytes).toBeLessThanOrEqual(usage.geometryCapacityBytes);
    expect(usage.usedBytes).toBeLessThanOrEqual(usage.capacityBytes);
    const lease = Symbol('visible-details');
    client.retain(lease, 'x'.repeat(3 * 1024 * 1024));
    expect(NativeStructureClient.cacheUsage().usedBytes).toBeLessThanOrEqual(usage.capacityBytes);
    expect(() => client.retain(Symbol('another-view'), 'x'.repeat(2 * 1024 * 1024))).toThrow('共享预算');
    client.releaseRetained(lease);
    NativeStructureClient.releaseDocument(identity.documentId); expect(NativeStructureClient.cacheUsage().usedBytes).toBe(0); client.dispose();
  });
});
