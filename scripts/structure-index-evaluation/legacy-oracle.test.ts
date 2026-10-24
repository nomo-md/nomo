// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { MarkdownStructureScanner, type MarkdownStructureBlock } from '../../src/lib/markdown-document/structure';
import { MarkdownStructureIndex } from '../../src/lib/markdown-document/structureIndex';

const output = process.env.NOMO_STRUCTURE_EVALUATION_OUTPUT;
describe.skipIf(!output)('原有结构实现迁移对照（显式启用，不进入产品）', () => {
  it('输出真实块边界、标题和引用；检查分批输入的覆盖与一致性', () => {
    const fixtures: Array<{ name: string; markdown: string; family: string }> = JSON.parse(readFileSync(
      resolve('scripts/structure-index-evaluation/nomo-fixtures.json'), 'utf8'));
    const reports: unknown[] = [];
    for (const fixture of fixtures) {
      const runs = [1, 3, 17, 16384].map(size => {
        const scanner = new MarkdownStructureScanner(1);
        const index = new MarkdownStructureIndex(1, new TextEncoder().encode(fixture.markdown).length);
        const chars = [...fixture.markdown];
        for (let from = 0; from < chars.length; from += size) {
          index.publish(scanner.push(chars.slice(from, from + size).join(''), from + size >= chars.length));
        }
        if (!chars.length) index.publish(scanner.push('', true));
        const blocks = index.blocks.map(({ id: _id, version: _version, contentVersion: _contentVersion, ...block }: MarkdownStructureBlock) => block);
        const headings = index.outline.map(({ blockId: _blockId, ...heading }) => heading);
        let cursor = 0;
        for (const block of blocks) { expect(block.fromByte).toBe(cursor); cursor = block.toByte; }
        expect(cursor).toBe(new TextEncoder().encode(fixture.markdown).length);
        expect(index.progress.state).toBe('ready');
        return { size, blocks, headings, references: index.references };
      });
      // 不把旧行为自动当成新规范；差异保留在产物中，由语法契约判定。
      const baseline = JSON.stringify({ ...runs.at(-1), size: 0 });
      reports.push({ ...fixture, legacyChunkInvariant: runs.every(run => JSON.stringify({ ...run, size: 0 }) === baseline), runs });
    }
    mkdirSync(resolve(output!), { recursive: true });
    writeFileSync(resolve(output!, 'legacy-oracle.json'), JSON.stringify(reports, null, 2) + '\n', 'utf8');
  });
});
