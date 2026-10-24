import { describe, expect, it, vi } from 'vitest';
import { createMemoryMarkdownPort } from './port';
import { ProgressiveMarkdownDocument } from './runtime';
import { NativeStructureClient } from './nativeStructureClient';
import { NativeMarkdownStructureIndex } from './nativeStructureIndex';
import { emptyEnhancements } from './enhancements';
import { STRUCTURE_SYNTAX_VERSION, type StructureQueryPort, type StructureResponse } from './structureProtocol';

describe('原生宿主接入', () => {
  it('渐进几何立即发布首批，后续仅重读末个区域而不扫描已确认前缀', async () => {
    const identity = { documentId: 'geometry-prefix', revision: 0, indexGeneration: 31, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
    let end = 30;
    const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn(async (_, query) => {
      if (query.kind !== 'geometry') throw new Error('unexpected query');
      const items = Array.from({ length: end / 10 }, (_, i) => ({fromByte:i*10,toByte:(i+1)*10,blockCount:1,
        layoutUnits:1,enhancements:emptyEnhancements(),metricsComplete:true})).filter(r=>r.fromByte>=query.fromByte);
      return {...identity,state:end===40?'ready':'building',totalBytes:40,indexedBytes:end,semanticComplete:true,failure:null,
        items,next:end===40?null:{generation:31,ordinal:end,offset:0},fragment:null};
    }) as StructureQueryPort['queryStructure'] };
    const signal = new AbortController().signal;
    const index = new NativeMarkdownStructureIndex(await NativeStructureClient.open(port,identity.documentId,0,signal),40);
    expect(await index.refresh(signal)).toBe(false);
    expect(port.queryStructure).toHaveBeenCalledTimes(1);
    end=40; expect(await index.refresh(signal)).toBe(true);
    expect(port.queryStructure).toHaveBeenLastCalledWith(identity,expect.objectContaining({kind:'geometry',fromByte:20}));
    expect(index.blocks.map(b=>[b.fromByte,b.toByte])).toEqual([[0,10],[10,20],[20,30],[30,40]]);
    index.dispose();NativeStructureClient.releaseDocument(identity.documentId);
  });
  it('密集短标题按可见详情数量限流，不把整段字节范围的标题下载到宿主', async () => {
    const identity = { documentId: 'dense-headings', revision: 0, indexGeneration: 21, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
    const blocks = Array.from({ length: 2000 }, (_, i) => ({ id: `dense:${i}`, version: 0,
      fromByte: i * 12, toByte: (i + 1) * 12, fromLine: i + 1, toLine: i + 2, kind: 'heading_open', mode: 'semantic',
      layoutUnits: 1, enhancements: emptyEnhancements(), enhancementObjects: 0, metricsComplete: true, headings: [], references: [] }));
    const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn(async (_, query) => {
      let items: unknown[] = [], next = null;
      if (query.kind === 'geometry') items = [{ fromByte: 0, toByte: 24000, blockCount: 2000, layoutUnits: 2000, enhancements: emptyEnhancements(), metricsComplete: true }];
      if (query.kind === 'blocks' || query.kind === 'headings') {
        const found = blocks.filter(b => b.toByte > Math.max(query.fromByte, query.cursor?.ordinal ?? 0) && b.fromByte < query.toByte);
        items = found.slice(0, 256);
        if (found.length > 256) next = { generation: 21, ordinal: found[255].toByte, offset: 0 };
        if (query.kind === 'headings') items = (items as typeof blocks).map(b => ({ id: b.id, title: '重复标题', level: 2, byte: b.fromByte, line: b.fromLine, blockId: b.id, blockHeadingIndex: 0 }));
      }
      return { ...identity, state: 'ready', totalBytes: 24000, indexedBytes: 24000, semanticComplete: true, failure: null, items, next, fragment: null };
    }) as StructureQueryPort['queryStructure'] };
    const signal = new AbortController().signal;
    const index = new NativeMarkdownStructureIndex(await NativeStructureClient.open(port, identity.documentId, 0, signal), 24000);
    await index.refresh(signal);
    const owner = {};
    await index.refine(owner, [{ fromByte: 0, toByte: 24000 }], signal);
    expect(index.blocks.filter(b => b.kind !== 'native_region')).toHaveLength(128);
    expect(index.outline).toHaveLength(128);
    const firstProjection = index.blocks;
    const secondOwner = {};
    await index.refine(secondOwner, [{ fromByte: 0, toByte: 24000 }], signal);
    expect(index.blocks).toBe(firstProjection);
    index.release(secondOwner);
    expect(index.blocks).toBe(firstProjection);
    await index.refine(owner, [{ fromByte: 12000, toByte: 24000 }], signal);
    expect(index.blockAt(12000)?.id).toBe('dense:1000');
    expect(index.blocks.some(b => b.id === 'dense:0')).toBe(false);
    index.dispose(); NativeStructureClient.releaseDocument(identity.documentId);
  });
  it('桌面端口建立索引不读取正文快照，关闭清理后不可继续查询', async () => {
    const port = createMemoryMarkdownPort();
    const status = await port.openDocument({ text: '# A' });
    const identity = { documentId: status.documentId, revision: 0, indexGeneration: 1, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
    port.startStructure = vi.fn(async () => identity);
    port.queryStructure = vi.fn(async () => ({ ...identity, state: 'ready', totalBytes: 3,
      indexedBytes: 3, semanticComplete: true, failure: null, items: [], next: null, fragment: null })) as StructureQueryPort['queryStructure'];
    const capture = vi.spyOn(port, 'captureSnapshot');
    const read = vi.spyOn(port, 'readWindow');
    const runtime = new ProgressiveMarkdownDocument(port, status);
    await vi.waitFor(() => expect(runtime.state.progress.state).toBe('ready'));
    expect(runtime.usesNativeStructure).toBe(true);
    expect(runtime.state.mountMode).toBe('windowed');
    expect(capture).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled();
    const client = runtime.nativeStructure!.client;
    await runtime.close();
    await expect(client.query({ kind: 'progress' })).rejects.toThrow();
  });

  it('粗几何只细化请求范围，换区释放旧详情并注入全文有效定义', async () => {
    const identity = { documentId: 'bounded-host', revision: 0, indexGeneration: 1, syntaxVersion: STRUCTURE_SYNTAX_VERSION };
    let complete = false;
    const response = (items: unknown[]): StructureResponse => ({ ...identity, state: complete ? 'ready' : 'building', totalBytes: 100000,
      indexedBytes: complete ? 100000 : 1000, semanticComplete: complete, failure: null, items, next: null, fragment: null });
    const port: StructureQueryPort = { startStructure: async () => identity, queryStructure: vi.fn(async (_, query) => {
      if (query.kind === 'geometry') return response([{ fromByte: 0, toByte: 100000, blockCount: 5000,
        layoutUnits: 5000, enhancements: emptyEnhancements(), metricsComplete: true }]);
      if (query.kind === 'blocks') return response([{ id: `b:${query.fromByte}`, version: 0,
        fromByte: query.fromByte, toByte: query.toByte, fromLine: 1, toLine: 1, kind: 'paragraph_open', mode: 'semantic',
        layoutUnits: 1, enhancements: emptyEnhancements(), enhancementObjects: 0, metricsComplete: true, headings: [], references: ['ID'] }]);
      if (query.kind === 'references') return response(complete ? [{ key: 'ID', definition: { href: '/first-global', title: '' } }] : []);
      return response([]);
    }) as StructureQueryPort['queryStructure'] };
    const signal = new AbortController().signal;
    const client = await NativeStructureClient.open(port, identity.documentId, 0, signal);
    const index = new NativeMarkdownStructureIndex(client, 100000); const owner = {};
    await index.refresh(signal);
    expect(index.blocks).toHaveLength(1); expect(index.blocks[0].kind).toBe('native_region');
    await index.refine(owner, [{ fromByte: 0, toByte: 100 }], signal);
    expect(index.references.ID).toBeUndefined();
    complete = true;
    await index.refresh(signal);
    expect(index.blocks[0].kind).toBe('native_region');
    await index.refine(owner, [{ fromByte: 0, toByte: 100 }], signal);
    expect(index.references.ID.href).toBe('/first-global');
    await index.refine(owner, [{ fromByte: 90000, toByte: 90100 }], signal);
    expect(index.blocks.find(block => block.id === 'b:0')).toBeUndefined();
    expect(index.blocks).toHaveLength(3);
    const nextIdentity = { ...identity, revision: 1, indexGeneration: 2 };
    let invalidated = false;
    const nextPort: StructureQueryPort = { startStructure: async () => nextIdentity,
      queryStructure: vi.fn(async (_, query) => ({ ...response(query.kind === 'changes' ? [{
        fromGeneration: 1, invalidateAll: false, outlineChanged: false, references: [], referenceVersion: 1,
        mappings: [{fromByte:0,toByte:100000,byteDelta:5,lineDelta:1}],
        ranges: invalidated ? [{fromByte:90005,toByte:90105}] : [],
      }] : query.kind === 'geometry' ? [{fromByte:0,toByte:100005,blockCount:5000,layoutUnits:5000,enhancements:emptyEnhancements(),metricsComplete:true}] : []), ...nextIdentity })) as StructureQueryPort['queryStructure'] };
    const next = new NativeMarkdownStructureIndex(await NativeStructureClient.open(nextPort, identity.documentId, 1, signal),100005);
    await next.refresh(signal); await next.adopt(index,signal);
    expect(next.blocks.find(block => block.id === 'b:90000')).toMatchObject({fromByte:90005,toByte:90105,version:1,fromLine:2});
    expect(next.references.ID.href).toBe('/first-global');
    expect(nextPort.queryStructure).not.toHaveBeenCalledWith(expect.anything(), expect.objectContaining({kind:'blocks'}));
    next.release(owner); expect(next.blocks).toHaveLength(1); next.dispose(); index.dispose();
    NativeStructureClient.releaseDocument(identity.documentId);
  });
});
