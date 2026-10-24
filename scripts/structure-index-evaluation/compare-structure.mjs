import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const output = resolve(process.argv[2] ?? '.artifacts/structure-index-20261024');
const native = JSON.parse(await readFile(join(output, 'kernel-report.json'), 'utf8'));
const legacy = JSON.parse(await readFile(join(output, 'legacy-oracle.json'), 'utf8'));
const runs = native.structureAdapters.fixtures.map(run => {
  const oracle = legacy.find(item => item.name === run.name);
  if (!oracle) throw new Error(`Missing legacy oracle: ${run.name}`);
  const expected = oracle.runs.at(-1);
  const simplifyBlocks = blocks => blocks.map(({ fromByte, toByte, kind }) => ({ fromByte, toByte, kind }));
  const simplifyHeadings = headings => headings.map(({ level, text, title, byte, line }) => ({ level, text: title ?? text, byte, line }));
  const simplifyReferences = refs => Object.fromEntries(Object.entries(refs).sort(([a], [b]) => a.localeCompare(b))
    .map(([key, { href, title }]) => [key, { href, title }]));
  const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const actual = run.facts;
  const checks = {
    blockRangesAndKinds: equal(simplifyBlocks(actual.blocks), simplifyBlocks(expected.blocks)),
    headingsAndSourceLocations: equal(simplifyHeadings(actual.headings), simplifyHeadings(expected.headings)),
    effectiveLinkDefinitions: equal(simplifyReferences(actual.references), simplifyReferences(expected.references)),
    chunkInvariant: run.chunkRuns.every(chunk => chunk.matchesFull),
  };
  return { name: run.name, family: run.family, engine: run.engine, checks,
    allComparedFactsEqual: Object.values(checks).every(Boolean),
    expected: { blocks: simplifyBlocks(expected.blocks), headings: simplifyHeadings(expected.headings), references: simplifyReferences(expected.references) },
    actual,
  };
});
const summaries = ['pulldown-cmark', 'markdown-it'].map(engine => {
  const selected = runs.filter(run => run.engine === engine);
  return { engine, total: selected.length, allComparedFactsEqual: selected.filter(run => run.allComparedFactsEqual).length,
    differences: selected.filter(run => !run.allComparedFactsEqual).map(({ name, family, checks }) => ({ name, family, checks })) };
});
await writeFile(join(output, 'structure-comparison.json'), JSON.stringify({ schemaVersion: 1,
  warning: 'Prototype adapters are not complete Nomo implementations. Differences are compatibility work, not proof a library is unsuitable. This comparison does not validate slug allocation, enhancements, invalidation or lifecycle.',
  summaries, runs }, null, 2) + '\n');
for (const summary of summaries) console.log(`${summary.engine}: ${summary.allComparedFactsEqual}/${summary.total} compared structure facts equal to legacy`);
